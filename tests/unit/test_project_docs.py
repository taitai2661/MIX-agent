"""``AGENTS.md`` / ``CLAUDE.md`` read from the bind-mounted project folder."""

from mix_agent import project_docs


def test_agents_md_is_read(tmp_path):
    (tmp_path / "AGENTS.md").write_text("always run the test suite", encoding="utf-8")

    name, content = project_docs.read_project_docs(tmp_path)
    assert name == "AGENTS.md"
    assert content == "always run the test suite"
    block = project_docs.render_project_docs_block(tmp_path)
    assert "AGENTS.md (project instructions file):" in block
    assert "always run the test suite" in block
    assert "user-provided data" in block


def test_claude_md_is_the_fallback(tmp_path):
    (tmp_path / "CLAUDE.md").write_text("prefer ruff defaults", encoding="utf-8")

    name, content = project_docs.read_project_docs(tmp_path)
    assert name == "CLAUDE.md"
    assert content == "prefer ruff defaults"


def test_agents_md_wins_when_both_exist(tmp_path):
    (tmp_path / "AGENTS.md").write_text("the standard file", encoding="utf-8")
    (tmp_path / "CLAUDE.md").write_text("the other file", encoding="utf-8")

    name, content = project_docs.read_project_docs(tmp_path)
    assert name == "AGENTS.md"
    assert content == "the standard file"


def test_no_instruction_file_means_no_block(tmp_path):
    assert project_docs.read_project_docs(tmp_path) is None
    assert project_docs.render_project_docs_block(tmp_path) == ""


def test_a_missing_folder_degrades_to_no_block(tmp_path):
    assert project_docs.read_project_docs(tmp_path / "absent") is None
    assert project_docs.render_project_docs_block(tmp_path / "absent") == ""


def test_content_is_capped(tmp_path):
    (tmp_path / "AGENTS.md").write_text("x" * 500_000, encoding="utf-8")

    _name, content = project_docs.read_project_docs(tmp_path)
    assert len(content) == project_docs.DOC_MAX_CHARS


def test_an_empty_file_renders_nothing(tmp_path):
    (tmp_path / "AGENTS.md").write_text("   \n\n", encoding="utf-8")

    assert project_docs.render_project_docs_block(tmp_path) == ""
