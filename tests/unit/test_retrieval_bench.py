"""The retrieval bench is under test: a measuring device nobody tests is a
device nobody notices has stopped measuring.

``experiments/run_retrieval_bench.py`` is the observation device for spec §5.
These tests run its **fixture** suite only — no database, no network — and pin
the two things the rest of the tooling depends on: the numbers it reports for
the shared recall contract, and the exit code contract (measure and pass 0, fail
only on a threshold that was asked for 1, refuse to pretend a threshold was
checked when nothing could measure it 2).
"""

from __future__ import annotations

import importlib.util
from pathlib import Path

import pytest

BENCH = Path(__file__).resolve().parents[2] / "experiments" / "run_retrieval_bench.py"


@pytest.fixture(scope="module")
def bench():
    """Load the bench by path — ``experiments/`` is not a package."""
    spec = importlib.util.spec_from_file_location("run_retrieval_bench", BENCH)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_fixture_suite_reports_the_shared_contract_numbers(bench):
    args = bench.parse_args(["--suite", "fixture"])
    report = bench.build_report(args, lambda _message: None)
    values = report["metrics"]["fixture"]["values"]

    # Every fixture case with an expected answer puts it first (that is what
    # tests/unit/test_memory_ranking.py asserts), so recall and MRR are 1.0 by
    # construction: a drop here means the bench changed how it reads the
    # fixture, not that retrieval got worse.
    assert values["recall_at_5"] == 1.0
    assert values["recall_at_8"] == 1.0
    assert values["mrr"] == 1.0

    # The fixture never calls service.search(), so it has no stopwatch and no
    # latency to report — `None`, not a made-up zero.
    assert values["latency_p50_ms"] is None
    assert values["latency_p95_ms"] is None

    counts = report["counts"]
    assert counts["fixture_cases"] >= 7
    # At least one case answers "nothing should come back" and therefore has
    # no expected set: it counts as a case but not as an evaluable one.
    assert counts["fixture_evaluable"] < counts["fixture_cases"]
    assert report["definitions"]["noise_rate"]


def test_bench_exit_codes_only_answer_thresholds(bench, capsys):
    # Measured something, whatever the numbers: 0.
    assert bench.main(["--suite", "fixture"]) == bench.EXIT_OK
    # A threshold that holds: 0.
    assert bench.main(["--suite", "fixture", "--min-recall5", "1.0"]) == bench.EXIT_OK
    # A known degradation (nothing here is degraded, the bar is simply higher
    # than the measurement): 1, and only because it was asked for.
    assert bench.main(["--suite", "fixture", "--min-recall5", "1.1"]) == bench.EXIT_REGRESSION
    # A latency threshold with no suite that runs search(): refuse to claim it
    # was checked: 2.
    assert bench.main(["--suite", "fixture", "--max-p95-ms", "50"]) == bench.EXIT_ERROR
    capsys.readouterr()


def test_score_case_never_divides_by_zero(bench):
    # Expected set of 0: recall and MRR are undefined, not 0/0.
    empty = bench.score_case("empty", [], [], [])
    assert empty["recall_at_5"] is None
    assert empty["recall_at_8"] is None
    assert empty["mrr"] is None
    assert empty["noise_rate"] is None

    # Everything dropped (min_score) or nothing matched: a case with an expected
    # set scores 0 for recall/MRR and reports no noise because it returned rows
    # of nothing.
    dropped = bench.score_case("dropped", {"a", "b"}, {"x", "y"}, [])
    assert dropped["recall_at_5"] == 0.0
    assert dropped["mrr"] == 0.0
    assert dropped["noise_rate"] is None

    # One expectation-violating row ahead of the hit: recall sees "found it",
    # MRR sees "found it late" — the two metrics have to move apart.
    split = bench.score_case("split", {"a"}, {"x"}, ["x", "a"])
    assert split["recall_at_5"] == 1.0
    assert split["mrr"] == 0.5  # first hit at rank 2
    assert split["noise_rate"] == 0.5
