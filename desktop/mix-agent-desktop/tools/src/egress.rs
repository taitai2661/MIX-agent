//! Egress allowlist + private-IP deny for outbound Tool calls.
//!
//! Tools should call [`EgressPolicy::enforce_async`] before opening any
//! outbound HTTP connection. The allowlist is a vector of exact hostnames or
//! `*.example.com` patterns; when the list is empty the gate is wide-open
//! (matches the Docker edition's behaviour of "no allowlist set ⇒ allow public
//! domains, deny private IPs").

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use anyhow::{anyhow, bail};
use ipnetwork::IpNetwork;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct EgressPolicy {
    pub allowlist: Vec<String>,
}

impl EgressPolicy {
    pub fn from_allowlist(allowlist: Vec<String>) -> Self {
        Self { allowlist }
    }

    /// Allowlist and literal-address checks only — no DNS.
    ///
    /// Use [`enforce_async`] before actually connecting: a hostname is not
    /// evidence that the address behind it is public, and `localtest.me` and
    /// friends resolve to loopback on purpose.
    pub fn enforce(&self, url: &str) -> anyhow::Result<()> {
        let parsed = url::Url::parse(url).map_err(|e| anyhow!("invalid url: {}", e))?;
        let host = parsed
            .host_str()
            .ok_or_else(|| anyhow!("url is missing a host: {}", url))?;
        if self.allowlist.is_empty() {
            ensure_public_host(host)?;
            return Ok(());
        }
        if !self
            .allowlist
            .iter()
            .any(|pattern| host_matches(pattern, host))
        {
            bail!("host {} not in egress allowlist", host);
        }
        ensure_public_host(host)?;
        Ok(())
    }

    /// [`enforce`] plus a check on what the hostname actually resolves to.
    ///
    /// Called for every hop of a redirect chain by the network Tool, because a
    /// permitted host is free to answer with `302 → http://169.254.169.254/`.
    pub async fn enforce_async(&self, url: &str) -> anyhow::Result<()> {
        self.enforce(url)?;
        let parsed = url::Url::parse(url).map_err(|e| anyhow!("invalid url: {}", e))?;
        let host = bare_host(
            parsed
                .host_str()
                .ok_or_else(|| anyhow!("url is missing a host: {}", url))?,
        );
        if host.parse::<IpAddr>().is_ok() {
            // Already vetted by `ensure_public_host`.
            return Ok(());
        }
        let port = parsed.port_or_known_default().unwrap_or(443);
        let addrs = tokio::net::lookup_host((host, port))
            .await
            .map_err(|e| anyhow!("cannot resolve {}: {}", host, e))?;
        for addr in addrs {
            if is_private(&addr.ip()) {
                bail!(
                    "{} resolves to private address {}, refusing to connect",
                    host,
                    addr.ip()
                );
            }
        }
        Ok(())
    }
}

fn ensure_public_host(host: &str) -> anyhow::Result<()> {
    // `Url::host_str()` yields IPv6 literals with their brackets intact, and
    // `IpAddr::from_str` rejects those — so `[::1]` slipped past the check.
    let host = bare_host(host);
    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_private(&ip) {
            bail!("refusing to connect to private IP {}", ip);
        }
        return Ok(());
    }
    // Hostname — try a literal-IP check on the literal form.
    if let Ok(net) = host.parse::<IpNetwork>() {
        if is_private(&net.network()) {
            bail!("refusing to connect to private network {}", net);
        }
    }
    Ok(())
}

/// Drop the brackets IPv6 URLs carry around their literal address.
fn bare_host(host: &str) -> &str {
    host.strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host)
}

fn is_private(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_private_v4(*v4),
        IpAddr::V6(v6) => is_private_v6(*v6),
    }
}

fn in_cidr(ip: u32, net: u32, prefix: u8) -> bool {
    debug_assert!(prefix > 0 && prefix <= 32);
    let mask = u32::MAX << (32 - prefix);
    (ip & mask) == (net & mask)
}

/// Address ranges a Tool must never reach: loopback, RFC1918, link-local
/// (the cloud metadata service lives at `169.254.169.254`), CGNAT,
/// documentation/benchmark ranges, and multicast/reserved space.
fn is_private_v4(ip: Ipv4Addr) -> bool {
    const BLOCKS: &[(u32, u8)] = &[
        (0x0000_0000, 8),  // 0.0.0.0/8        "this network"
        (0x0a00_0000, 8),  // 10.0.0.0/8       RFC1918
        (0x6440_0000, 10), // 100.64.0.0/10    CGNAT
        (0x7f00_0000, 8),  // 127.0.0.0/8      loopback
        (0xa9fe_0000, 16), // 169.254.0.0/16   link-local / metadata
        (0xac10_0000, 12), // 172.16.0.0/12    RFC1918
        (0xc000_0000, 24), // 192.0.0.0/24     IETF protocol assignments
        (0xc000_0200, 24), // 192.0.2.0/24     TEST-NET-1
        (0xc0a8_0000, 16), // 192.168.0.0/16   RFC1918
        (0xc612_0000, 15), // 198.18.0.0/15    benchmarking
        (0xc633_6400, 24), // 198.51.100.0/24  TEST-NET-2
        (0xcb00_7100, 24), // 203.0.113.0/24   TEST-NET-3
        (0xe000_0000, 4),  // 224.0.0.0/4      multicast
        (0xf000_0000, 4),  // 240.0.0.0/4      reserved (+ broadcast)
    ];
    let raw = u32::from(ip);
    BLOCKS
        .iter()
        .any(|(net, prefix)| in_cidr(raw, *net, *prefix))
}

fn is_private_v6(ip: Ipv6Addr) -> bool {
    // IPv4 addresses tunnelled into IPv6 (`::ffff:127.0.0.1`) are the classic
    // way around a naive IPv6-only check.
    if let Some(v4) = ip.to_ipv4() {
        return is_private_v4(v4);
    }
    if ip.is_loopback() || ip.is_unspecified() {
        return true;
    }
    let head = ip.segments()[0];
    (head & 0xffc0) == 0xfe80 // fe80::/10 link-local
        || (head & 0xfe00) == 0xfc00 // fc00::/7 unique-local
        || (head & 0xff00) == 0xff00 // ff00::/8 multicast
        || head == 0x0100 // 100::/64 discard-only
}

fn host_matches(pattern: &str, host: &str) -> bool {
    if let Some(suffix) = pattern.strip_prefix("*.") {
        return host == suffix || host.ends_with(&format!(".{}", suffix));
    }
    pattern == host
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_match_exact() {
        let policy = EgressPolicy::from_allowlist(vec!["api.example.com".to_string()]);
        assert!(policy.enforce("https://api.example.com/v1/chat").is_ok());
        assert!(policy.enforce("https://other.example.com/v1").is_err());
    }

    #[test]
    fn allowlist_wildcard() {
        let policy = EgressPolicy::from_allowlist(vec!["*.openai.com".to_string()]);
        assert!(policy.enforce("https://api.openai.com/v1").is_ok());
        assert!(policy.enforce("https://openai.com/v1").is_ok());
        assert!(policy.enforce("https://example.com/v1").is_err());
    }

    #[test]
    fn empty_allowlist_blocks_private_ips() {
        let policy = EgressPolicy::default();
        assert!(policy.enforce("https://192.168.1.10").is_err());
        assert!(policy.enforce("https://example.com").is_ok());
    }

    #[test]
    fn metadata_and_tunnelled_addresses_are_private() {
        for url in [
            "http://169.254.169.254/latest/meta-data/",
            "http://127.0.0.1:8080/",
            "http://[::1]/",
            "http://[::ffff:127.0.0.1]/",
            "http://[fe80::1]/",
            "http://[fc00::1]/",
            "http://100.64.0.1/",
            "http://0.0.0.0/",
        ] {
            assert!(
                EgressPolicy::default().enforce(url).is_err(),
                "{url} must be blocked"
            );
        }
    }

    #[test]
    fn public_addresses_are_allowed() {
        for url in [
            "https://example.com/",
            "https://93.184.216.34/",
            "https://[2606:2800:220:1:248:1893:25c8:1946]/",
        ] {
            assert!(
                EgressPolicy::default().enforce(url).is_ok(),
                "{url} must be allowed"
            );
        }
    }
}
