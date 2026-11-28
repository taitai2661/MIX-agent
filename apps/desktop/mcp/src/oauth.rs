//! Remote MCP OAuth 2.0 with PKCE.
//!
//! The Desktop app registers a custom URL scheme (`mix-agent-desktop://`) so
//! the system browser can hand the authorization code back without running a
//! local HTTP listener.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Utc};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::host::SafeHttpClient;

/// OAuth 2.0 endpoints discovered from the MCP server metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthEndpoints {
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub client_id: String,
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PkcePair {
    pub verifier: String,
    pub challenge: String,
    pub state: String,
}

impl Default for PkcePair {
    fn default() -> Self {
        Self::new()
    }
}

impl PkcePair {
    pub fn new() -> Self {
        let mut verifier_bytes = [0u8; 48];
        rand::thread_rng().fill_bytes(&mut verifier_bytes);
        let mut state_bytes = [0u8; 24];
        rand::thread_rng().fill_bytes(&mut state_bytes);
        let verifier = URL_SAFE_NO_PAD.encode(verifier_bytes);
        let state = URL_SAFE_NO_PAD.encode(state_bytes);
        let challenge = Self::challenge_for(&verifier);
        Self {
            verifier,
            challenge,
            state,
        }
    }

    /// RFC 7636 S256 challenge: `BASE64URL(SHA256(verifier))`.
    pub fn challenge_for(verifier: &str) -> String {
        let digest = Sha256::digest(verifier.as_bytes());
        URL_SAFE_NO_PAD.encode(digest)
    }
}

/// How long a pending authorization flow may sit in `mcp_auth_states` before
/// it can no longer be completed. Ten minutes is longer than any real browser
/// redirect, but short enough that a leaked state cannot be replayed days later.
pub const OAUTH_STATE_TTL_SECS: i64 = 10 * 60;

/// Hash the CSRF `state` for storage in `mcp_auth_states.state_hash`.
///
/// The raw state travels in the authorization URL and comes back in the
/// deep-link callback, but a database read must not hand the secret back. Same
/// encoding as the PKCE challenge (base64url of the 32-byte digest), and used
/// for both INSERT and lookup so the two can never disagree.
pub fn hash_state(state: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(state.as_bytes()))
}

/// Whether a pending flow created at `created_at` (an RFC 3339 timestamp) has
/// aged out. Unparsable timestamps fail closed: a corrupted or stale row must
/// never be completed just because its age cannot be read.
pub fn state_is_expired(created_at: &str, now: DateTime<Utc>) -> bool {
    match DateTime::parse_from_rfc3339(created_at) {
        Ok(created) => {
            let age = now.signed_duration_since(created.with_timezone(&Utc));
            // The absolute value also expires a nonsense far-future timestamp;
            // small clock-skew differences stay inside the TTL.
            age.num_seconds().abs() > OAUTH_STATE_TTL_SECS
        }
        Err(_) => true,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRequest {
    pub endpoints: OAuthEndpoints,
    pub pkce: PkcePair,
    pub redirect_uri: String,
    pub resource: Option<String>,
}

impl AuthRequest {
    /// Build the authorization URL the system browser should open.
    pub fn authorization_url(&self) -> anyhow::Result<String> {
        // No silent fallback: a misconfigured authorization endpoint must
        // surface as an error, not as a redirect to a placeholder host that
        // hides the misconfiguration (and could leak query parameters).
        let mut url = url::Url::parse(&self.endpoints.authorization_endpoint)
            .map_err(|e| anyhow::anyhow!("invalid authorization endpoint: {e}"))?;
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("response_type", "code");
            q.append_pair("client_id", &self.endpoints.client_id);
            q.append_pair("redirect_uri", &self.redirect_uri);
            q.append_pair("state", &self.pkce.state);
            q.append_pair("code_challenge", &self.pkce.challenge);
            q.append_pair("code_challenge_method", "S256");
            if !self.endpoints.scopes.is_empty() {
                q.append_pair("scope", &self.endpoints.scopes.join(" "));
            }
            if let Some(resource) = &self.resource {
                q.append_pair("resource", resource);
            }
        }
        Ok(url.to_string())
    }
}

/// The result of a completed redirect.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthCallback {
    pub code: String,
    pub state: String,
}

/// Parse the `mix-agent-desktop://oauth?...` deep link into an [`AuthCallback`].
pub fn parse_callback(url: &str) -> Result<AuthCallback, crate::McpError> {
    use crate::McpError;
    let parsed = url::Url::parse(url).map_err(McpError::CallbackUrl)?;
    let mut code = None;
    let mut state = None;
    for (key, value) in parsed.query_pairs() {
        match key.as_ref() {
            "code" => code = Some(value.to_string()),
            "state" => state = Some(value.to_string()),
            _ => {}
        }
    }
    Ok(AuthCallback {
        code: code.ok_or(McpError::MissingCallbackField("code"))?,
        state: state.ok_or(McpError::MissingCallbackField("state"))?,
    })
}

/// Exchange the authorization code for tokens.
pub async fn exchange_code(
    http: &SafeHttpClient,
    endpoints: &OAuthEndpoints,
    pkce: &PkcePair,
    redirect_uri: &str,
    code: &str,
) -> Result<serde_json::Value, crate::McpError> {
    use crate::McpError;
    /// Labelled into every OAuth error message; `token` and `refresh` are the
    /// only two round trips, so one variant covers both.
    const PHASE: &str = "token";
    let form = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("client_id", endpoints.client_id.as_str()),
        ("code_verifier", pkce.verifier.as_str()),
    ];
    let resp = http
        .client()
        .post(&endpoints.token_endpoint)
        .timeout(std::time::Duration::from_secs(30))
        .form(&form)
        .send()
        .await
        .map_err(|e| McpError::OauthRequest(PHASE, e))?;
    let status = resp.status();
    let value: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| McpError::OauthBody(PHASE, e))?;
    if !status.is_success() {
        return Err(McpError::OauthStatus(
            PHASE,
            status.to_string(),
            value.to_string(),
        ));
    }
    Ok(value)
}

/// Refresh an access token.
pub async fn refresh_token(
    http: &SafeHttpClient,
    endpoints: &OAuthEndpoints,
    refresh_token: &str,
) -> Result<serde_json::Value, crate::McpError> {
    use crate::McpError;
    const PHASE: &str = "refresh";
    let form = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", endpoints.client_id.as_str()),
    ];
    let resp = http
        .client()
        .post(&endpoints.token_endpoint)
        .timeout(std::time::Duration::from_secs(30))
        .form(&form)
        .send()
        .await
        .map_err(|e| McpError::OauthRequest(PHASE, e))?;
    let status = resp.status();
    let value: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| McpError::OauthBody(PHASE, e))?;
    if !status.is_success() {
        return Err(McpError::OauthStatus(
            PHASE,
            status.to_string(),
            value.to_string(),
        ));
    }
    Ok(value)
}

/// The subset of RFC 8414 Authorization Server Metadata this client uses.
#[derive(Debug, Deserialize)]
struct AsMetadata {
    authorization_endpoint: String,
    token_endpoint: String,
}

/// The subset of RFC 9728 Protected Resource Metadata this client uses.
#[derive(Debug, Deserialize)]
struct ResourceMetadata {
    #[serde(default)]
    authorization_servers: Vec<String>,
}

/// Validate an endpoint advertised by server metadata: HTTPS and the same
/// origin as the configured MCP server.
///
/// A metadata document is untrusted input. Without these checks a malicious
/// (or compromised) server could downgrade authorization to plain HTTP or
/// point the browser at `https://evil.example/authorize`, leaking the code and
/// PKCE state to a different origin. Cross-origin authorization servers are
/// intentionally rejected here — discovery must keep the trust boundary at the
/// server the user configured; callers fall back to the legacy paths instead.
pub fn validate_discovered_endpoint(endpoint: &str, server_origin: &str) -> anyhow::Result<String> {
    let parsed = url::Url::parse(endpoint)
        .map_err(|err| anyhow::anyhow!("invalid OAuth endpoint `{endpoint}`: {err}"))?;
    if parsed.scheme() != "https" {
        anyhow::bail!("discovered OAuth endpoint must use https: {endpoint}");
    }
    if parsed.origin().ascii_serialization() != server_origin {
        anyhow::bail!("discovered OAuth endpoint is cross-origin: {endpoint}");
    }
    Ok(parsed.to_string())
}

/// Re-validate stored endpoints immediately before a token exchange.
///
/// `start_oauth` validates endpoints at discovery time, but the callback reads
/// them back from `mcp_connections.data`, which can be edited or imported
/// between the two steps. Re-checking the origin here keeps the PKCE code and
/// verifier from being posted to a different origin. For local `http://` MCP
/// servers the legacy endpoints are same-origin HTTP, which is allowed only
/// when the configured server itself is HTTP.
pub fn validate_flow_endpoints(endpoints: &OAuthEndpoints, base_url: &str) -> anyhow::Result<()> {
    let base = url::Url::parse(base_url)
        .map_err(|err| anyhow::anyhow!("invalid MCP server URL `{base_url}`: {err}"))?;
    let base_origin = base.origin().ascii_serialization();
    let https_only = base.scheme() == "https";
    for endpoint in [&endpoints.authorization_endpoint, &endpoints.token_endpoint] {
        let parsed = url::Url::parse(endpoint)
            .map_err(|err| anyhow::anyhow!("invalid OAuth endpoint `{endpoint}`: {err}"))?;
        if parsed.origin().ascii_serialization() != base_origin {
            anyhow::bail!("stored OAuth endpoint is cross-origin: {endpoint}");
        }
        if https_only && parsed.scheme() != "https" {
            anyhow::bail!(
                "stored OAuth endpoint downgraded to {}: {endpoint}",
                parsed.scheme()
            );
        }
    }
    Ok(())
}

/// Parse an RFC 8414 metadata document into the endpoints used by the PKCE
/// flow. `scopes` is supplied by the caller (the legacy `["mcp"]` default) so a
/// server cannot widen the requested scope through metadata.
pub fn parse_authorization_server_metadata(
    body: &str,
    server_origin: &str,
    client_id: &str,
    scopes: Vec<String>,
) -> anyhow::Result<OAuthEndpoints> {
    let meta: AsMetadata = serde_json::from_str(body)
        .map_err(|err| anyhow::anyhow!("invalid OAuth metadata: {err}"))?;
    Ok(OAuthEndpoints {
        authorization_endpoint: validate_discovered_endpoint(
            &meta.authorization_endpoint,
            server_origin,
        )?,
        token_endpoint: validate_discovered_endpoint(&meta.token_endpoint, server_origin)?,
        client_id: client_id.to_string(),
        scopes,
    })
}

/// Return the same-origin HTTPS authorization server named by RFC 9728 Protected
/// Resource Metadata, if any. Cross-origin / HTTP issuers are ignored.
pub fn protected_resource_authorization_server(body: &str, server_origin: &str) -> Option<String> {
    let meta: ResourceMetadata = serde_json::from_str(body).ok()?;
    meta.authorization_servers.into_iter().find_map(|issuer| {
        let parsed = url::Url::parse(&issuer).ok()?;
        if parsed.scheme() == "https" && parsed.origin().ascii_serialization() == server_origin {
            Some(parsed.origin().ascii_serialization())
        } else {
            None
        }
    })
}

fn legacy_endpoints(origin: &str, client_id: &str, scopes: Vec<String>) -> OAuthEndpoints {
    OAuthEndpoints {
        authorization_endpoint: format!("{origin}/authorize"),
        token_endpoint: format!("{origin}/token"),
        client_id: client_id.to_string(),
        scopes,
    }
}

/// GET a metadata document, bounded in time and gated by the same private-IP
/// policy as the MCP HTTP transport so discovery cannot become an SSRF route.
async fn get_metadata(http: &SafeHttpClient, url: &str) -> anyhow::Result<String> {
    // The caller built `http` for this origin with addresses pinned and
    // redirects disabled; no second DNS lookup or 3xx hop can redirect the
    // metadata fetch into a private network.
    let resp = http
        .client()
        .get(url)
        .header("accept", "application/json")
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await?;
    if !resp.status().is_success() {
        anyhow::bail!("metadata GET {url} returned {}", resp.status());
    }
    Ok(resp.text().await?)
}

async fn fetch_as_metadata(
    http: &SafeHttpClient,
    as_origin: &str,
    server_origin: &str,
    client_id: &str,
    scopes: &[String],
) -> anyhow::Result<OAuthEndpoints> {
    let body = get_metadata(
        http,
        &format!("{as_origin}/.well-known/oauth-authorization-server"),
    )
    .await?;
    parse_authorization_server_metadata(&body, server_origin, client_id, scopes.to_vec())
}

/// Discover OAuth endpoints for an MCP server, falling back to the legacy
/// `{origin}/authorize` / `{origin}/token` paths when no usable metadata is
/// published.
///
/// Order: RFC 8414 AS metadata at the server origin, then RFC 9728 Protected
/// Resource Metadata naming a (same-origin) authorization server. Both metadata
/// fetches are best-effort — a server that does not publish them keeps working
/// exactly as before. Any metadata that fails HTTPS / same-origin validation is
/// discarded rather than partially trusted.
pub async fn discover_endpoints(
    http: &SafeHttpClient,
    base_url: &str,
    client_id: &str,
    scopes: Vec<String>,
) -> OAuthEndpoints {
    let origin = url::Url::parse(base_url)
        .map(|u| u.origin().ascii_serialization())
        .unwrap_or_else(|_| base_url.trim_end_matches('/').to_string());
    let legacy = legacy_endpoints(&origin, client_id, scopes.clone());
    // Metadata served over plain HTTP is not trustworthy: an on-path attacker
    // could choose the authorization endpoint. Discovery is https-only; local
    // http MCP servers keep the legacy paths they used before this change.
    if !origin.starts_with("https://") {
        return legacy;
    }
    match fetch_as_metadata(http, &origin, &origin, client_id, &scopes).await {
        Ok(endpoints) => return endpoints,
        Err(err) => tracing::debug!(%err, "MCP OAuth authorization-server metadata unavailable"),
    }
    let resource_url = format!("{origin}/.well-known/oauth-protected-resource");
    if let Ok(body) = get_metadata(http, &resource_url).await {
        if let Some(as_origin) = protected_resource_authorization_server(&body, &origin) {
            match fetch_as_metadata(http, &as_origin, &origin, client_id, &scopes).await {
                Ok(endpoints) => return endpoints,
                Err(err) => {
                    tracing::debug!(%err, "MCP OAuth AS metadata unavailable after protected-resource discovery")
                }
            }
        }
    }
    legacy
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_matches_rfc7636_example() {
        // RFC 7636 Appendix B.
        assert_eq!(
            PkcePair::challenge_for("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn parses_deep_link_callback() {
        let cb = parse_callback("mix-agent-desktop://oauth?code=abc123&state=xyz").expect("parse");
        assert_eq!(cb.code, "abc123");
        assert_eq!(cb.state, "xyz");
    }

    #[test]
    fn missing_code_is_an_error() {
        assert!(parse_callback("mix-agent-desktop://oauth?state=xyz").is_err());
    }

    #[test]
    fn builds_authorization_url() {
        let req = AuthRequest {
            endpoints: OAuthEndpoints {
                authorization_endpoint: "https://auth.example.com/authorize".into(),
                token_endpoint: "https://auth.example.com/token".into(),
                client_id: "mixd".into(),
                scopes: vec!["mcp".into()],
            },
            pkce: PkcePair::new(),
            redirect_uri: "mix-agent-desktop://oauth".into(),
            resource: None,
        };
        let url = req.authorization_url().expect("valid endpoint");
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("client_id=mixd"));
        assert!(url.contains("scope=mcp"));
    }

    #[test]
    fn invalid_endpoint_is_an_error_not_a_fallback() {
        let req = AuthRequest {
            endpoints: OAuthEndpoints {
                authorization_endpoint: "://not a url".into(),
                token_endpoint: "https://auth.example.com/token".into(),
                client_id: "mixd".into(),
                scopes: vec![],
            },
            pkce: PkcePair::new(),
            redirect_uri: "mix-agent-desktop://oauth".into(),
            resource: None,
        };
        assert!(req.authorization_url().is_err());
    }

    #[test]
    fn state_hash_is_a_stable_sha256_digest_not_the_secret() {
        let state = "a-random-csrf-secret";
        let hash = hash_state(state);
        assert_eq!(hash, hash_state(state), "INSERT and lookup must agree");
        assert_ne!(hash, hash_state("another-state"));
        assert!(!hash.contains(state));
        assert_eq!(hash.len(), 43, "base64url of 32 bytes without padding");
    }

    #[test]
    fn stale_and_unparsable_oauth_states_are_rejected() {
        let now = Utc::now();
        let fresh = (now - chrono::Duration::seconds(30)).to_rfc3339();
        let stale = (now - chrono::Duration::seconds(OAUTH_STATE_TTL_SECS + 1)).to_rfc3339();
        let future = (now + chrono::Duration::seconds(OAUTH_STATE_TTL_SECS + 1)).to_rfc3339();
        assert!(!state_is_expired(&fresh, now), "a fresh flow is usable");
        assert!(
            state_is_expired(&stale, now),
            "a stale flow must be refused"
        );
        assert!(
            state_is_expired(&future, now),
            "a nonsense far-future timestamp must be refused"
        );
        assert!(
            state_is_expired("not-a-timestamp", now),
            "fail closed on a corrupt row"
        );
    }

    #[test]
    fn rfc8414_metadata_is_parsed_and_bad_endpoints_rejected() {
        const BODY: &str = r#"{
            "issuer": "https://mcp.example.com",
            "authorization_endpoint": "https://mcp.example.com/oauth/authorize",
            "token_endpoint": "https://mcp.example.com/oauth/token",
            "response_types_supported": ["code"]
        }"#;
        let ep = parse_authorization_server_metadata(
            BODY,
            "https://mcp.example.com",
            "mixd",
            vec!["mcp".into()],
        )
        .expect("valid metadata");
        assert_eq!(
            ep.authorization_endpoint,
            "https://mcp.example.com/oauth/authorize"
        );
        assert_eq!(ep.token_endpoint, "https://mcp.example.com/oauth/token");
        assert_eq!(ep.scopes, vec!["mcp".to_string()]);

        let cross_origin = BODY.replace(
            "https://mcp.example.com/oauth/authorize",
            "https://evil.example/authorize",
        );
        assert!(
            parse_authorization_server_metadata(
                &cross_origin,
                "https://mcp.example.com",
                "mixd",
                vec![]
            )
            .is_err(),
            "a cross-origin authorization endpoint must be refused"
        );

        let downgraded = BODY.replace(
            "https://mcp.example.com/oauth/token",
            "http://mcp.example.com/token",
        );
        assert!(
            parse_authorization_server_metadata(
                &downgraded,
                "https://mcp.example.com",
                "mixd",
                vec![]
            )
            .is_err(),
            "a plain-HTTP token endpoint must be refused"
        );
    }

    #[test]
    fn stored_endpoints_are_revalidated_against_the_server_url() {
        let endpoints = OAuthEndpoints {
            authorization_endpoint: "https://mcp.example.com/authorize".into(),
            token_endpoint: "https://mcp.example.com/token".into(),
            client_id: "mixd".into(),
            scopes: vec!["mcp".into()],
        };
        assert!(validate_flow_endpoints(&endpoints, "https://mcp.example.com/mcp").is_ok());
        assert!(validate_flow_endpoints(&endpoints, "https://evil.example/mcp").is_err());
        let downgraded = OAuthEndpoints {
            token_endpoint: "http://mcp.example.com/token".into(),
            ..endpoints.clone()
        };
        assert!(validate_flow_endpoints(&downgraded, "https://mcp.example.com/mcp").is_err());
        // A local HTTP server keeps same-origin legacy endpoints.
        let local = OAuthEndpoints {
            authorization_endpoint: "http://127.0.0.1:3000/authorize".into(),
            token_endpoint: "http://127.0.0.1:3000/token".into(),
            ..endpoints
        };
        assert!(validate_flow_endpoints(&local, "http://127.0.0.1:3000/mcp").is_ok());
    }

    #[test]
    fn protected_resource_metadata_only_accepts_same_origin_https() {
        let body = r#"{
            "resource": "https://mcp.example.com/mcp",
            "authorization_servers": ["https://mcp.example.com", "https://evil.example"]
        }"#;
        assert_eq!(
            protected_resource_authorization_server(body, "https://mcp.example.com").as_deref(),
            Some("https://mcp.example.com")
        );
        let cross_only = r#"{"authorization_servers": ["https://evil.example"]}"#;
        assert_eq!(
            protected_resource_authorization_server(cross_only, "https://mcp.example.com"),
            None
        );
        let http_only = r#"{"authorization_servers": ["http://mcp.example.com"]}"#;
        assert_eq!(
            protected_resource_authorization_server(http_only, "https://mcp.example.com"),
            None
        );
    }

    #[test]
    fn legacy_endpoints_are_the_no_metadata_fallback() {
        let ep = legacy_endpoints("https://mcp.example.com", "mixd", vec!["mcp".into()]);
        assert_eq!(
            ep.authorization_endpoint,
            "https://mcp.example.com/authorize"
        );
        assert_eq!(ep.token_endpoint, "https://mcp.example.com/token");
    }
}
