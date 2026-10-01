import base64
import io
import zipfile

import pytest

from mix_agent.skills import package

VALID = """---
name: expense-report
description: File and validate expense reports. Use when asked about reimbursements.
license: Apache-2.0
compatibility: Requires python3
metadata:
  author: finance
  version: "2.1"
allowed-tools: Read Write
---

# Expense report

Follow the company policy.
"""


def test_parse_valid_frontmatter():
    parsed = package.parse_skill_md(VALID)
    assert parsed["name"] == "expense-report"
    assert parsed["license"] == "Apache-2.0"
    assert parsed["metadata"] == {"author": "finance", "version": "2.1"}
    assert parsed["allowed_tools"] == ["Read", "Write"]
    assert parsed["content"].startswith("# Expense report")
    assert "---" not in parsed["content"]


def test_parse_rejects_bad_frontmatter():
    with pytest.raises(package.SkillFormatError):
        package.parse_skill_md("# no frontmatter")
    with pytest.raises(package.SkillFormatError):
        package.parse_skill_md("---\nname: Bad Name\ndescription: x\n---\nbody")
    with pytest.raises(package.SkillFormatError):
        package.parse_skill_md("---\nname: ok\ndescription: ''\n---\nbody")
    with pytest.raises(package.SkillFormatError):
        package.parse_skill_md("---\nname: ok\ndescription: " + "x" * 1025 + "\n---\nbody")
    with pytest.raises(package.SkillFormatError):
        package.parse_skill_md("---\nname: a--b\ndescription: x\n---\nbody")


def test_render_round_trip():
    parsed = package.parse_skill_md(VALID)
    rendered = package.render_skill_md(parsed)
    assert rendered.startswith("---\nname: expense-report\n")
    again = package.parse_skill_md(rendered)
    assert again["name"] == parsed["name"]
    assert again["metadata"] == parsed["metadata"]
    assert again["allowed_tools"] == parsed["allowed_tools"]
    assert again["content"] == parsed["content"]


def test_encode_file_handles_binary():
    text = package.encode_file(b"hello")
    assert text["binary"] is False and text["content"] == "hello"
    binary = package.encode_file(b"\xff\xfe\x00")
    assert binary["binary"] is True
    assert base64.b64decode(binary["content"]) == b"\xff\xfe\x00"


def test_archive_round_trip_with_files():
    raw = io.BytesIO()
    with zipfile.ZipFile(raw, "w") as archive:
        archive.writestr("expense-report/SKILL.md", VALID)
        archive.writestr("expense-report/references/forms.md", "form text")
        archive.writestr("expense-report/scripts/validate.py", "print('ok')")
    packages = package.parse_archive(raw.getvalue())
    assert len(packages) == 1
    assert sorted(packages[0]["files"]) == ["references/forms.md", "scripts/validate.py"]

    rebuilt = package.bundle_archive({**packages[0], "files": packages[0]["files"]})
    with zipfile.ZipFile(io.BytesIO(rebuilt)) as archive:
        assert set(archive.namelist()) == {"SKILL.md", "references/forms.md", "scripts/validate.py"}
        assert archive.read("references/forms.md") == b"form text"
    round_trip = package.parse_archive(rebuilt)
    assert round_trip[0]["content"] == packages[0]["content"]


def test_archive_root_level_skill():
    raw = io.BytesIO()
    with zipfile.ZipFile(raw, "w") as archive:
        archive.writestr("SKILL.md", VALID)
        archive.writestr("references/forms.md", "form text")
    packages = package.parse_archive(raw.getvalue())
    assert len(packages) == 1
    assert list(packages[0]["files"]) == ["references/forms.md"]


def test_archive_rejects_traversal_and_missing_skill():
    raw = io.BytesIO()
    with zipfile.ZipFile(raw, "w") as archive:
        archive.writestr("../evil/SKILL.md", VALID)
    with pytest.raises(package.SkillFormatError):
        package.parse_archive(raw.getvalue())
    with pytest.raises(package.SkillFormatError):
        package.parse_archive(b"not a zip")
    empty = io.BytesIO()
    with zipfile.ZipFile(empty, "w") as archive:
        archive.writestr("readme.md", "hello")
    with pytest.raises(package.SkillFormatError):
        package.parse_archive(empty.getvalue())


def test_scan_directory(tmp_path):
    skill = tmp_path / "my-skill"
    (skill / "references").mkdir(parents=True)
    skill.joinpath("SKILL.md").write_text(VALID.replace("expense-report", "my-skill"))
    skill.joinpath("references/note.md").write_text("note")
    (tmp_path / "not-a-skill").mkdir()
    packages = package.scan_directory(tmp_path)
    assert [item["name"] for item in packages] == ["my-skill"]
    assert list(packages[0]["files"]) == ["references/note.md"]


def test_scan_directory_missing(tmp_path):
    assert package.scan_directory(tmp_path / "nope") == []
