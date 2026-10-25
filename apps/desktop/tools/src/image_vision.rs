//! Vision Tool: validate image attachments and surface a base64 preview
//! suitable for attaching to a multimodal Provider request.

use async_trait::async_trait;
use base64::Engine;

use super::{Tool, ToolContext, ToolResult};

#[derive(Default)]
pub struct ImageVisionTool;

const MAX_BYTES: usize = 20 * 1024 * 1024;

#[async_trait]
impl Tool for ImageVisionTool {
    fn id(&self) -> &str {
        "image_vision"
    }
    /// Pure decode of bytes the caller already supplied.
    fn is_sensitive(&self, _args: &serde_json::Value) -> bool {
        false
    }
    fn name(&self) -> &str {
        "Image Vision"
    }
    fn description(&self) -> &str {
        "Decode a base64 image and return size + dimensions hint."
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "mime_type": { "type": "string" },
                "data_base64": { "type": "string" }
            },
            "required": ["mime_type", "data_base64"]
        })
    }
    async fn invoke(
        &self,
        _ctx: ToolContext,
        args: serde_json::Value,
    ) -> anyhow::Result<ToolResult> {
        let mime = args.get("mime_type").and_then(|v| v.as_str()).unwrap_or("");
        let data = args
            .get("data_base64")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        // Reject an oversized payload *before* decoding it: `decode` allocates
        // the whole output buffer, so a caller-supplied gigabyte of base64 would
        // be materialised before the size check below could run. Base64 expands
        // to at most 3/4 of the encoded length (padding only makes it smaller).
        if data.len() / 4 * 3 > MAX_BYTES {
            anyhow::bail!("image exceeds 20 MiB limit");
        }
        let decoded = base64::engine::general_purpose::STANDARD.decode(data.as_bytes())?;
        if decoded.len() > MAX_BYTES {
            anyhow::bail!("image exceeds 20 MiB limit");
        }
        // `data` is model-controlled and only guaranteed to be UTF-8: slicing
        // it by byte length panicked whenever byte 64 fell inside a multi-byte
        // character. Taking characters cannot split one, and the preview is
        // informational rather than a decodable data URL either way.
        let preview_prefix: String = data.chars().take(64).collect();
        Ok(ToolResult::ok(serde_json::json!({
            "mime_type": mime,
            "bytes": decoded.len(),
            "preview": format!("data:{};base64,{}", mime, preview_prefix),
        })))
    }
}
