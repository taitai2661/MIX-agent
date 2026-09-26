"""Tests for tiered summary rotation."""

from mix_agent.context import tiered_summary


def test_install_summary_caps_text():
    run_data: dict = {}
    tiered_summary.install_summary(run_data, "x" * 5000, covered_count=2)
    assert len(run_data["summary"]["text"]) <= tiered_summary.MAX_ACTIVE_CHARS


def test_archive_summary_rotates_previous_text():
    run_data: dict = {}
    tiered_summary.install_summary(run_data, "first summary", covered_count=1)
    tiered_summary.archive_summary(run_data, "second summary", covered_count=5)
    assert run_data["summary"]["text"] == "second summary"
    assert run_data["summary"]["covered_count"] == 5
    archive = tiered_summary.current_archive(run_data)
    assert len(archive) == 1
    assert archive[0]["text"] == "first summary"


def test_archive_summary_caps_rotation():
    run_data: dict = {}
    for index in range(tiered_summary.MAX_ARCHIVE_ROWS + 5):
        tiered_summary.install_summary(run_data, f"v{index}", covered_count=index)
        tiered_summary.archive_summary(run_data, f"v{index + 1}", covered_count=index + 1)
    archive = tiered_summary.current_archive(run_data)
    assert len(archive) == tiered_summary.MAX_ARCHIVE_ROWS


def test_rerank_archive_prefers_overlapping_rows():
    rows = [
        {"text": "The sky is blue.", "covered_count": 1},
        {"text": "Random unrelated text.", "covered_count": 1},
        {"text": "The grass is green; the sky is blue.", "covered_count": 1},
    ]
    ranked = tiered_summary.rerank_archive(rows, "sky color", top_k=2)
    # Both rows mentioning "sky" must rank above the unrelated one.
    assert "sky" in ranked[0]["text"]


def test_render_archive_emits_header_and_clips_long_rows():
    rows = [{"text": "x" * 2000, "covered_count": 1}]
    rendered = tiered_summary.render_archive(rows)
    assert rendered.startswith("Archived summary excerpts")
    assert rendered.count("\n") >= 1
