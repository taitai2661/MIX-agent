//! Network Tool: HTTP fetch with egress policy + optional DuckDuckGo search.

use async_trait::async_trait;

use super::egress::{EgressPolicy, ResolvedTarget};
use super::{Tool, ToolContext, ToolResult};

#[derive(Default)]
pub struct NetworkTool {
    pub egress: EgressPolicy,
}

/// Shared base builder for Tool HTTP calls.
///
/// Built once per process instead of per call (a fresh connection pool each
/// time). The redirect policy is turned off so the network Tool can re-evaluate
/// every hop through the egress policy; without it reqwest follows up to ten
/// redirects on its own, and a permitted host can bounce the request straight
/// at the cloud metadata address before the policy ever sees the target.
fn base_client() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .user_agent("mix-agent-desktop/0.1")
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
}

/// Build a `reqwest::Client` whose DNS is pinned to the addresses the egress
/// policy approved for this URL.
///
/// The DNS rebinding class of attack checks the hostname *before* connecting
/// and then trusts the resolver not to change its mind. Pinning the resolver
/// to the vetted addresses forces reqwest to dial the IPs we approved, even if
/// a TTL flip happens between the two lookups.
fn client_for(target: &ResolvedTarget) -> anyhow::Result<reqwest::Client> {
    let mut builder = base_client();
    let addrs: Vec<std::net::SocketAddr> = target.addrs.clone();
    builder = builder.resolve_to_addrs(&target.host, &addrs);
    builder
        .build()
        .map_err(|e| anyhow::anyhow!("building pinned client failed: {}", e))
}

/// Redirect hops followed before giving up; matches reqwest's own default.
const MAX_REDIRECTS: u8 = 10;

impl NetworkTool {
    pub fn new(egress: EgressPolicy) -> Self {
        Self { egress }
    }
}

#[async_trait]
impl Tool for NetworkTool {
    fn id(&self) -> &str {
        "network"
    }
    fn name(&self) -> &str {
        "Network"
    }
    fn description(&self) -> &str {
        "GET / POST JSON / text with egress allowlist and private-IP deny."
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "Absolute http(s) URL" },
                "method": { "type": "string", "enum": ["GET", "POST", "PUT", "DELETE"], "default": "GET" },
                "headers": { "type": "object", "additionalProperties": { "type": "string" } },
                "body": { "description": "Optional JSON / string body" }
            },
            "required": ["url"],
            "additionalProperties": false
        })
    }
    fn pattern_target(&self, args: &serde_json::Value) -> Option<String> {
        crate::args::pattern_arg(args, "url")
    }
    async fn invoke(
        &self,
        _ctx: ToolContext,
        args: serde_json::Value,
    ) -> anyhow::Result<ToolResult> {
        let start = args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("url is required"))?
            .to_string();
        let method = args
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("GET")
            .parse::<reqwest::Method>()
            .unwrap_or(reqwest::Method::GET);
        let headers = args
            .get("headers")
            .cloned()
            .unwrap_or(serde_json::json!({}));
        let body = args.get("body").cloned();

        // Every hop is checked: the first URL passed the allowlist, but a 302
        // from it points wherever the server wants. Each hop gets a fresh
        // reqwest client pinned to the addresses vetted for that URL.
        let mut url = start.clone();
        let mut method = method;
        let mut send_body = true;
        let mut hop = 0u8;
        let response = loop {
            let target = self.egress.resolve_async(&url).await?;
            let client = client_for(&target)?;
            let mut req = client.request(method.clone(), &target.url);
            if let Some(obj) = headers.as_object() {
                for (k, v) in obj {
                    if let Some(s) = v.as_str() {
                        req = req.header(k, s);
                    }
                }
            }
            // 307/308 keep method and payload; the downgrade to GET (303, and
            // 301/302 for non-GET) drops it, as browsers do.
            if send_body {
                if let Some(body) = &body {
                    if body.is_string() {
                        if let Some(s) = body.as_str() {
                            req = req.body(s.to_string());
                        }
                    } else {
                        req = req.json(body);
                    }
                }
            }
            let response = req.send().await?;
            let status = response.status();
            if !status.is_redirection() {
                break response;
            }
            hop += 1;
            if hop > MAX_REDIRECTS {
                anyhow::bail!("too many redirects fetching {}", start);
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| anyhow::anyhow!("redirect without a Location header"))?;
            url = url::Url::parse(&url)
                .and_then(|base| base.join(location))
                .map_err(|e| anyhow::anyhow!("bad redirect target: {}", e))?
                .to_string();
            if status == reqwest::StatusCode::SEE_OTHER
                || ((status == reqwest::StatusCode::MOVED_PERMANENTLY
                    || status == reqwest::StatusCode::FOUND)
                    && method != reqwest::Method::GET)
            {
                method = reqwest::Method::GET;
                send_body = false;
            }
        };
        let status = response.status().as_u16();
        let text = response.text().await?;
        Ok(ToolResult {
            ok: status < 400,
            content: serde_json::json!({ "status": status, "body": text }),
            error: if status >= 400 {
                Some(format!("HTTP {}", status))
            } else {
                None
            },
        })
    }
}

#[derive(Default)]
pub struct WebSearchTool;

#[async_trait]
impl Tool for WebSearchTool {
    fn id(&self) -> &str {
        "web_search"
    }
    /// A query against a fixed search engine. Note this is deliberately *not*
    /// shared with `web_fetch`: a model-chosen URL can exfiltrate whatever is
    /// in the prompt, and that one is gated.
    fn is_sensitive(&self, _args: &serde_json::Value) -> bool {
        false
    }
    fn name(&self) -> &str {
        "Web search"
    }
    fn description(&self) -> &str {
        "DuckDuckGo search and HTML extraction."
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "max_results": { "type": "integer", "default": 6 }
            },
            "required": ["query"]
        })
    }
    async fn invoke(
        &self,
        _ctx: ToolContext,
        args: serde_json::Value,
    ) -> anyhow::Result<ToolResult> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("query required"))?;
        let max = args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(6);
        let ddgs = ddgs::Ddgs::new().map_err(|e| anyhow::anyhow!("ddgs init: {}", e))?;
        let results = ddgs
            .text(query)
            .await
            .map_err(|e| anyhow::anyhow!("ddgs search: {}", e))?
            .into_iter()
            .take(max as usize)
            .map(|r| serde_json::json!({ "title": r.title, "url": r.href, "snippet": r.body }))
            .collect::<Vec<_>>();
        Ok(ToolResult::ok(serde_json::json!(results)))
    }
}
