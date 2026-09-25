import io
import zipfile

import pytest
from sqlalchemy import select

from mix_agent import config
from mix_agent.db.models import User
from mix_agent.db.session import SessionLocal
from mix_agent.skills import service as skill_service

SKILL_MD = """---
name: release-runbook
description: Verify and release the app. Use when asked to cut a release.
license: MIT
allowed-tools: read_file
---

# Release runbook

1. Run tests.
2. Build.
"""


def test_import_from_text_and_export(signed):
    row = signed.post("/api/v1/skills/import", json={"text": SKILL_MD})
    assert row.status_code == 200, row.text
    data = row.json()["data"]
    key = row.json()["id"]
    assert data["format"] == "agent-skill"
    assert data["source_slug"] == "release-runbook"
    assert data["allowed_tools"] == ["read_file"]
    assert data["content"].startswith("# Release runbook")

    export = signed.get("/api/v1/skills/" + key + "/export")
    assert export.status_code == 200
    assert export.text.startswith("---\nname: release-runbook\n")
    assert "attachment" in export.headers["content-disposition"]

    archive = signed.get("/api/v1/skills/" + key + "/export.zip")
    assert archive.status_code == 200
    with zipfile.ZipFile(io.BytesIO(archive.content)) as bundle:
        assert bundle.namelist() == ["SKILL.md"]
        assert b"# Release runbook" in bundle.read("SKILL.md")


def test_import_rejects_invalid_text(signed):
    response = signed.post("/api/v1/skills/import", json={"text": "# no frontmatter"})
    assert response.status_code == 422


def test_import_archive_and_read_resource(signed):
    raw = io.BytesIO()
    with zipfile.ZipFile(raw, "w") as archive:
        archive.writestr("zip-skill/SKILL.md", SKILL_MD.replace("release-runbook", "zip-skill"))
        archive.writestr("zip-skill/references/policy.md", "policy body")
    response = signed.post(
        "/api/v1/skills/import/archive",
        files={"file": ("zip-skill.zip", raw.getvalue(), "application/zip")},
    )
    assert response.status_code == 200, response.text
    imported = response.json()
    assert len(imported) == 1
    key = imported[0]["id"]
    assert list(imported[0]["data"]["files"]) == ["references/policy.md"]

    listing = signed.get("/api/v1/skills/" + key + "/resources")
    assert listing.status_code == 200
    assert listing.json()["files"][0]["path"] == "references/policy.md"

    resource = signed.get("/api/v1/skills/" + key + "/resources", params={"path": "references/policy.md"})
    assert resource.json()["content"] == "policy body"

    missing = signed.get("/api/v1/skills/" + key + "/resources", params={"path": "nope.md"})
    assert missing.status_code == 404


def test_import_from_directory(signed, tmp_path, monkeypatch):
    monkeypatch.setattr(config, "SKILLS_DIR", tmp_path)
    skill = tmp_path / "dir-skill"
    skill.mkdir()
    skill.joinpath("SKILL.md").write_text(SKILL_MD.replace("release-runbook", "dir-skill"))
    response = signed.post("/api/v1/skills/import/directory")
    assert response.status_code == 200, response.text
    assert response.json()["imported"] == 1
    assert response.json()["skills"][0]["data"]["source"] == "directory"


def test_import_upserts_by_slug(signed):
    first = signed.post("/api/v1/skills/import", json={"text": SKILL_MD}).json()
    second = signed.post(
        "/api/v1/skills/import",
        json={"text": SKILL_MD.replace("1. Run tests.", "1. Run tests twice.")},
    ).json()
    assert first["id"] == second["id"]
    assert "twice" in second["data"]["content"]


def test_skill_resource_tool_resolution(signed):
    signed.post("/api/v1/skills/import", json={"text": SKILL_MD})
    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        row = skill_service.find(db, owner, "release-runbook")
        assert row is not None
        assert skill_service.read_resource(db, owner, row.id, "")["files"] == []
        with pytest.raises(ValueError):
            skill_service.read_resource(db, owner, row.id, "../escape")
        assert skill_service.find(db, owner, "missing-skill") is None
