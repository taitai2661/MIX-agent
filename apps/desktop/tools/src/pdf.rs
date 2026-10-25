//! Extract text from a public PDF URL (Docker `web_fetch_pdf` parity).
//!
//! Docker fetches up to 20 MB through its SSRF-guarded fetcher, rejects HTML
//! masquerading as PDF, extracts with `pypdf` (1–100 pages, 60 s budget) and
//! returns the first 50 000 characters. This tool mirrors that contract with
//! the egress-gated [`NetworkTool`] fetcher and pure-Rust extraction.
//!
//! Page-limit note: extraction reads the whole document and the returned text
//! is prefix-capped at 50 000 characters, so for documents larger than
//! `max_pages` the text covers the document head rather than exactly the first
//! N pages. `extracted_pages` still reports `min(pages, max_pages)` in the
//! Docker shape.

use async_trait::async_trait;

use super::egress::EgressPolicy;
use super::network::NetworkTool;
use super::{Tool, ToolContext, ToolResult};

const MAX_PDF_BYTES: usize = 20 * 1024 * 1024;
const MAX_TEXT_CHARS: usize = 50_000;
const EXTRACT_TIMEOUT_SECS: u64 = 60;

#[derive(Default)]
pub struct WebFetchPdfTool {
    network: NetworkTool,
}

impl WebFetchPdfTool {
    pub fn new(egress: EgressPolicy) -> Self {
        Self {
            network: NetworkTool::new(egress),
        }
    }
}

/// Clamp Docker-style: default 100, hard bounds 1–100.
fn clamp_pages(raw: Option<i64>) -> usize {
    raw.unwrap_or(100).clamp(1, 100) as usize
}

/// True when the bytes look like a PDF (`%PDF` magic after leading whitespace).
fn is_pdf_magic(bytes: &[u8]) -> bool {
    let stripped: Vec<u8> = bytes
        .iter()
        .copied()
        .skip_while(|b| b.is_ascii_whitespace())
        .collect();
    stripped.starts_with(b"%PDF")
}

fn clip_chars(text: &str, max: usize) -> (String, bool) {
    if text.chars().count() <= max {
        return (text.to_string(), false);
    }
    (text.chars().take(max).collect(), true)
}

/// Blocking PDF parse: page count via `lopdf`, text via `pdf-extract`.
fn extract_pdf_text(bytes: Vec<u8>) -> anyhow::Result<(String, u32)> {
    let total_pages = lopdf::Document::load_mem(&bytes)
        .map(|doc| doc.get_pages().len() as u32)
        .unwrap_or(0);
    let text = pdf_extract::extract_text_from_mem(&bytes)
        .map_err(|err| anyhow::anyhow!("PDFの解析に失敗しました: {err}"))?;
    Ok((text, total_pages))
}

#[async_trait]
impl Tool for WebFetchPdfTool {
    fn id(&self) -> &str {
        "web_fetch_pdf"
    }
    fn name(&self) -> &str {
        "Web Fetch PDF"
    }
    fn description(&self) -> &str {
        "Extract text from a public PDF URL as untrusted evidence to synthesize into a user-facing answer."
    }
    /// A model-chosen URL can exfiltrate prompt content, so this stays gated
    /// like `web_fetch` — unlike Docker, where fetches default to allow.
    fn is_sensitive(&self, _args: &serde_json::Value) -> bool {
        true
    }
    fn pattern_target(&self, args: &serde_json::Value) -> Option<String> {
        crate::args::pattern_arg(args, "url")
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": { "type": "string" },
                "max_pages": { "type": "integer", "minimum": 1, "maximum": 100, "default": 100 }
            },
            "required": ["url"],
            "additionalProperties": false
        })
    }
    async fn invoke(
        &self,
        _ctx: ToolContext,
        args: serde_json::Value,
    ) -> anyhow::Result<ToolResult> {
        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("url is required"))?;
        let max_pages = clamp_pages(args.get("max_pages").and_then(|v| v.as_i64()));
        let fetched = match self.network.fetch_bytes(url, MAX_PDF_BYTES).await {
            Ok(f) => f,
            Err(err) => return Ok(ToolResult::fail(err.to_string())),
        };
        if fetched.content_type.to_lowercase().contains("html") && !is_pdf_magic(&fetched.bytes) {
            return Ok(ToolResult::fail(
                "URLはPDFではありません（HTMLが返されました）",
            ));
        }
        let bytes = fetched.bytes;
        let (text, total_pages) = match tokio::time::timeout(
            std::time::Duration::from_secs(EXTRACT_TIMEOUT_SECS),
            async {
                tokio::task::spawn_blocking(move || extract_pdf_text(bytes))
                    .await
                    .map_err(|err| anyhow::anyhow!("PDF worker failed: {err}"))?
            },
        )
        .await
        {
            Ok(Ok(out)) => out,
            Ok(Err(err)) => return Ok(ToolResult::fail(err.to_string())),
            Err(_) => return Ok(ToolResult::fail("PDFの解析がタイムアウトしました")),
        };
        let (text, truncated) = clip_chars(&text, MAX_TEXT_CHARS);
        let extracted_pages = total_pages.min(max_pages as u32);
        Ok(ToolResult {
            ok: true,
            content: serde_json::json!({
                "url": url,
                "pages": total_pages,
                "extracted_pages": extracted_pages,
                "text": text,
                "truncated": truncated || fetched.truncated,
            }),
            error: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a valid one-page PDF with Helvetica text via `lopdf` itself, so
    /// the bytes always carry a correct xref table.
    fn one_page_pdf(text: &str) -> Vec<u8> {
        use lopdf::{
            content::{Content, Operation},
            dictionary, Document, Object, Stream,
        };
        let mut doc = Document::with_version("1.4");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica"
        });
        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new(
                    "Tf",
                    vec![Object::Name(b"F1".to_vec()), Object::Integer(12)],
                ),
                Operation::new("Td", vec![Object::Integer(50), Object::Integer(150)]),
                Operation::new("Tj", vec![Object::string_literal(text)]),
                Operation::new("ET", vec![]),
            ],
        };
        let content_id = doc.add_object(Stream::new(
            dictionary! {},
            content.encode().expect("encode"),
        ));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 200.into(), 200.into()],
            "Contents" => content_id,
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font_id } }
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page_id.into()],
                "Count" => 1
            }),
        );
        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id
        });
        doc.trailer.set("Root", catalog_id);
        let mut buf = Vec::new();
        doc.save_to(&mut buf).expect("save pdf");
        buf
    }

    #[test]
    fn pages_clamp_to_1_100() {
        assert_eq!(clamp_pages(None), 100);
        assert_eq!(clamp_pages(Some(0)), 1);
        assert_eq!(clamp_pages(Some(-5)), 1);
        assert_eq!(clamp_pages(Some(1000)), 100);
        assert_eq!(clamp_pages(Some(10)), 10);
    }

    #[test]
    fn magic_detector_skips_leading_whitespace() {
        assert!(is_pdf_magic(b"  \n%PDF-1.4 rest"));
        assert!(!is_pdf_magic(b"<html><body>hi"));
        assert!(!is_pdf_magic(b""));
    }

    #[test]
    fn garbage_is_not_a_pdf() {
        let err = extract_pdf_text(b"this is not a pdf".to_vec()).unwrap_err();
        assert!(err.to_string().contains("PDFの解析に失敗"));
    }

    #[test]
    fn minimal_pdf_yields_one_page_of_text() {
        let (text, pages) = extract_pdf_text(one_page_pdf("Hello PDF")).expect("extract");
        assert_eq!(pages, 1, "expected one page, got {pages}");
        assert!(text.contains("Hello PDF"), "unexpected text: {text:?}");
    }

    #[test]
    fn fetching_is_gated() {
        let tool = WebFetchPdfTool::default();
        assert!(tool.is_sensitive(&serde_json::json!({"url": "https://example.com/a.pdf"})));
    }

    #[tokio::test]
    async fn non_http_urls_are_rejected_without_network() {
        let tool = WebFetchPdfTool::default();
        let ctx = ToolContext {
            run_id: uuid::Uuid::new_v4(),
            workspace_id: uuid::Uuid::new_v4(),
            root_path: None,
            additional_paths: Vec::new(),
            approval_token: None,
        };
        let result = tool
            .invoke(ctx, serde_json::json!({"url": "file:///etc/passwd"}))
            .await
            .expect("invoke");
        assert!(!result.ok, "file URLs must not be fetched");
    }
}
