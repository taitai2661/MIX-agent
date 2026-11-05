"""Discovery of skills that live in the project folder or the user's home.

A repository keeps its own procedures next to the code they describe, so the
agent should find them without an import step.  The directory names are the
ones other coding agents already use: ``.mix/skills`` first (ours), then
``.claude/skills`` and ``.agents/skills``.

Within one scope only the first existing directory is read.  Merging all three
would let a repository accidentally contribute a skill it did not author, and
makes the effective catalogue depend on which files happen to be present.
Across scopes the order is project → user → configured skills directory, so a
repository can override a personal skill of the same name.

Discovery is read-only: packages are offered to the prompt and to the
``skill_search`` / ``skill_resource`` tools as they are, never written to the
database, so editing a ``SKILL.md`` on disk takes effect on the next run
without an import.
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
    return _first_root(base)


def user_skill_root(home: str | Path | None = None) -> Path | None:
    """The one user-level skill directory that applies, if any."""
    base = Path(home) if home is not None else Path.home()
    return _first_root(base)


def _first_root(base: Path) -> Path | None:
    for relative in SKILL_ROOTS:
        candidate = base / relative
        if candidate.is_dir():
            return candidate
    return None


def _user_roots(home: str | Path | None = None) -> list[Path]:
    """Every user-scoped source that should contribute, in precedence order.

    ``MIX_SKILLS_DIR`` is included for self-hosted deployments where skills are
    mounted rather than imported.  It is a separate directory, not one of the
    dot-directory conventions, so it contributes in addition to the first
    existing home directory.
    """
    roots: list[Path] = []
    first = user_skill_root(home)
    if first is not None:
        roots.append(first)
    mounted = Path(config.SKILLS_DIR)
    if mounted.is_dir() and mounted not in roots:
        roots.append(mounted)
    return roots


def discover(project_root: str | Path | None = None) -> tuple[list[dict], list[dict]]:
    """Scan the project's skill directory.

    Returns ``(packages, failures)``.  A ``SKILL.md`` that does not parse is
    reported rather than dropped: one malformed file in a repository must not
    silently disappear, and the user needs the reason to fix it.
    """
    root = skill_root(project_root)
    if root is None:
        return [], []
    return _discover_root(root)


def discover_all(
    project_root: str | Path | None = None,
    home: str | Path | None = None,
) -> tuple[list[dict], list[dict]]:
    """Project skills, then user-level skills, first name winning."""
    packages: list[dict] = []
    failures: list[dict] = []
    roots = []
    project = skill_root(project_root)
    if project is not None:
        roots.append(project)
    for root in _user_roots(home):
        if root not in roots:
            roots.append(root)

    seen: set[str] = set()
    for index, root in enumerate(roots):
        batch, failed = _discover_root(root, "project" if index == 0 and project == root else "user")
        failures.extend(failed)
        for pkg in batch:
            name = str(pkg.get("name", "")).casefold()
            if name in seen:
                LOGGER.warning("skill %r is supplied by more than one root; keeping the first", name)
                continue
            seen.add(name)
            packages.append(pkg)
    return packages, failures


def _discover_root(root: Path, source: str = "project") -> tuple[list[dict], list[dict]]:
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
        if pkg["name"] != entry.name:
            # The specification requires the name to match the parent folder.
            reason = f"skill name {pkg['name']!r} must match its directory name {entry.name!r}"
            LOGGER.warning("%s: %s", skill_md, reason)
            failures.append({"path": str(skill_md), "reason": reason})
            continue
        pkg["files"] = _bundle_manifest(skill_md.parent)
        pkg["root"] = str(skill_md.parent)
        pkg["source"] = source
        packages.append(pkg)
    return packages, failures


def _bundle_manifest(skill_dir: Path) -> dict:
    """Bundled file metadata next to a ``SKILL.md``, under the import caps.

    Discovery runs on every Run (prompt construction) and only needs the paths,
    so reading and base64-encoding every file here made a large skill expensive
    per turn. Contents are read lazily by :func:`read_resource`.
    """
    files: dict = {}
    total = 0
    for path in package.iter_skill_files(skill_dir):
        if path.name.lower() == "skill.md":
            continue
        try:
            size = path.stat().st_size
        except OSError:
            continue
        if size > package.FILE_MAX:
            LOGGER.warning("%s: bundled file exceeds the size limit; skipped", path)
            continue
        total += size
        if len(files) >= package.FILES_COUNT_MAX or total > package.FILES_TOTAL_MAX:
            LOGGER.warning("%s: bundled files exceed the package limit", skill_dir)
            break
        files[path.relative_to(skill_dir).as_posix()] = {"size": size, "binary": False}
    return files


def find(name: str, project_root: str | Path | None = None, home: str | Path | None = None) -> dict | None:
    """A discovered package by name, for the ``skill_resource`` tool."""
    packages, _failures = discover_all(project_root, home)
    wanted = (name or "").strip().casefold()
    for pkg in packages:
        if str(pkg.get("name", "")).casefold() == wanted:
            return pkg
    return None


def read_resource(pkg: dict, path: str = "") -> dict:
    """List or read a bundled file of a discovered package.

    The package itself only carries paths/sizes; the file is read here so the
    prompt/catalogue path never pays for every byte of every skill.
    """
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
    if safe not in files:
        raise ValueError("Resource not found")
    root_text = str(pkg.get("root") or "")
    if not root_text:
        raise ValueError("Resource not found")
    root = Path(root_text)
    if not root.is_dir():
        raise ValueError("Resource not found")
    full = root / safe
    if full.is_symlink() or not full.is_file():
        raise ValueError("Resource not found")
    try:
        resolved = full.resolve()
        if not resolved.is_relative_to(root.resolve()):
            raise ValueError("Resource not found")
        raw = resolved.read_bytes()
    except OSError:
        raise ValueError("Resource not found") from None
    try:
        entry = package.encode_file(raw)
    except package.SkillFormatError as exc:
        raise ValueError(str(exc)) from None
    return {"path": safe, **entry}


def context_view(pkg: dict) -> dict:
    """Shape a discovered package the way the skills block renders a stored one.

    Same projection as :func:`mix_agent.skills.service._context_view` minus the
    database id: bundled files are listed as paths, never inlined, and the id
    is namespaced so it cannot collide with a stored skill's UUID.
    """
    source = pkg.get("source", "project")
    return {
        "id": f"{source}:" + str(pkg.get("name", "")),
        "name": pkg.get("name", ""),
        "description": pkg.get("description", ""),
        "content": pkg.get("content", ""),
        "files": sorted((pkg.get("files") or {}).keys()),
        "source": source,
    }


def search(
    query: str = "",
    project_root: str | Path | None = None,
    home: str | Path | None = None,
) -> list[dict]:
    """Discovered skills matching ``query``, shaped for the prompt.

    Token overlap rather than a substring of the whole query: the prompt passes
    the user's entire message, and "draft the release notes" is not a substring
    of anything while still plainly being about the release-notes skill.
    """
    packages, _failures = discover_all(project_root, home)
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
