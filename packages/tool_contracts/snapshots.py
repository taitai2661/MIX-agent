"""Shared snapshot implementation. Only regular files; no archive path extraction."""

import base64
import binascii
import os
from pathlib import Path, PurePosixPath
import shutil
from uuid import uuid4

LIMIT = 64 * 1024 * 1024

# Staging/rollback directories are excluded from capture and never restored
# as payload entries, so both prefixes stay out of the workspace listing.
EXCLUDE_PREFIXES = (".mix-rollback-", ".mix-restore-")


def _decode(encoded):
    try:
        return base64.b64decode(encoded, validate=True)
    except (binascii.Error, ValueError) as exc:
        # Callers map ValueError to HTTP 422; a raw binascii.Error would
        # surface as a 500 and leak decoder details.
        raise ValueError("Invalid snapshot") from exc


def validate_snapshot(payload):
    files = payload.get("files")
    # An empty snapshot is legitimate: a freshly created workspace, or the MCP
    # /shared volume before any server has been installed. `capture` emits
    # {"files": {}} for those, so rejecting it makes their backups unrestorable.
    if not isinstance(files, dict) or len(files) > 10000:
        raise ValueError("Invalid snapshot")
    total = 0
    paths = set(files)
    for name, encoded in files.items():
        path = PurePosixPath(name)
        if (
            path.is_absolute()
            or not path.parts
            or ".." in path.parts
            or "\\" in name
            or str(path) != name
            or any(p.startswith(".mix-") for p in path.parts)
        ):
            raise ValueError("Invalid snapshot path")
        if any(str(parent) in paths for parent in path.parents if str(parent) != "."):
            raise ValueError("File/directory path collision")
        total += len(_decode(encoded))
        if total > LIMIT:
            raise ValueError("Workspace snapshot exceeds 64MB")
    return files


def capture(root):
    files, total = {}, 0
    for current, directories, names in os.walk(root, followlinks=False):
        if any(Path(current, d).is_symlink() for d in directories):
            raise ValueError("Remove symlinks before backup")
        directories[:] = [
            d for d in directories if not Path(current, d).is_symlink() and not d.startswith(EXCLUDE_PREFIXES)
        ]
        for name in names:
            file = Path(current, name)
            if file.is_symlink() or not file.is_file():
                raise ValueError("Remove symlinks and special files before backup")
            data = file.read_bytes()
            total += len(data)
            if total > LIMIT:
                raise ValueError("Workspace snapshot exceeds 64MB")
            files[str(file.relative_to(root))] = base64.b64encode(data).decode()
    return {"files": files}


def restore(root, payload):
    files = validate_snapshot(payload)
    staging = root / (".mix-restore-" + str(uuid4()))
    rollback = root / (".mix-rollback-" + str(uuid4()))
    try:
        staging.mkdir()
        rollback.mkdir()
        for name, content in files.items():
            file = staging / name
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_bytes(_decode(content))
        original = [
            p
            for p in root.iterdir()
            if p not in (staging, rollback) and not p.name.startswith(EXCLUDE_PREFIXES)
        ]
        moved = []
        installed = []
        for file in original:
            file.rename(rollback / file.name)
            moved.append(file.name)
        for file in list(staging.iterdir()):
            file.rename(root / file.name)
            installed.append(file.name)
        staging.rmdir()
        shutil.rmtree(rollback)
    except BaseException:
        for name in locals().get("installed", []):
            target = root / name
            if target.is_dir() and not target.is_symlink():
                shutil.rmtree(target)
            else:
                target.unlink(missing_ok=True)
        if rollback.exists():
            for file in rollback.iterdir():
                target = root / file.name
                if target.exists():
                    if target.is_dir() and not target.is_symlink():
                        shutil.rmtree(target)
                    else:
                        target.unlink()
                file.rename(target)
        raise
    return {"ok": True}
