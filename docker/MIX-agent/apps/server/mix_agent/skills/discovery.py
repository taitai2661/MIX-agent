"""Discovery of skills that live in the project folder rather than the database.

A repository keeps its own procedures next to the code they describe, so the
agent should find them without an import step.  The directory names are the
ones other coding agents already use: ``.mix/skills`` first (ours), then
``.claude/skills`` and ``.agents/skills``.

Only the first directory that exists is read.  Merging all three would let a
repository accidentally contribute a skill it did not author, and makes the
effective catalogue depend on which files happen to be present.

Discovery is read-only: packages are offered to the prompt and to
``skill_search`` as they are, never written to the database, so editing a
``SKILL.md`` on disk takes effect on the next run without an import.
"""

from __future__ import annotations

import logging
from pathlib import Path

from mix_agent import config
from mix_agent.skills import package

LOGGER = logging.getLogger(__name__)

SKILL_ROOTS: tuple[str, ...] = (".mix/skills", ".claude/skills", ".agents/skills")


def skill_root(project_root: str | Path | None = None) -> Path | None:
    """The one skill directory that applies to this project, if any."""
    base = Path(project_root) if project_root is not None else config.PROJECT_DIR
    for relative in SKILL_ROOTS:
        candidate = base / relative
        if candidate.is_dir():
            return candidate
    return None


def discover(project_root: str | Path | None = None) -> tuple[list[dict], list[dict]]:
    """Scan the project's skill directory.

    Returns ``(packages, failures)``.  A ``SKILL.md`` that does not parse is
    reported rather than dropped: one malformed file in a repository must not
    silently disappear, and the user needs the reason to fix it.
    """
    root = skill_root(project_root)
    if root is None:
        return [], []
    packages: list[dict] = []
    failures: list[dict] = []
    try:
        entries = sorted(root.iterdir())
    except OSError as exc:
        LOGGER.warning("skill directory %s is unreadable: %s", root, exc)
        return [], []
    for entry in entries:
        skill_md = entry / "SKILL.md"
        if not entry.is_dir() or skill_md.is_symlink() or not skill_md.is_file():
            continue
        try:
            text = skill_md.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError) as exc:
            failures.append({"path": str(skill_md), "reason": str(exc)})
            continue
        try:
            pkg = package.parse_skill_md(text)
        except package.SkillFormatError as exc:
            failures.append({"path": str(skill_md), "reason": str(exc)})
            continue
        pkg["files"] = _bundle_files(skill_md.parent)
        packages.append(pkg)
    return packages, failures


def _bundle_files(skill_dir: Path) -> dict:
    """Bundled files next to a ``SKILL.md``, under the same caps as an import.

    Read-only, symlinks skipped: the mount is user-controlled, but following a
    link out of it would turn a repository into an arbitrary-file reader.
    """
    files: dict = {}
    total = 0
    try:
        candidates = sorted(skill_dir.rglob("*"))
    except OSError:
        return files
    for path in candidates:
        if path.is_symlink() or not path.is_file() or path.name.lower() == "skill.md":
            continue
        try:
            payload = path.read_bytes()
        except OSError:
            continue
        total += len(payload)
        if len(files) >= package.FILES_COUNT_MAX or total > package.FILES_TOTAL_MAX:
            break
        try:
            files[path.relative_to(skill_dir).as_posix()] = package.encode_file(payload)
        except package.SkillFormatError:
            continue
    return files


def find(name: str, project_root: str | Path | None = None) -> dict | None:
    """A discovered package by name, for the ``skill_resource`` tool."""
    packages, _failures = discover(project_root)
    wanted = (name or "").strip().casefold()
    for pkg in packages:
        if str(pkg.get("name", "")).casefold() == wanted:
            return pkg
    return None


def read_resource(pkg: dict, path: str = "") -> dict:
    """List or read a bundled file of a discovered package."""
    files = pkg.get("files") or {}
    if not path:
        return {
            "files": [
                {"path": name, "size": entry.get("size", 0), "binary": bool(entry.get("binary"))}
                for name, entry in sorted(files.items())
            ]
        }
    try:
        safe = package.safe_relative_path(path)
    except package.SkillFormatError:
        raise ValueError("Invalid resource path") from None
    entry = files.get(safe)
    if entry is None:
        raise ValueError("Resource not found")
    return {
        "path": safe,
        "size": entry.get("size", 0),
        "binary": entry.get("binary", False),
        "content": entry.get("content", ""),
    }


def context_view(pkg: dict) -> dict:
    """Shape a discovered package the way the skills block renders a stored one.

    Same projection as :func:`mix_agent.skills.service._context_view` minus the
    database id: bundled files are listed as paths, never inlined, and the id
    is namespaced so it cannot collide with a stored skill's UUID.
    """
    return {
        "id": "project:" + str(pkg.get("name", "")),
        "name": pkg.get("name", ""),
        "description": pkg.get("description", ""),
        "content": pkg.get("content", ""),
        "files": sorted((pkg.get("files") or {}).keys()),
        "source": "project",
    }


def search(query: str = "", project_root: str | Path | None = None) -> list[dict]:
    """Discovered skills matching ``query``, shaped for the prompt.

    Token overlap rather than a substring of the whole query: the prompt passes
    the user's entire message, and "draft the release notes" is not a substring
    of anything while still plainly being about the release-notes skill.
    """
    packages, _failures = discover(project_root)
    terms = [
        term
        for term in "".join(ch if ch.isalnum() else " " for ch in (query or "")).casefold().split()
        if len(term) > 1
    ]
    matched = []
    for pkg in packages:
        haystack = " ".join(
            [str(pkg.get("name", "")), str(pkg.get("description", "")), str(pkg.get("content", ""))]
            + sorted((pkg.get("files") or {}).keys())
        ).casefold()
        if not terms or any(term in haystack for term in terms):
            matched.append(context_view(pkg))
    return matched[:20]
