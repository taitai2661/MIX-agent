"""Shared Agent Skills conformance fixtures.

The same directory is read by the Desktop Rust test, so a parser change in one
edition that disagrees with the other fails in CI instead of only in the field.
"""

import json
from pathlib import Path

from mix_agent.skills import package

FIXTURES = Path(__file__).resolve().parents[2] / "tests" / "fixtures" / "skills"


def expected() -> dict:
    return json.loads((FIXTURES / "expected.json").read_text(encoding="utf-8"))


def server_packages() -> dict:
    packages, _failures = package.scan_directory_report(FIXTURES)
    return {item["name"]: item for item in packages}


def test_every_spec_fixture_parses_to_the_expected_fields():
    packages = server_packages()
    for name, want in expected().items():
        if want.get("desktop_only"):
            # The plain-Markdown fallback is deliberately Desktop-only; the
            # Server requires the specification's frontmatter.
            assert name not in packages
            continue
        got = packages[name]
        assert got["name"] == want["name"]
        assert got["description"] == want["description"]
        assert got["license"] == want["license"]
        assert got["compatibility"] == want["compatibility"]
        assert got["metadata"] == want["metadata"]
        assert got["allowed_tools"] == want["allowed_tools"]
        assert sorted(got["files"]) == want["resources"]


def test_optional_fields_survive_a_render_round_trip():
    packages = server_packages()
    for name, want in expected().items():
        if want.get("desktop_only"):
            continue
        rendered = package.render_skill_md(packages[name])
        again = package.parse_skill_md(rendered)
        for field in ("name", "description", "license", "compatibility", "metadata", "allowed_tools"):
            assert again[field] == want[field], (name, field)


def test_allowed_tools_is_rendered_as_a_space_separated_string():
    packages = server_packages()
    rendered = package.render_skill_md(packages["optional-fields"])
    assert "allowed-tools: Read Write Bash(git:*)" in rendered
    assert "allowed-tools: [" not in rendered


def test_a_binary_asset_is_preserved_in_the_package():
    packages = server_packages()
    file = packages["bundled"]["files"]["assets/logo.bin"]
    assert file["binary"] is True
    assert file["size"] == 3
