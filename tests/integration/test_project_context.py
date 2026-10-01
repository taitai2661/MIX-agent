"""The bind-mounted project's instruction file and skills reach the run.

``MIX_PROJECT_MOUNT`` is a read-only view of one host folder; this asserts the
two things read from it — ``AGENTS.md`` / ``CLAUDE.md`` and the skills under
``.mix/skills`` — actually land in the frozen system head rather than being
discovered and then dropped.
"""

from pathlib import Path

from mix_agent import config, project_docs
from mix_agent.api import routes
from mix_agent.db.models import Model, Provider, Run, User
from mix_agent.db.session import SessionLocal
from mix_agent.skills import discovery
from sqlalchemy import select


def make_model(context=20000):
    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        provider = Provider(owner_id=owner, data={"kind": "openai"})
        db.add(provider)
        db.flush()
        model = Model(
            owner_id=owner,
            data={
                "provider_id": provider.id,
                "model_id": "project-context-test",
                "capabilities": {"tools": True},
                "context_window": context,
            },
        )
        db.add(model)
        db.commit()
        return model.id


def send(signed, monkeypatch, model_id, prompt="hello"):
    monkeypatch.setattr(routes, "launch", lambda _: None)
    conversation = signed.post("/api/v1/conversations", json={}).json()["id"]
    response = signed.post(
        f"/api/v1/conversations/{conversation}/messages",
        json={"model_id": model_id, "content": prompt, "mode": "chat"},
        headers={"Idempotency-Key": conversation},
    )
    assert response.status_code == 200, response.text
    with SessionLocal() as db:
        run = db.get(Run, response.json()["run_id"])
        head = (run.data.get("history") or [{}])[0].get("content", "")
        return head


def write_project(tmp_path: Path):
    (tmp_path / "AGENTS.md").write_text(
        "always run `pytest` before answering", encoding="utf-8"
    )
    skill = tmp_path / ".mix" / "skills" / "release-notes"
    skill.mkdir(parents=True)
    (skill / "SKILL.md").write_text(
        "---\nname: release-notes\ndescription: draft release notes\n---\n\nSteps.\n",
        encoding="utf-8",
    )
    return tmp_path


def test_agents_md_reaches_the_system_head(signed, tmp_path, monkeypatch):
    monkeypatch.setattr(config, "PROJECT_DIR", write_project(tmp_path))
    head = send(signed, monkeypatch, make_model())
    assert "AGENTS.md (project instructions file):" in head
    assert "always run `pytest` before answering" in head
    assert project_docs.UNTRUSTED_NOTE.strip() in head


def test_project_skills_reach_the_system_head(signed, tmp_path, monkeypatch):
    monkeypatch.setattr(config, "PROJECT_DIR", write_project(tmp_path))
    head = send(signed, monkeypatch, make_model(), prompt="draft the release notes")
    assert "Relevant reusable skills" in head
    assert "release-notes" in head
    assert "draft release notes" in head


def test_without_a_mounted_folder_nothing_is_invented(signed, tmp_path, monkeypatch):
    monkeypatch.setattr(config, "PROJECT_DIR", tmp_path / "absent")
    head = send(signed, monkeypatch, make_model())
    assert "AGENTS.md" not in head
    assert "Relevant reusable skills" not in head


def test_discovery_reads_from_the_configured_mount(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "PROJECT_DIR", tmp_path)
    skill = tmp_path / ".claude" / "skills" / "triage"
    skill.mkdir(parents=True)
    (skill / "SKILL.md").write_text(
        "---\nname: triage\ndescription: sort incoming issues\n---\nbody",
        encoding="utf-8",
    )
    hits = discovery.search("issues")
    assert [row["name"] for row in hits] == ["triage"]
