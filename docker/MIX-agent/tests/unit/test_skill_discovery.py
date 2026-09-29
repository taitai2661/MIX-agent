"""Discovery of skills that live in the project folder.

The chain mirrors the other coding agents' directory names and stops at the
first hit: a repository contributes skills from one directory, never from all
three at once, so the effective catalogue does not depend on which files
happen to exist.
"""

from pathlib import Path

from mix_agent.skills import discovery


def write_skill(
    root: Path, folder: str, name: str, description: str = "does a thing"
) -> Path:
    skill_dir = root / folder / name
    skill_dir.mkdir(parents=True, exist_ok=True)
    (skill_dir / "SKILL.md").write_text(
        f"---\nname: {name}\ndescription: {description}\n---\n\nSteps.\n",
        encoding="utf-8",
    )
    return skill_dir


def test_mix_directory_wins_over_the_others(tmp_path):
    write_skill(tmp_path, ".mix/skills", "alpha")
    write_skill(tmp_path, ".claude/skills", "beta")
    write_skill(tmp_path, ".agents/skills", "gamma")

    assert discovery.skill_root(tmp_path) == tmp_path / ".mix/skills"
    packages, failures = discovery.discover(tmp_path)
    assert failures == []
    assert [pkg["name"] for pkg in packages] == ["alpha"]


def test_chain_falls_back_to_agents(tmp_path):
    write_skill(tmp_path, ".agents/skills", "gamma")

    assert discovery.skill_root(tmp_path) == tmp_path / ".agents/skills"
    packages, failures = discovery.discover(tmp_path)
    assert failures == []
    assert [pkg["name"] for pkg in packages] == ["gamma"]


def test_no_skill_directory_means_no_skills(tmp_path):
    assert discovery.skill_root(tmp_path) is None
    assert discovery.discover(tmp_path) == ([], [])


def test_a_malformed_skill_is_reported_not_dropped(tmp_path):
    write_skill(tmp_path, ".mix/skills", "good")
    bad = tmp_path / ".mix/skills" / "bad"
    bad.mkdir(parents=True)
    (bad / "SKILL.md").write_text("no frontmatter, no heading", encoding="utf-8")

    packages, failures = discovery.discover(tmp_path)
    assert [pkg["name"] for pkg in packages] == ["good"]
    assert len(failures) == 1
    assert failures[0]["path"].endswith("SKILL.md")
    assert failures[0]["reason"]


def test_a_name_the_rule_rejects_is_a_failure(tmp_path):
    bad = tmp_path / ".mix/skills" / "Bad Name"
    bad.mkdir(parents=True)
    (bad / "SKILL.md").write_text(
        "---\nname: Bad Name\ndescription: nope\n---\nbody", encoding="utf-8"
    )
    packages, failures = discovery.discover(tmp_path)
    assert packages == []
    assert len(failures) == 1


def test_bundled_files_are_listed_not_inlined(tmp_path):
    skill_dir = write_skill(tmp_path, ".mix/skills", "packaged")
    (skill_dir / "helper.py").write_text("print('hi')\n", encoding="utf-8")

    packages, _failures = discovery.discover(tmp_path)
    assert sorted(packages[0]["files"]) == ["helper.py"]
    view = discovery.context_view(packages[0])
    assert view["files"] == ["helper.py"]
    assert view["id"] == "project:packaged"
    assert view["source"] == "project"


def test_symlinks_are_not_followed(tmp_path):
    skill_dir = write_skill(tmp_path, ".mix/skills", "linked")
    outside = tmp_path / "outside"
    outside.mkdir()
    (outside / "secret.txt").write_text("sensitive", encoding="utf-8")
    (skill_dir / "escape").symlink_to(outside / "secret.txt")

    packages, _failures = discovery.discover(tmp_path)
    assert "escape" not in packages[0]["files"]


def test_search_filters_by_query(tmp_path):
    write_skill(tmp_path, ".mix/skills", "release-notes", "draft release notes")
    write_skill(tmp_path, ".mix/skills", "expense-report", "build expense reports")

    hits = discovery.search("release", tmp_path)
    assert [row["name"] for row in hits] == ["release-notes"]
    assert discovery.search("nothing-matches", tmp_path) == []


def test_find_and_read_resource_round_trip(tmp_path):
    skill_dir = write_skill(tmp_path, ".mix/skills", "packaged")
    (skill_dir / "helper.py").write_text("print('hi')\n", encoding="utf-8")

    pkg = discovery.find("packaged", tmp_path)
    assert pkg is not None
    listing = discovery.read_resource(pkg)
    assert [entry["path"] for entry in listing["files"]] == ["helper.py"]
    body = discovery.read_resource(pkg, "helper.py")
    assert body["content"] == "print('hi')\n"
    assert discovery.find("absent", tmp_path) is None
