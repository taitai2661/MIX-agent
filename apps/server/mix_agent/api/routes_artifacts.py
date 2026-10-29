"""Artifact upload and download endpoints.

Kept separate from ``routes.py`` because they are the only endpoints with a
file body and a file response; both need the upload-name sanitiser and the
attachment-only ``Content-Disposition`` policy in one place.
"""

from fastapi import APIRouter, Depends, File, HTTPException, UploadFile
from fastapi.responses import FileResponse

from mix_agent import config
from mix_agent.api.common import context, own
from mix_agent.api.schemas import ArtifactView
from mix_agent.db.models import Artifact
from mix_agent.tools.execute import safe_upload_name, save_artifact

router = APIRouter(prefix="/api/v1")


@router.post("/artifacts", response_model=ArtifactView)
async def upload(file: UploadFile = File(...), ctx=Depends(context)):
    db, s = ctx
    content = await file.read(config.MAX_UPLOAD + 1)
    if len(content) > config.MAX_UPLOAD:
        raise HTTPException(413, "添付は20MBまでです")
    mime = file.content_type or "text/plain"
    if mime not in (
        "image/png",
        "image/jpeg",
        "image/webp",
        "image/gif",
        "application/pdf",
        "text/plain",
        "text/markdown",
    ):
        raise HTTPException(422, "対応形式: 画像・PDF・テキスト・Markdown")
    result = save_artifact(db, s.owner_id, content, safe_upload_name(file.filename), mime)
    db.commit()
    return result


@router.get("/artifacts/{key}")
def download(key: str, ctx=Depends(context)):
    db, s = ctx
    row = own(db, Artifact, key, s.owner_id)
    name = row.data["name"]
    # Always serve as an attachment: model-created text/html or SVG artifacts
    # must never render as active documents in this origin.  Let Starlette own
    # `Content-Disposition` (`filename=` encodes non-ASCII names as RFC 5987
    # `filename*`); hand-building the header broke on names it could not encode.
    return FileResponse(
        config.ARTIFACTS / key,
        media_type=row.data["mime"],
        filename=name,
        content_disposition_type="attachment",
        headers={
            "X-Content-Type-Options": "nosniff",
            "X-Download-Options": "noopen",
        },
    )
