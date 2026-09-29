//! Remote MCP OAuth 2.0 with PKCE.
//!
//! The Desktop app registers a custom URL scheme (`mix-agent-desktop://`) so
//! the system browser can hand the authorization code back without running a
//! local HTTP listener.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRequest {
    pub endpoints: OAuthEndpoints,
    pub pkce: PkcePair,
    pub redirect_uri: String,
    pub resource: Option<String>,
}

impl AuthRequest {
    /// Build the authorization URL the system browser should open.
    pub fn authorization_url(&self) -> String {
        let mut url = url::Url::parse(&self.endpoints.authorization_endpoint)
            .unwrap_or_else(|_| url::Url::parse("https://invalid.local").expect("static url"));
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
        url.to_string()
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
    http: &reqwest::Client,
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
    http: &reqwest::Client,
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
        let url = req.authorization_url();
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("client_id=mixd"));
        assert!(url.contains("scope=mcp"));
    }
}
