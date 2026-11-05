"""The SKILL.md naming rule on the write paths that do not go through SKILL.md.

``POST /skills``, ``PATCH`` and the ``skill_add`` / ``skill_update`` tools take
name and description directly, so without this they would bypass the rule the
import paths enforce. Validation is the same function ``parse_skill_md`` uses.
"""

import pytest
from mix_agent.skills import package, service


def test_a_valid_identity_passes():
    service._validate_identity("my-skill", "does a thing")


@pytest.mark.parametrize(
    "name",
    ["", "My Skill", "my--skill", "-my-skill", "my-skill-", "skill_1", "x" * 65],
)
def test_names_the_rule_rejects(name):
    with pytest.raises(ValueError, match="name"):
        service._validate_identity(name, "does a thing")


def test_a_missing_description_is_rejected():
    with pytest.raises(ValueError, match="description"):
        service._validate_identity("my-skill", "")
    with pytest.raises(ValueError, match="description"):
        service._validate_identity("my-skill", "   ")


def test_the_error_names_every_problem_at_once():
    with pytest.raises(ValueError) as excinfo:
        service._validate_identity("Bad Name", "")
    message = str(excinfo.value)
    assert "name" in message
    assert "description" in message


def test_an_empty_compatibility_is_rejected():
    with pytest.raises(package.SkillFormatError, match="compatibility"):
        package.parse_skill_md("---\nname: ok\ndescription: ok\ncompatibility: ''\n---\nbody")


def test_scan_directory_report_rejects_a_name_that_does_not_match_the_folder(tmp_path):
    skill = tmp_path / "alpha"
    skill.mkdir()
    (skill / "SKILL.md").write_text("---\nname: beta\ndescription: copied\n---\nbody", encoding="utf-8")

    packages, failures = package.scan_directory_report(tmp_path)
    assert packages == []
    assert len(failures) == 1
    assert "must match" in failures[0]["reason"]


def test_scan_directory_report_returns_failures(tmp_path):
    good = tmp_path / "good-skill"
    good.mkdir()
    (good / "SKILL.md").write_text("---\nname: good-skill\ndescription: ok\n---\nbody", encoding="utf-8")
    bad = tmp_path / "bad-skill"
    bad.mkdir()
    (bad / "SKILL.md").write_text("broken", encoding="utf-8")

    packages, failures = package.scan_directory_report(tmp_path)
    assert [pkg["name"] for pkg in packages] == ["good-skill"]
    assert len(failures) == 1
    assert failures[0]["path"].endswith("SKILL.md")
    # scan_directory keeps its original shape for existing callers.
    assert [pkg["name"] for pkg in package.scan_directory(tmp_path)] == ["good-skill"]
