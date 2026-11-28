//! Vision Tool: validate image attachments and surface a base64 preview
//! suitable for attaching to a multimodal Provider request.
//!
//! The frontend already caps picks at 8 MiB; this is the second gate before a
//! model call. It checks the claimed MIME type against an allow-list, sniffs
//! the real container from magic bytes (a mismatched `mime_type` would send a
//! confusing payload to the provider), decodes bounds-checked base64, and
//! extracts width/height from the container header so the engine can budget
//! context without pulling an image crate into the tool tree.

use async_trait::async_trait;
use base64::Engine;

use super::{Tool, ToolContext, ToolResult};

#[derive(Default)]
pub struct ImageVisionTool;

const MAX_BYTES: usize = 20 * 1024 * 1024;

/// MIME types the vision path accepts. SVG is vector markup rather than a
/// raster image most vision endpoints accept, so it is rejected here instead
/// of failing opaquely at the provider.
const ALLOWED_MIME: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/bmp",
];

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
        let mime = mime.trim();
        if mime.is_empty() {
            anyhow::bail!("mime_type is required");
        }
        if !ALLOWED_MIME.contains(&mime) {
            anyhow::bail!(
                "unsupported image type {mime}; expected one of: {}",
                ALLOWED_MIME.join(", ")
            );
        }
        if data.is_empty() {
            anyhow::bail!("data_base64 is empty");
        }
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
        let sniffed = sniff_format(&decoded);
        if let Some(actual) = sniffed {
            // JPEG is the only container with two spellings here (`image/jpeg`
            // on the wire vs `jpg` in most sniffers); normalise before compare.
            let claimed = mime.strip_prefix("image/").unwrap_or(mime);
            let claimed = if claimed == "jpeg" { "jpg" } else { claimed };
            if actual != claimed {
                anyhow::bail!(
                    "mime_type {mime} does not match image contents (looks like image/{actual})"
                );
            }
        } else if decoded.len() >= 8 {
            // Long enough to sniff but matched nothing: not a raster image the
            // vision path can hand to a provider.
            anyhow::bail!("bytes are not a recognised raster image (png/jpeg/gif/webp/bmp)");
        } else {
            anyhow::bail!("image data is too short to be a valid image");
        }
        let (width, height) = dimensions(&decoded);
        // `data` is model-controlled and only guaranteed to be UTF-8: slicing
        // it by byte length panicked whenever byte 64 fell inside a multi-byte
        // character. Taking characters cannot split one, and the preview is
        // informational rather than a decodable data URL either way.
        let preview_prefix: String = data.chars().take(64).collect();
        Ok(ToolResult::ok(serde_json::json!({
            "mime_type": mime,
            "bytes": decoded.len(),
            "width": width,
            "height": height,
            "preview": format!("data:{};base64,{}", mime, preview_prefix),
        })))
    }
}

/// Container sniffed from magic bytes. Returns the `image/<this>` suffix.
fn sniff_format(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() >= 8
        && bytes[0] == 0x89
        && bytes[1] == b'P'
        && bytes[2] == b'N'
        && bytes[3] == b'G'
        && bytes[4] == 0x0D
        && bytes[5] == 0x0A
        && bytes[6] == 0x1A
        && bytes[7] == 0x0A
    {
        return Some("png");
    }
    if bytes.len() >= 3 && bytes[0] == 0xFF && bytes[1] == 0xD8 && bytes[2] == 0xFF {
        return Some("jpg");
    }
    if bytes.len() >= 6
        && bytes[0] == b'G'
        && bytes[1] == b'I'
        && bytes[2] == b'F'
        && bytes[3] == b'8'
        && (bytes[4] == b'7' || bytes[4] == b'9')
        && bytes[5] == b'a'
    {
        return Some("gif");
    }
    if bytes.len() >= 12
        && bytes[0] == b'R'
        && bytes[1] == b'I'
        && bytes[2] == b'F'
        && bytes[3] == b'F'
        && bytes[8] == b'W'
        && bytes[9] == b'E'
        && bytes[10] == b'B'
        && bytes[11] == b'P'
    {
        return Some("webp");
    }
    if bytes.len() >= 2 && bytes[0] == b'B' && bytes[1] == b'M' {
        return Some("bmp");
    }
    None
}

fn dimensions(bytes: &[u8]) -> (Option<u32>, Option<u32>) {
    match sniff_format(bytes) {
        Some("png") => png_dimensions(bytes),
        Some("jpg") => jpeg_dimensions(bytes),
        Some("gif") => {
            if bytes.len() >= 10 {
                let w = u16::from_le_bytes([bytes[6], bytes[7]]) as u32;
                let h = u16::from_le_bytes([bytes[8], bytes[9]]) as u32;
                (Some(w), Some(h))
            } else {
                (None, None)
            }
        }
        Some("bmp") => {
            if bytes.len() >= 26 {
                let w = i32::from_le_bytes([bytes[18], bytes[19], bytes[20], bytes[21]]);
                let h = i32::from_le_bytes([bytes[22], bytes[23], bytes[24], bytes[25]]);
                // `i32::MIN.abs()` overflows (panic in debug builds). A
                // crafted BMP height must not take down the tool thread.
                (u32::try_from(w).ok(), Some(h.unsigned_abs()))
            } else {
                (None, None)
            }
        }
        Some("webp") => webp_dimensions(bytes),
        _ => (None, None),
    }
}

fn png_dimensions(bytes: &[u8]) -> (Option<u32>, Option<u32>) {
    // Signature (8) + IHDR length (4) + "IHDR" (4) + width (4) + height (4).
    if bytes.len() >= 24 && &bytes[12..16] == b"IHDR" {
        let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        (Some(w), Some(h))
    } else {
        (None, None)
    }
}

fn jpeg_dimensions(bytes: &[u8]) -> (Option<u32>, Option<u32>) {
    // Walk markers until a Start-Of-Frame carrying w/h is found.
    let mut i = 2usize;
    while i + 4 <= bytes.len() {
        if bytes[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = bytes[i + 1];
        // Standalone markers carry no length.
        if marker == 0xD8 || marker == 0xD9 || (0xD0..=0xD7).contains(&marker) || marker == 0x01 {
            i += 2;
            continue;
        }
        if i + 4 > bytes.len() {
            break;
        }
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        if len < 2 || i + 2 + len > bytes.len() {
            break;
        }
        // SOF0–SOF3, SOF5–SOF15 except DHT/JPG/DAC (0xC4/0xC8/0xCC).
        let is_sof =
            (0xC0..=0xCF).contains(&marker) && marker != 0xC4 && marker != 0xC8 && marker != 0xCC;
        if is_sof && len >= 7 {
            let h = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
            let w = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]) as u32;
            if w > 0 && h > 0 {
                return (Some(w), Some(h));
            }
            return (None, None);
        }
        i += 2 + len;
    }
    (None, None)
}

fn webp_dimensions(bytes: &[u8]) -> (Option<u32>, Option<u32>) {
    if bytes.len() < 16 {
        return (None, None);
    }
    match &bytes[12..16] {
        // Lossy VP8: frame tag (3) + start code (3) + w-1 (2 LE) + h-1 (2 LE).
        b"VP8 " if bytes.len() >= 30 => {
            if bytes[23] != 0x9D || bytes[24] != 0x01 || bytes[25] != 0x2A {
                return (None, None);
            }
            let w = u16::from_le_bytes([bytes[26], bytes[27]]) as u32 & 0x3FFF;
            let h = u16::from_le_bytes([bytes[28], bytes[29]]) as u32 & 0x3FFF;
            (Some(w), Some(h))
        }
        // Lossless VP8L: signature byte + 4 packed bytes (w-1: 14 bit, h-1: 14 bit).
        b"VP8L" if bytes.len() >= 25 => {
            if bytes[20] != 0x2F {
                return (None, None);
            }
            let b1 = bytes[21] as u32;
            let b2 = bytes[22] as u32;
            let b3 = bytes[23] as u32;
            let b4 = bytes[24] as u32;
            let w = ((b2 & 0x3F) << 8) | b1;
            let h = ((b4 & 0x0F) << 10) | (b3 << 2) | ((b2 >> 6) & 0x03);
            (Some(w + 1), Some(h + 1))
        }
        // Extended VP8X: canvas size minus one, 24-bit big-endian each.
        b"VP8X" if bytes.len() >= 30 => {
            let w = ((bytes[24] as u32) << 16) | ((bytes[25] as u32) << 8) | bytes[26] as u32;
            let h = ((bytes[27] as u32) << 16) | ((bytes[28] as u32) << 8) | bytes[29] as u32;
            (Some(w + 1), Some(h + 1))
        }
        _ => (None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal 1x1 PNG (signature + IHDR with w=1 h=1 + IEND).
    fn png_1x1() -> Vec<u8> {
        vec![
            0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, // signature
            0x00, 0x00, 0x00, 0x0D, // IHDR length
            b'I', b'H', b'D', b'R', // IHDR
            0x00, 0x00, 0x00, 0x01, // width 1
            0x00, 0x00, 0x00, 0x01, // height 1
            0x08, 0x02, 0x00, 0x00, 0x00, // bit depth etc.
            0x90, 0x77, 0x53, 0xDE, // crc (arbitrary)
            0x00, 0x00, 0x00, 0x00, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82,
        ]
    }

    #[tokio::test]
    async fn accepts_png_with_dimensions() {
        let tool = ImageVisionTool;
        let bytes = png_1x1();
        let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
        let ctx = ToolContext {
            run_id: uuid::Uuid::new_v4(),
            workspace_id: uuid::Uuid::new_v4(),
            root_path: None,
            additional_paths: Vec::new(),
            approval_token: None,
        };
        let out = tool
            .invoke(
                ctx,
                serde_json::json!({ "mime_type": "image/png", "data_base64": data }),
            )
            .await
            .expect("invoke");
        assert!(out.ok);
        assert_eq!(out.content["bytes"], bytes.len() as u64);
        assert_eq!(out.content["width"], 1);
        assert_eq!(out.content["height"], 1);
    }

    #[tokio::test]
    async fn rejects_mismatched_mime() {
        let tool = ImageVisionTool;
        let data = base64::engine::general_purpose::STANDARD.encode(png_1x1());
        let ctx = ToolContext {
            run_id: uuid::Uuid::new_v4(),
            workspace_id: uuid::Uuid::new_v4(),
            root_path: None,
            additional_paths: Vec::new(),
            approval_token: None,
        };
        let err = tool
            .invoke(
                ctx,
                serde_json::json!({ "mime_type": "image/jpeg", "data_base64": data }),
            )
            .await
            .expect_err("mismatch must fail");
        assert!(err.to_string().contains("does not match"), "got: {err}");
    }

    #[tokio::test]
    async fn rejects_svg_and_empty() {
        let tool = ImageVisionTool;
        let ctx = || ToolContext {
            run_id: uuid::Uuid::new_v4(),
            workspace_id: uuid::Uuid::new_v4(),
            root_path: None,
            additional_paths: Vec::new(),
            approval_token: None,
        };
        let err = tool
            .invoke(
                ctx(),
                serde_json::json!({ "mime_type": "image/svg+xml", "data_base64": "PHN2Zz4=" }),
            )
            .await
            .expect_err("svg must fail");
        assert!(err.to_string().contains("unsupported"), "got: {err}");
        let err = tool
            .invoke(
                ctx(),
                serde_json::json!({ "mime_type": "image/png", "data_base64": "" }),
            )
            .await
            .expect_err("empty must fail");
        assert!(err.to_string().contains("empty"), "got: {err}");
    }
}
