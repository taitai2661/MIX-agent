//! Egress allowlist + private-IP deny for outbound Tool calls.
//!
//! Tools should call [`enforce`] before opening any outbound HTTP connection.
//! The allowlist is a vector of exact hostnames or `*.example.com` patterns;
//! when the list is empty the gate is wide-open (matches the Docker edition's
//! behaviour of "no allowlist set ⇒ allow public domains, deny private IPs").

use std::net::IpAddr;

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

    pub fn enforce(&self, url: &str) -> anyhow::Result<()> {
        let parsed = url::Url::parse(url).map_err(|e| anyhow!("invalid url: {}", e))?;
        let host = parsed
            .host_str()
            .ok_or_else(|| anyhow!("url is missing a host: {}", url))?;
        let port = parsed.port_or_known_default();
        if self.allowlist.is_empty() {
            // Public-domain only, deny private IPs.
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
        let _ = port;
        Ok(())
    }
}

fn ensure_public_host(host: &str) -> anyhow::Result<()> {
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

fn is_private(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_private() || v4.is_loopback() || v4.is_link_local(),
        IpAddr::V6(v6) => v6.is_loopback() || v6.segments()[0] == 0xfe80,
    }
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
}
