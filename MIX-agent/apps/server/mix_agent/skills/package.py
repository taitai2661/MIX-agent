"""Agent Skills (``SKILL.md``) parsing, validation, and packaging.

A skill is a directory containing ``SKILL.md`` with YAML frontmatter followed
by Markdown.  This module only validates and stores packages; it never executes
bundled scripts, matching the project's untrusted-content boundary.
"""

from __future__ import annotations

import base64
import re
import zipfile
from pathlib import Path, PurePosixPath

import yaml

NAME_RE = re.compile(r"^[a-z0-9]+(?:-[a-z0-9]+)*$")
NAME_MAX = 64
DESCRIPTION_MAX = 1024
COMPATIBILITY_MAX = 500
CONTENT_MAX = 50000
FILE_MAX = 262144
FILES_TOTAL_MAX = 2 * 1024 * 1024
FILES_COUNT_MAX = 100
ARCHIVE_MAX = 5 * 1024 * 1024

_FRONTMATTER = re.compile(r"^---[ \t]*\r?\n(.*?)\r?\n---[ \t]*(?:\r?\n(.*))?$", re.DOTALL)


class SkillFormatError(ValueError):
    pass


def parse_skill_md(text: str) -> dict:
    """Parse a ``SKILL.md`` document into normalized metadata and a body."""
    if not isinstance(text, str):
        raise SkillFormatError("SKILL.md must be text")
    match = _FRONTMATTER.match(text.lstrip("\ufeff").strip())
    if not match:
        raise SkillFormatError("SKILL.md must start with YAML frontmatter delimited by ---")
    try:
        meta = yaml.safe_load(match.group(1)) or {}
    except yaml.YAMLError as exc:
        raise SkillFormatError(f"invalid YAML frontmatter: {exc}") from exc
    if not isinstance(meta, dict):
        raise SkillFormatError("frontmatter must be a YAML mapping")
    errors = validate_meta(meta)
    if errors:
        raise SkillFormatError("; ".join(errors))
    content = (match.group(2) or "").strip()
    if len(content) > CONTENT_MAX:
        raise SkillFormatError(f"SKILL.md body must be at most {CONTENT_MAX} characters")
    return {**_normalize(meta), "content": content, "files": {}}


def validate_meta(meta: dict) -> list[str]:
    errors: list[str] = []
    name = meta.get("name")
    if not isinstance(name, str) or not name.strip():
        errors.append("name is required")
    elif len(name) > NAME_MAX or not NAME_RE.match(name):
        errors.append("name must be lowercase letters, numbers, and hyphens (max 64 chars)")
    description = meta.get("description")
    if not isinstance(description, str) or not description.strip():
        errors.append("description is required")
    elif len(description) > DESCRIPTION_MAX:
        errors.append("description must be at most 1024 characters")
    compatibility = meta.get("compatibility")
    if compatibility is not None and (
        not isinstance(compatibility, str) or len(compatibility) > COMPATIBILITY_MAX
    ):
        errors.append("compatibility must be a string of at most 500 characters")
    if meta.get("metadata") is not None and not isinstance(meta["metadata"], dict):
        errors.append("metadata must be a mapping")
    allowed = meta.get("allowed-tools")
    if allowed is not None and not isinstance(allowed, str):
        errors.append("allowed-tools must be a space-separated string")
    return errors


def _normalize(meta: dict) -> dict:
    metadata = meta.get("metadata") or {}
    allowed = meta.get("allowed-tools") or ""
    return {
        "name": str(meta["name"]).strip(),
        "description": str(meta["description"]).strip(),
        "license": str(meta.get("license") or "").strip(),
        "compatibility": str(meta.get("compatibility") or "").strip(),
        "metadata": {str(key): str(value) for key, value in metadata.items()},
        "allowed_tools": allowed.split() if isinstance(allowed, str) else [],
    }


def render_skill_md(skill: dict) -> str:
    """Render a stored skill back into the Agent Skills format."""
    meta: dict = {
        "name": skill.get("name", ""),
        "description": skill.get("description", ""),
    }
    if skill.get("license"):
        meta["license"] = skill["license"]
    if skill.get("compatibility"):
        meta["compatibility"] = skill["compatibility"]
    if skill.get("metadata"):
        meta["metadata"] = skill["metadata"]
    if skill.get("allowed_tools"):
        meta["allowed-tools"] = " ".join(str(tool) for tool in skill["allowed_tools"])
    dumped = yaml.safe_dump(meta, allow_unicode=True, sort_keys=False).strip()
    body = str(skill.get("content") or "").strip()
    return f"---\n{dumped}\n---\n\n{body}\n"


def encode_file(raw: bytes) -> dict:
    if len(raw) > FILE_MAX:
        raise SkillFormatError(f"bundled file exceeds {FILE_MAX} bytes")
    try:
        return {"content": raw.decode("utf-8"), "size": len(raw), "binary": False}
    except UnicodeDecodeError:
        return {"content": base64.b64encode(raw).decode("ascii"), "size": len(raw), "binary": True}


def safe_relative_path(name: str) -> str:
    path = PurePosixPath(name.replace("\\", "/"))
    if path.is_absolute() or not path.parts or ".." in path.parts:
        raise SkillFormatError(f"unsafe archive path: {name}")
    return str(path)


def parse_archive(raw: bytes) -> list[dict]:
    """Parse a zip archive into one or more skill packages."""
    if len(raw) > ARCHIVE_MAX:
        raise SkillFormatError(f"archive exceeds {ARCHIVE_MAX} bytes")
    try:
        bundle = zipfile.ZipFile(_bytes_reader(raw))
    except zipfile.BadZipFile as exc:
        raise SkillFormatError("archive is not a valid zip file") from exc
    entries: dict[str, bytes] = {}
    with bundle:
        for info in bundle.infolist():
            if info.is_dir():
                continue
            name = safe_relative_path(info.filename)
            if len(entries) >= FILES_COUNT_MAX + 20:
                raise SkillFormatError("archive contains too many files")
            entries[name] = bundle.read(info)

    roots = sorted(
        {str(PurePosixPath(name).parent) for name in entries if PurePosixPath(name).name.lower() == "skill.md"},
        key=lambda value: value.count("/"),
    )
    if not roots:
        raise SkillFormatError("archive does not contain a SKILL.md")

    selected: list[str] = []
    for root in roots:
        prefix = "" if root in (".", "") else root + "/"
        if any(root != other and root.startswith(other + "/") for other in selected):
            continue
        selected.append(root if root != "." else "")

    packages = []
    for root in selected:
        prefix = "" if root == "" else root + "/"
        skill_md = entries.get(prefix + "SKILL.md") or entries.get(prefix + "skill.md")
        if skill_md is None:
            continue
        package = parse_skill_md(skill_md.decode("utf-8", errors="replace"))
        files: dict[str, dict] = {}
        total = 0
        for name, payload in entries.items():
            if not name.startswith(prefix) or name == prefix + "SKILL.md":
                continue
            relative = name[len(prefix):]
            if relative.lower() == "skill.md":
                continue
            total += len(payload)
            if total > FILES_TOTAL_MAX:
                raise SkillFormatError("bundled files exceed the total size limit")
            files[relative] = encode_file(payload)
        package["files"] = files
        packages.append(package)
    if not packages:
        raise SkillFormatError("archive does not contain a usable SKILL.md")
    return packages


def scan_directory(root: str | Path) -> list[dict]:
    """Load every ``*/SKILL.md`` directly under ``root`` (read-only)."""
    packages, _failures = scan_directory_report(root)
    return packages


def scan_directory_report(root: str | Path) -> tuple[list[dict], list[dict]]:
    """Like :func:`scan_directory`, but also report the entries it skipped.

    Silently dropping a malformed ``SKILL.md`` leaves the user staring at an
    import that reports success while nothing appeared; the failures come back
    as ``{"path", "reason"}`` so the caller can show them.
    """
    base = Path(root)
    if not base.is_dir():
        return [], []
    packages = []
    failures = []
    for skill_md in sorted(base.glob("*/SKILL.md")):
        if skill_md.is_symlink() or not skill_md.is_file():
            continue
        try:
            package = parse_skill_md(skill_md.read_text(encoding="utf-8"))
        except (OSError, SkillFormatError) as exc:
            failures.append({"path": str(skill_md), "reason": str(exc)})
            continue
        except UnicodeDecodeError as exc:
            failures.append({"path": str(skill_md), "reason": f"SKILL.md is not UTF-8: {exc}"})
            continue
        files: dict[str, dict] = {}
        total = 0
        for path in sorted(skill_md.parent.rglob("*")):
            if path.is_symlink() or not path.is_file() or path == skill_md:
                continue
            try:
                payload = path.read_bytes()
            except OSError:
                continue
            total += len(payload)
            if len(files) >= FILES_COUNT_MAX or total > FILES_TOTAL_MAX:
                break
            try:
                files[path.relative_to(skill_md.parent).as_posix()] = encode_file(payload)
            except SkillFormatError:
                continue
        package["files"] = files
        packages.append(package)
    return packages, failures


def bundle_archive(skill: dict) -> bytes:
    """Build a zip archive containing ``SKILL.md`` and bundled files."""
    buffer = _bytes_writer()
    with zipfile.ZipFile(buffer, "w", zipfile.ZIP_DEFLATED) as archive:
        archive.writestr("SKILL.md", render_skill_md(skill))
        for name, entry in (skill.get("files") or {}).items():
            raw = (
                base64.b64decode(entry.get("content", ""))
                if entry.get("binary")
                else str(entry.get("content", "")).encode("utf-8")
            )
            archive.writestr(name, raw)
    return buffer.getvalue()


def _bytes_reader(raw: bytes):
    import io

    return io.BytesIO(raw)


def _bytes_writer():
    import io

    return io.BytesIO()
