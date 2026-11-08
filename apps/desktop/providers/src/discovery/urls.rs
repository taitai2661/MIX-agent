use super::*;

pub fn http_client() -> anyhow::Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .no_proxy()
        .build()
        .map_err(|e| anyhow::anyhow!("http client: {e}"))
}

/// Join a base URL and a path without doubling the API version segment.
///
/// Local base URLs are inconsistent: the preset catalogue stores
/// `http://127.0.0.1:1234/v1`, LM Studio's own docs show the bare origin, and a
/// user pasting from either place should get the same result. Naive
/// concatenation produces `/v1/v1/models`, which 404s.
pub fn join_api(base_url: &str, path: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');
    let path = path.trim_start_matches('/');
    // Only the version segment is de-duplicated; a caller asking for
    // `/v1/models` against a base that already ends in `/v1` gets one `/v1`.
    let path = if base.ends_with("/v1") {
        path.strip_prefix("v1/").unwrap_or(path)
    } else {
        path
    };
    format!("{}/{}", base, path)
}

/// True for loopback, private-network, and link-local hosts, where an API key
/// is usually not configured. Shared with the OpenAI adapter's own key gate so
/// both agree on what "local" means.
pub fn is_loopback_host(host: &str) -> bool {
    let host = host.trim_matches(|c| c == '[' || c == ']');
    if host.eq_ignore_ascii_case("localhost") || host == "::1" || host == "0.0.0.0" {
        return true;
    }
    if host.starts_with("127.") || host.starts_with("10.") || host.starts_with("192.168.") {
        return true;
    }
    host.starts_with("172.")
        && matches!(
            host.split('.').nth(1).and_then(|o| o.parse::<u8>().ok()),
            Some(second) if (16..=31).contains(&second)
        )
}

/// Extract the host component of a base URL, without a URL-parser dependency.
pub fn host_of(base_url: &str) -> &str {
    let after_scheme = base_url.split("://").nth(1).unwrap_or(base_url);
    let authority = after_scheme.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    // Keep IPv6 literals intact: the only `:` in those separates the port from
    // a bracketed address, never the address itself.
    if let Some(end) = authority.find(']') {
        return &authority[..=end];
    }
    match authority.split_once(':') {
        Some((host, _port)) => host,
        None => authority,
    }
}

/// True when a base URL points at hardware the user controls.
///
/// [`is_loopback_host`] only understands IP literals and `localhost`, but a
/// local inference server is routinely reached under another name: an mDNS
/// host (`mac-mini.local`), a container alias (`host.docker.internal`, the
/// default in the Docker edition), or a Tailscale address (`100.64.0.0/10`).
/// All of those are the user's own machine, so demanding an API key from them
/// turned a working local model into "OpenAI provider requires an API key".
///
/// This is deliberately the *only* place that decides "local" for the key
/// gate, so the scan, the tool probe and the transport cannot disagree.
pub fn is_local_base_url(base_url: &str) -> bool {
    if base_url.trim().is_empty() {
        return false;
    }
    let host = host_of(base_url.trim());
    if is_loopback_host(host) {
        return true;
    }
    let host = host
        .trim_matches(|c| c == '[' || c == ']')
        .to_ascii_lowercase();
    // Container runtimes hand the host a well-known alias; it never leaves the
    // machine even though it is not an IP literal.
    if matches!(
        host.as_str(),
        "host.docker.internal" | "host.containers.internal" | "gateway.docker.internal"
    ) || host.ends_with(".local")
        || host.ends_with(".localhost")
    {
        return true;
    }
    // Tailscale / CGNAT addresses (100.64.0.0/10) are a private overlay
    // network: the server is on the user's own tailnet, not a cloud endpoint.
    if let Some(second) = host
        .split('.')
        .nth(1)
        .and_then(|octet| octet.parse::<u8>().ok())
    {
        if host.starts_with("100.") && (64..=127).contains(&second) {
            return true;
        }
    }
    // IPv6 unique-local (`fc00::/7`) and link-local (`fe80::/10`). Only look at
    // hosts that actually contain a colon, so a hostname is never misread as a
    // hexadecimal address.
    if host.contains(':') {
        if let Some(first) = host.split(':').next() {
            if let Ok(value) = u16::from_str_radix(first, 16) {
                if (0xfc00..=0xfdff).contains(&value) || (0xfe80..=0xfebf).contains(&value) {
                    return true;
                }
            }
        }
    }
    false
}

/// True only when the URL points at this machine's loopback interface.
///
/// [`is_local_base_url`] deliberately includes the LAN, mDNS and overlay
/// networks for the API-key gate: all of those are "local" in the sense that
/// they do not need a cloud credential. Process lifecycle is a different
/// question. Spawning `ollama serve` because a provider points at a GPU box on
/// `192.168.1.20` would start a *second* server on this machine, and sending it
/// the loopback context cap would throttle hardware that may have 24 GB of
/// VRAM. Only a loopback URL can be managed this way.
pub fn is_loopback_base_url(base_url: &str) -> bool {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return false;
    }
    let host = host_of(trimmed)
        .trim_matches(|c| c == '[' || c == ']')
        .to_ascii_lowercase();
    host == "localhost" || host == "::1" || host == "0.0.0.0" || host.starts_with("127.")
}

/// Scheme + authority of a base URL, without the API path.
///
/// Local runtimes disagree about whether their base URL ends in `/v1` (the
/// preset catalogue) or is the bare origin (LM Studio's docs). Native
/// management endpoints live beside `/v1`, not under it, so `/v1` has to be
/// stripped before asking for `/api/tags` or `/api/v0/models`.
pub fn origin_of(base_url: &str) -> String {
    let trimmed = base_url.trim();
    let scheme = trimmed
        .split("://")
        .next()
        .filter(|scheme| *scheme == "http" || *scheme == "https")
        .unwrap_or("http");
    let after_scheme = trimmed.split("://").nth(1).unwrap_or(trimmed);
    let authority = after_scheme.split(['/', '?', '#']).next().unwrap_or("");
    format!("{scheme}://{authority}")
}
