"""Reference-ification of tool outputs and images.

Long tool results and base64 images are stored as Artifacts and replaced by
small pointer envelopes in history. Resolution back to provider format happens
only at send time, with graceful degradation when a file is missing.
"""

from __future__ import annotations

import base64
import hashlib
import json

IMAGE_REF_PREFIX = "image_ref://"
TOOL_REF_MARKER = "artifact_ref"

# Heuristic: inline short tool results, reference-ify the rest (configurable).
DEFAULT_TOOL_INLINE_LIMIT = 4000


def is_base64_image(value: str) -> bool:
    text = str(value or "")
    return text.startswith("data:image/") and ";base64," in text


def image_ref_id(data_url: str) -> str:
    digest = hashlib.sha256(data_url.encode("utf-8")).hexdigest()[:16]
    return f"{IMAGE_REF_PREFIX}{digest}"


def extract_data_url_images(message: dict) -> tuple[dict, list[dict]]:
    """Replace inline data-URL images with image_ref pointers.

    Returns (message_without_inline_images, extracted[{ref_id, mime, bytes}]).
    """
    images = message.get("images") or []
    if not images:
        return message, []
    kept, extracted = [], []
    for item in images:
        if not is_base64_image(item):
            kept.append(item)
            continue
        header, _, payload = item.partition(",")
        mime = header.split(";")[0].split(":")[-1] or "image/png"
        try:
            raw = base64.b64decode(payload, validate=False)
        except Exception:  # noqa: BLE001, S112 - corrupt attachment data degrades to text
            continue
        ref_id = image_ref_id(item)
        extracted.append({"ref_id": ref_id, "mime": mime, "bytes": raw})
    updated = {**message, "images": kept}
    refs = message.get("image_refs") or []
    updated["image_refs"] = [*refs, *[e["ref_id"] for e in extracted]]
    return updated, extracted


def tool_envelope_text(content: str, inline_limit: int = DEFAULT_TOOL_INLINE_LIMIT) -> tuple[bool, str]:
    """Decide whether a tool payload must be reference-ified. Returns (is_long, summary)."""
    text = str(content or "")
    if len(text) <= inline_limit:
        return False, text
    try:
        payload = json.loads(text)
        if isinstance(payload, dict):
            result = payload.get("result", payload)
            summary_src = json.dumps(result, ensure_ascii=False)
        else:
            summary_src = text
    except (TypeError, ValueError):
        summary_src = text
    return True, summary_src[:2000]


def tool_ref_message(call_id: str, name: str, summary: str, artifact_id: str | None, truncated: bool) -> dict:
    """Small history entry preserving tool protocol fields (call_id/name)."""
    envelope: dict = {
        "status": "succeeded",
        "tool": name,
        "summary": summary,
        TOOL_REF_MARKER: f"artifact://{artifact_id}" if artifact_id else None,
        "truncated": bool(truncated),
    }
    return {
        "role": "tool",
        "call_id": call_id,
        "name": name,
        "content": json.dumps(envelope, ensure_ascii=False),
        "tool_ref": artifact_id,
    }


def resolve_message_images(message: dict, loader) -> dict:
    """Resolve image_refs to provider data-URLs at send time; degrade on miss."""
    refs = message.get("image_refs") or []
    if not refs:
        return message
    images = list(message.get("images") or [])
    missing = []
    for ref_id in refs:
        try:
            data_url = loader(ref_id)
        except Exception:  # noqa: BLE001 - missing refs degrade, never crash the run
            data_url = None
        if data_url:
            images.append(data_url)
        else:
            missing.append(ref_id)
    resolved = {**message, "images": images}
    if missing:
        resolved["missing_image_refs"] = missing
    return resolved
