from fastapi import APIRouter, Depends, HTTPException, UploadFile, File, Form
from fastapi.responses import Response
from mix_agent.api.routes import context
from mix_agent.api.schemas import BackupInput
from mix_agent.storage import backup

router = APIRouter(prefix="/api/v1")


@router.post("/backups")
async def create(body: BackupInput, ctx=Depends(context)):
    if backup.LOCK.locked():
        raise HTTPException(409, "バックアップ処理中です")
    async with backup.LOCK:
        backup.ACTIVE = True
        try:
            raw = await backup.create(ctx[0], body.passphrase)
            return Response(
                raw,
                media_type="application/octet-stream",
                headers={"Content-Disposition": 'attachment; filename="mix-agent-backup.mix"'},
            )
        except Exception:
            raise HTTPException(
                422,
                "バックアップできません。実行・バックグラウンドProcessを停止し、Runner接続とサイズ上限を確認してください。",
            )
        finally:
            backup.ACTIVE = (backup.config.DATA / "restore-journal.json").exists()


@router.post("/backups/restore")
async def restore(
    file: UploadFile = File(...),
    passphrase: str = Form(..., min_length=12, max_length=256),
    ctx=Depends(context),
):
    raw = await file.read(backup.MAX_SIZE + 1)
    if backup.LOCK.locked():
        raise HTTPException(409)
    async with backup.LOCK:
        backup.ACTIVE = True
        try:
            return await backup.restore(ctx[0], raw, passphrase)
        except Exception:
            raise HTTPException(
                422,
                "復元に失敗しました。パスフレーズ、形式、Runner接続を確認してください。退避データは保持されます。",
            )
        finally:
            backup.ACTIVE = (backup.config.DATA / "restore-journal.json").exists()
