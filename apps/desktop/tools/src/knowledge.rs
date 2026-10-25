//! Agent-facing Knowledge store: add / search / delete / web-clip.
//!
//! Docker parity for `knowledge_add`, `knowledge_search`, `knowledge_delete`
//! and `web_clip_save`, consolidated into one tool in the Desktop style
//! (`files`, `memory`). Search stays available on the `memory` tool as
//! `search_knowledge` for compatibility; this tool is the writable surface.
//!
//! Storage lives behind [`KnowledgeDispatch`] (implemented by the app crate),
//! the same pattern as [`crate::MemoryDispatch`], because the tools crate
//! cannot depend on `mixd-core`.

use std::sync::Arc;

use async_trait::async_trait;

use super::egress::EgressPolicy;
use super::network::NetworkTool;
use super::{Tool, ToolContext, ToolResult};

const MAX_TEXT_CHARS: usize = 100_000;
const MAX_CLIP_CHARS: usize = 30_000;

#[async_trait]
pub trait KnowledgeDispatch: Send + Sync {
    async fn ingest(&self, filename: &str, text: &str) -> anyhow::Result<serde_json::Value>;
    async fn delete(&self, doc_id: &str) -> anyhow::Result<serde_json::Value>;
    async fn search(&self, query: &str, limit: usize) -> anyhow::Result<serde_json::Value>;
}

pub struct KnowledgeTool {
    dispatch: Arc<dyn KnowledgeDispatch>,
    network: NetworkTool,
}

impl KnowledgeTool {
    pub fn new(dispatch: Arc<dyn KnowledgeDispatch>, egress: EgressPolicy) -> Self {
        Self {
            dispatch,
            network: NetworkTool::new(egress),
        }
    }
}

/// Remove `<script>` / `<style>` blocks together with their bodies.
///
/// `regex` has no backreference, so a combined `<(script|style)[^>]*>.*?</\1>`
/// never compiles: the old code then fell back to the raw input, and the later
/// tag-stripping pass removed only the tags while leaving the script and style
/// *bodies* in the extracted text. Match the two elements separately instead.
fn strip_element_blocks(html: &str) -> String {
    let mut out = html.to_string();
    for pattern in [
        r"(?is)<script[^>]*>.*?</script>",
        r"(?is)<style[^>]*>.*?</style>",
    ] {
        match regex::Regex::new(pattern) {
            Ok(re) => out = re.replace_all(&out, " ").to_string(),
            // The patterns above are literals; keep the old "give up, pass the
            // input through" behaviour rather than dropping what we already cut.
            Err(_) => return html.to_string(),
        }
    }
    out
}

/// Strip HTML tags to plain text: good enough for a v1 clip.
fn html_to_text(html: &str) -> String {
    let without_scripts = strip_element_blocks(html);
    let without_tags = regex::Regex::new(r"<[^>]+>")
        .map(|re| re.replace_all(&without_scripts, " ").to_string())
        .unwrap_or(without_scripts);
    regex::Regex::new(r"\s+")
        .map(|re| re.replace_all(&without_tags, " ").trim().to_string())
        .unwrap_or(without_tags)
        .trim()
        .to_string()
}

fn clip_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    text.chars().take(max).collect()
}

#[async_trait]
impl Tool for KnowledgeTool {
    fn id(&self) -> &str {
        "knowledge"
    }
    fn name(&self) -> &str {
        "Knowledge"
    }
    fn description(&self) -> &str {
        "Save text into the searchable knowledge store (add), search it (search), delete a document (delete), or fetch a public web page and save it in one step (clip). Never include credentials."
    }
    /// Only `search` is free; everything else writes or reaches the network.
    fn is_sensitive(&self, args: &serde_json::Value) -> bool {
        !matches!(args.get("action").and_then(|v| v.as_str()), Some("search"))
    }
    fn pattern_target(&self, args: &serde_json::Value) -> Option<String> {
        args.get("doc_id")
            .or_else(|| args.get("id"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .or_else(|| {
                args.get("url")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["add", "search", "delete", "clip"] },
                "filename": { "type": "string", "description": "Filename for add/clip" },
                "text": { "type": "string", "description": "Text content for add" },
                "memo": { "type": "string", "description": "Optional note stored with the document" },
                "query": { "type": "string", "description": "Search query" },
                "limit": { "type": "integer", "default": 8 },
                "doc_id": { "type": "string", "description": "Knowledge document id for delete" },
                "id": { "type": "string", "description": "Alias for doc_id" },
                "url": { "type": "string", "description": "Public http(s) URL for clip" },
                "title": { "type": "string", "description": "Optional title for clip" }
            },
            "required": ["action"],
            "additionalProperties": false
        })
    }
    async fn invoke(
        &self,
        ctx: ToolContext,
        args: serde_json::Value,
    ) -> anyhow::Result<ToolResult> {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");
        match action {
            "search" => {
                let query = args
                    .get("query")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                if query.is_empty() {
                    return Ok(ToolResult::fail("query is required for search"));
                }
                let limit = args
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(8)
                    .clamp(1, 20) as usize;
                Ok(ToolResult::ok(self.dispatch.search(query, limit).await?))
            }
            "add" => {
                let text = args
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                if text.is_empty() {
                    return Ok(ToolResult::fail("text is required for add"));
                }
                if text.chars().count() > MAX_TEXT_CHARS {
                    return Ok(ToolResult::fail(format!(
                        "text is too long (max {MAX_TEXT_CHARS} chars)"
                    )));
                }
                let filename = args
                    .get("filename")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or("note.txt")
                    .trim();
                let memo = args
                    .get("memo")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                let stored = if memo.is_empty() {
                    text.to_string()
                } else {
                    format!("> {memo}\n\n{text}")
                };
                Ok(ToolResult::ok(
                    self.dispatch.ingest(filename, &stored).await?,
                ))
            }
            "delete" => {
                let doc_id = args
                    .get("doc_id")
                    .or_else(|| args.get("id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if doc_id.is_empty() {
                    return Ok(ToolResult::fail("doc_id is required for delete"));
                }
                Ok(ToolResult::ok(self.dispatch.delete(&doc_id).await?))
            }
            "clip" => {
                let url = args
                    .get("url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                if url.is_empty() {
                    return Ok(ToolResult::fail("url is required for clip"));
                }
                // Fetch through the egress-gated network tool so private IPs
                // and non-allowlisted hosts are rejected like `web_fetch`.
                let fetch = self
                    .network
                    .invoke(ctx, serde_json::json!({ "url": url, "method": "GET" }))
                    .await?;
                if !fetch.ok {
                    return Ok(ToolResult::fail(
                        fetch.error.unwrap_or_else(|| "fetch failed".to_string()),
                    ));
                }
                let body = fetch
                    .content
                    .get("body")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let text = clip_chars(&html_to_text(body), MAX_CLIP_CHARS);
                if text.trim().is_empty() {
                    return Ok(ToolResult::fail("no readable text at that URL"));
                }
                let title = args
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                let memo = args
                    .get("memo")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                let filename = if title.is_empty() {
                    "web-clip.txt"
                } else {
                    title
                };
                let stored = if memo.is_empty() {
                    format!("Source: {url}\n\n{text}")
                } else {
                    format!("Source: {url}\n> {memo}\n\n{text}")
                };
                Ok(ToolResult::ok(
                    self.dispatch.ingest(filename, &stored).await?,
                ))
            }
            other => Ok(ToolResult::fail(format!(
                "unknown knowledge action: {other}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    struct MemDispatch {
        docs: StdMutex<Vec<(String, String)>>,
    }

    #[async_trait]
    impl KnowledgeDispatch for MemDispatch {
        async fn ingest(&self, filename: &str, text: &str) -> anyhow::Result<serde_json::Value> {
            let id = format!("doc-{}", self.docs.lock().unwrap().len() + 1);
            self.docs
                .lock()
                .unwrap()
                .push((id.clone(), text.to_string()));
            Ok(serde_json::json!({ "doc_id": id, "filename": filename }))
        }
        async fn delete(&self, doc_id: &str) -> anyhow::Result<serde_json::Value> {
            self.docs.lock().unwrap().retain(|(id, _)| id != doc_id);
            Ok(serde_json::json!({ "deleted": doc_id }))
        }
        async fn search(&self, query: &str, _limit: usize) -> anyhow::Result<serde_json::Value> {
            Ok(serde_json::json!({ "knowledge": query }))
        }
    }

    fn tool() -> KnowledgeTool {
        KnowledgeTool::new(
            Arc::new(MemDispatch {
                docs: StdMutex::new(Vec::new()),
            }),
            EgressPolicy::default(),
        )
    }

    fn ctx() -> ToolContext {
        ToolContext {
            run_id: uuid::Uuid::new_v4(),
            workspace_id: uuid::Uuid::new_v4(),
            root_path: None,
            additional_paths: Vec::new(),
            approval_token: None,
        }
    }

    #[test]
    fn only_search_is_free() {
        let t = tool();
        assert!(!t.is_sensitive(&serde_json::json!({"action": "search"})));
        assert!(t.is_sensitive(&serde_json::json!({"action": "add"})));
        assert!(t.is_sensitive(&serde_json::json!({"action": "delete"})));
        assert!(t.is_sensitive(&serde_json::json!({"action": "clip"})));
    }

    #[tokio::test]
    async fn add_requires_text() {
        let t = tool();
        let r = t
            .invoke(ctx(), serde_json::json!({"action": "add", "text": ""}))
            .await
            .unwrap();
        assert!(!r.ok);
    }

    #[tokio::test]
    async fn add_and_delete_round_trip() {
        let t = tool();
        let added = t
            .invoke(
                ctx(),
                serde_json::json!({"action": "add", "filename": "a.txt", "text": "hello"}),
            )
            .await
            .unwrap();
        assert!(added.ok);
        let id = added.content["doc_id"].as_str().unwrap().to_string();
        let deleted = t
            .invoke(ctx(), serde_json::json!({"action": "delete", "doc_id": id}))
            .await
            .unwrap();
        assert!(deleted.ok);
    }

    #[tokio::test]
    async fn search_requires_query() {
        let t = tool();
        let r = t
            .invoke(ctx(), serde_json::json!({"action": "search"}))
            .await
            .unwrap();
        assert!(!r.ok);
    }

    #[tokio::test]
    async fn unknown_action_fails() {
        let t = tool();
        let r = t
            .invoke(ctx(), serde_json::json!({"action": "drop"}))
            .await
            .unwrap();
        assert!(!r.ok);
    }

    #[test]
    fn html_is_stripped() {
        let out = html_to_text("<html><body><h1>Hi</h1><p>there</p></body></html>");
        assert!(out.contains("Hi"));
        assert!(!out.contains('<'));
    }
}
