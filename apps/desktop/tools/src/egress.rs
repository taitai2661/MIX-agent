//! Egress allowlist + private-IP deny for outbound Tool calls.
//!
//! Tools should call [`EgressPolicy::resolve_async`] before opening any
//! outbound HTTP connection. The allowlist is a vector of exact hostnames or
//! `*.example.com` patterns; when the list is empty the gate is wide-open
//! (matches the Docker edition's behaviour of "no allowlist set ⇒ allow public
//! domains, deny private IPs").
//!
//! Resolution now returns the validated [`SocketAddr`]s alongside the URL so
//! callers can pin the actual connection to those addresses. Without the pin
//! `reqwest` would happily re-resolve the hostname and could land on a
//! different — possibly private — IP via DNS rebinding.
//!
//! The browser's explicit local-network opt-in is handled by the `_with`
//! variants: a loopback name, a literal private IP, or a hostname that resolves
//! only to private addresses is allowed without an allowlist entry, while
//! public destinations still have to match the list.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use anyhow::{anyhow, bail};
use mixd_browser::{is_forbidden_ip, is_private_ip};

/// How long a hostname resolution may take before the call is failed.
const DNS_TIMEOUT: Duration = Duration::from_secs(10);
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
    /// Use [`resolve_async`] before actually connecting: a hostname is not
    /// evidence that the address behind it is public, and `localtest.me` and
    /// friends resolve to loopback on purpose.
    pub fn enforce(&self, url: &str) -> anyhow::Result<()> {
        self.enforce_with(url, false)
    }

    /// [`enforce`] with an explicit local-network opt-in.
    ///
    /// `allow_private` is the browser tool's workspace setting, not a general
    /// egress relaxation: it lets the agent reach a local dev server while
    /// keeping link-local/metadata and multicast addresses blocked.
    pub fn enforce_with(&self, url: &str, allow_private: bool) -> anyhow::Result<()> {
        let parsed = url::Url::parse(url).map_err(|e| anyhow!("invalid url: {}", e))?;
        let host = parsed
            .host_str()
            .ok_or_else(|| anyhow!("url is missing a host: {}", url))?;
        ensure_public_host_with(host, allow_private)?;
        // The local opt-in is a private-network allowance, not an allowlist
        // bypass for public hosts. A loopback name or a literal private IP is
        // reachable without listing it; everything else still has to match.
        let bare = bare_host(host);
        let local_literal = allow_private
            && (is_loopback_name(bare)
                || bare
                    .parse::<IpAddr>()
                    .map(|ip| is_private_ip(&ip))
                    .unwrap_or(false));
        if !local_literal {
            self.check_allowlist(host)?;
        }
        Ok(())
    }

    fn check_allowlist(&self, host: &str) -> anyhow::Result<()> {
        if !self.allowlist.is_empty()
            && !self
                .allowlist
                .iter()
                .any(|pattern| host_matches(pattern, host))
        {
            bail!("host {} not in egress allowlist", host);
        }
        Ok(())
    }

    /// [`enforce`] plus a check on what the hostname actually resolves to, and
    /// the validated addresses themselves.
    ///
    /// Called for every hop of a redirect chain by the network Tool, because a
    /// permitted host is free to answer with `302 → http://169.254.169.254/`.
    /// The returned `Vec<SocketAddr>` is what callers must pin reqwest to:
    /// handing them back in `reqwest::ClientBuilder::resolve` keeps the
    /// connection on the address we vetted.
    pub async fn resolve_async(&self, url: &str) -> anyhow::Result<ResolvedTarget> {
        self.resolve_async_with(url, false).await
    }

    pub async fn resolve_async_with(
        &self,
        url: &str,
        allow_private: bool,
    ) -> anyhow::Result<ResolvedTarget> {
        let parsed = url::Url::parse(url).map_err(|e| anyhow!("invalid url: {}", e))?;
        let host = parsed
            .host_str()
            .ok_or_else(|| anyhow!("url is missing a host: {}", url))?;
        let bare = bare_host(host);
        let port = parsed.port_or_known_default().unwrap_or(443);

        // Literal IP in the URL: no DNS, no rebinding surface, but still
        // honour the policy.
        if let Ok(ip) = bare.parse::<IpAddr>() {
            if is_forbidden_ip(&ip, allow_private) {
                bail!("refusing to connect to private IP {}", ip);
            }
            // Loopback / RFC1918 literals are the whole point of the local
            // opt-in, so they do not also need an allowlist entry.
            if !(allow_private && is_private_ip(&ip)) {
                self.check_allowlist(host)?;
            }
            return Ok(ResolvedTarget {
                url: url.to_string(),
                host: bare.to_string(),
                addrs: vec![SocketAddr::from((ip, port))],
            });
        }

        // Hostname: resolve, vet every address, then hand the surviving list
        // back so the caller can pin reqwest to them. A resolver that never
        // answers would otherwise block the tool call (and its cancellation)
        // before reqwest's own timeout can apply.
        let addrs: Vec<SocketAddr> =
            tokio::time::timeout(DNS_TIMEOUT, tokio::net::lookup_host((bare, port)))
                .await
                .map_err(|_| anyhow!("cannot resolve {}: DNS lookup timed out", bare))?
                .map_err(|e| anyhow!("cannot resolve {}: {}", bare, e))?
                .collect();
        if addrs.is_empty() {
            bail!("{} did not resolve to any address", bare);
        }
        // A hostname that resolves *only* to private addresses is a local dev
        // server under the opt-in, so it is reachable without an allowlist
        // entry. A mixed answer still has to match: otherwise an unlisted
        // public host could sneak through by also publishing one private A
        // record.
        let all_private = addrs.iter().all(|addr| is_private_ip(&addr.ip()));
        if !(allow_private && all_private) {
            self.check_allowlist(host)?;
        }
        let mut vetted = Vec::with_capacity(addrs.len());
        for addr in addrs {
            if is_forbidden_ip(&addr.ip(), allow_private) {
                bail!(
                    "{} resolves to private address {}, refusing to connect",
                    bare,
                    addr.ip()
                );
            }
            vetted.push(addr);
        }
        Ok(ResolvedTarget {
            url: url.to_string(),
            host: bare.to_string(),
            addrs: vetted,
        })
    }

    /// Backwards-compatible wrapper kept for callers that only need the
    /// "deny/allow" verdict.
    pub async fn enforce_async(&self, url: &str) -> anyhow::Result<()> {
        self.resolve_async(url).await.map(|_| ())
    }
}

/// The result of [`EgressPolicy::resolve_async`]: the original URL plus the
/// `SocketAddr`s the caller must use to connect to it.
#[derive(Debug, Clone)]
pub struct ResolvedTarget {
    pub url: String,
    pub host: String,
    pub addrs: Vec<SocketAddr>,
}

fn ensure_public_host_with(host: &str, allow_private: bool) -> anyhow::Result<()> {
    // `Url::host_str()` yields IPv6 literals with their brackets intact, and
    // `IpAddr::from_str` rejects those — so `[::1]` slipped past the check.
    let host = bare_host(host);
    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_forbidden_ip(&ip, allow_private) {
            bail!("refusing to connect to private IP {}", ip);
        }
        return Ok(());
    }
    // Hostname — try a literal-IP check on the literal form.
    if let Ok(net) = host.parse::<IpNetwork>() {
        if is_forbidden_ip(&net.network(), allow_private) {
            bail!("refusing to connect to private network {}", net);
        }
    }
    Ok(())
}

fn is_loopback_name(host: &str) -> bool {
    let host = host.trim_end_matches('.');
    host.eq_ignore_ascii_case("localhost") || host.to_ascii_lowercase().ends_with(".localhost")
}

/// Drop the brackets IPv6 URLs carry around their literal address.
fn bare_host(host: &str) -> &str {
    host.strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host)
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
    fn local_opt_in_allows_dev_servers_but_not_metadata() {
        let policy = EgressPolicy::default();
        for url in [
            "http://127.0.0.1:5173/",
            "http://localhost:3000/",
            "http://192.168.1.20/",
            "http://[::1]:5173/",
            "http://[::ffff:127.0.0.1]:5173/",
        ] {
            assert!(
                policy.enforce_with(url, true).is_ok(),
                "{url} must be reachable with the local opt-in"
            );
        }
        for url in [
            "http://169.254.169.254/latest/meta-data/",
            "http://[::ffff:169.254.169.254]/",
            "http://[fe80::1]/",
            "http://224.0.0.1/",
        ] {
            assert!(
                policy.enforce_with(url, true).is_err(),
                "{url} must stay blocked even with the local opt-in"
            );
        }
    }

    #[test]
    fn local_opt_in_does_not_bypass_the_allowlist_for_public_hosts() {
        // The opt-in is for private destinations. A configured allowlist must
        // still gate public hosts; otherwise turning on "local access" would
        // silently disable the egress allowlist.
        let policy = EgressPolicy::from_allowlist(vec!["api.example.com".to_string()]);
        for url in [
            "http://127.0.0.1:5173/",
            "http://localhost:5173/",
            "http://192.168.1.20/",
        ] {
            assert!(
                policy.enforce_with(url, true).is_ok(),
                "{url} is local and must not need an allowlist entry"
            );
        }
        assert!(
            policy.enforce_with("https://example.com/", true).is_err(),
            "a public host still needs an allowlist entry"
        );
        assert!(
            policy
                .enforce_with("https://api.example.com/", true)
                .is_ok(),
            "an allowlisted public host stays reachable"
        );
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

    #[tokio::test]
    async fn resolve_async_returns_pinned_socket_addrs_for_a_literal_ip() {
        // No DNS needed for a literal IP; the resolved addresses are exactly
        // the one we asked for, so the network tool can hand them to reqwest
        // and reqwest will never go back to the resolver.
        let target = EgressPolicy::default()
            .resolve_async("https://93.184.216.34/")
            .await
            .expect("resolve");
        assert_eq!(target.host, "93.184.216.34");
        assert_eq!(target.addrs.len(), 1);
        assert_eq!(target.addrs[0].ip().to_string(), "93.184.216.34");
    }

    #[tokio::test]
    async fn resolve_async_rejects_a_hostname_that_resolves_to_a_private_ip() {
        // `localtest.me` is a public hostname that resolves to `127.0.0.1`.
        // Without a DNS check the URL would pass the allowlist, but it would
        // land on the loopback interface and reach the user's localhost
        // services.
        let err = EgressPolicy::default()
            .resolve_async("http://localtest.me/")
            .await
            .expect_err("must refuse");
        assert!(
            err.to_string().contains("private"),
            "expected a private-address rejection, got: {err}"
        );
    }
}
