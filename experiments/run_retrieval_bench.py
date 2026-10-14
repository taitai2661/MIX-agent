#!/usr/bin/env python3
"""Retrieval-quality bench for ``docs/memory-spec.md`` §5 (Docker edition).

Two suites, one set of metric names and one set of definitions — a number here
has to mean the same thing next week and in the Desktop edition:

``fixture``
    Reads ``tests/fixtures/memory_recall_cases.json`` **through**
    ``tests/unit/test_memory_ranking.py``. The bench imports that module's
    ``_candidates`` instead of writing a second reader: the fixture is parsed
    in exactly one place, so the bench and the ranking contract can never
    drift apart. Pure — no database, no network.

``synthetic``
    Builds a deterministic corpus of ``--size`` rows (seeded; the only
    randomness is one ``random.Random(seed)``) in the test database and scores
    real ``service.search()`` calls, timing each one. ``--size 10000`` gives the
    p50/p95 of a 10k store.

Modes
    ``lexical`` (default)  no embeddings anywhere, so §5.2's "embedding
        unavailable" weights (0.85 / 0.15) decide. This is the pre-embedding
        ranking, and running it is what makes the no-embedding identity
        measurable rather than assumed.
    ``--mode hybrid``      a deterministic, vocabulary-derived stub embedding
        goes through §5's ``vector`` term. No external API, no model download,
        no randomness: the same vocabulary always produces the same vector.

This program is an *observation device*. It never changes ranking, extraction
or linking behaviour, and by default it only measures: the exit code turns
non-zero solely when an explicitly supplied threshold is violated (1) or when a
supplied threshold cannot be measured at all (2). Thresholds are checked
against every suite that ran — gate one suite by running one suite
(``--suite synthetic``) — and a metric a suite structurally cannot produce
(fixture latency) is skipped there instead of failing the run.

Run it inside the test image (it needs ``mix_agent`` and PostgreSQL, but no
network and no LLM API)::

    docker compose --profile test run --build --rm test \
      sh -c "cd /app && alembic upgrade head && python /workspace/experiments/run_retrieval_bench.py --size 1000"

Progress goes to stderr, the human report to stdout, ``--json`` writes the
machine-readable twin of the same report.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import math
import platform
import sys
import time
from datetime import UTC, datetime, timedelta
from pathlib import Path

# Importing the ranking implementation touches no database and no network —
# only `mix_agent.db.session` (opened lazily, synthetic suite only) builds an
# engine, so the fixture suite runs without DATABASE_URL.
from mix_agent.memory import service

ROOT = Path(__file__).resolve().parents[1]
CONTRACT_MODULE = ROOT / "tests" / "unit" / "test_memory_ranking.py"
FIXTURE_PATH = ROOT / "tests" / "fixtures" / "memory_recall_cases.json"
BENCH_OWNER = "retrieval-bench"

EXIT_OK = 0
EXIT_REGRESSION = 1
EXIT_ERROR = 2

# §5.2 / §5.1, stated so the report carries the contract it measures against.
DEFINITIONS = {
    "recall_at_5": (
        "mean over cases with a non-empty expected set of "
        "|expected ∩ returned[:5]| / |expected|"
    ),
    "recall_at_8": (
        "mean over cases with a non-empty expected set of "
        "|expected ∩ returned[:8]| / |expected|"
    ),
    "mrr": (
        "mean over the same cases of 1 / rank of the first expected row in the "
        "returned order; 0 when a case returns none of its expected rows"
    ),
    "noise_rate": (
        "mean over cases that returned anything of |returned ∩ must_not_match| / "
        "|returned|; must_not_match is every candidate the case does not expect "
        "(fixture: its candidates minus expected, synthetic: the corpus minus expected)"
    ),
    "latency_p50_ms": "nearest-rank p50 of one service.search() call, wall clock",
    "latency_p95_ms": "nearest-rank p95 of one service.search() call, wall clock",
}

THRESHOLDS = (
    ("min_recall5", "recall_at_5", "min"),
    ("min_recall8", "recall_at_8", "min"),
    ("min_mrr", "mrr", "min"),
    ("max_noise", "noise_rate", "max"),
    ("max_p95_ms", "latency_p95_ms", "max"),
)

# What each suite can express. A threshold aimed at a metric a suite structurally
# cannot produce (latency: the fixture never calls search()) is *skipped* for
# that suite, not reported as "unmeasurable" — otherwise `--max-p95-ms` on a
# default run would fail because the fixture suite has no stopwatch.
SUITE_METRICS = {
    "fixture": ("recall_at_5", "recall_at_8", "mrr", "noise_rate"),
    "synthetic": (
        "recall_at_5", "recall_at_8", "mrr", "noise_rate",
        "latency_p50_ms", "latency_p95_ms",
    ),
}

# --- synthetic corpus ---------------------------------------------------------
# Eight topics, three topic words each, all distinct and all latin tokens, so
# `terms()` yields exactly the tokens the generator put in the text and
# |Q ∩ T| / |Q| is arithmetic rather than a guess. CJK bigram tokenisation is
# exercised by the unit tests; a bench needs predictable overlap.
TOPICS: tuple[tuple[str, tuple[str, ...]], ...] = (
    ("database", ("postgres", "schema", "migration")),
    ("frontend", ("react", "render", "component")),
    ("network", ("proxy", "timeout", "retry")),
    ("security", ("token", "session", "audit")),
    ("storage", ("backup", "restore", "archive")),
    ("billing", ("invoice", "refund", "quota")),
    ("retrieval", ("index", "analyzer", "ranking")),
    ("deploy", ("release", "rollback", "canary")),
)
# Two per topic (16 = 2 * len(TOPICS)): a query is "two shared words + one
# topic word", which is what lets a *different* topic still match 2/3 of it.
GENERIC = (
    "config", "error", "setup", "request", "result", "cache", "limit", "report",
    "status", "record", "process", "value", "field", "option", "detail", "event",
)
# Neutral words for document tails and filler: disjoint from every query term,
# so filler can only ever rank on recency.
TAILS = (
    "notebook", "summary", "draft", "agenda", "remind", "review", "sample", "lesson",
    "budget", "travel", "recipe", "weather", "garden", "music", "camera", "window",
    "bottle", "ticket", "pocket", "letter", "bridge", "forest", "island", "river",
    "mountain", "valley", "harbor", "desert", "canyon", "meadow", "cabin", "village",
)

# Stub embedding: how much a shared topic outweighs shared tokens. Documented
# constant, not a tuned one — it only has to be big enough for the vector term
# to express "same topic" at all.
TOPIC_MASS = 4.0
STUB_MODEL = "bench-bow-v1"

SCORED_AGE_MAX = 150  # expected/distractor rows, days
FILLER_AGE_MIN = 180  # filler rows, days: always older than any scored row
FILLER_AGE_SPAN = 720


class BenchError(RuntimeError):
    """A threshold was asked for but cannot be measured, or the run is impossible."""


def progress(message: str) -> None:
    print(f"bench: {message}", file=sys.stderr, flush=True)


# --- metrics ------------------------------------------------------------------
def percentile(values, fraction):
    """Nearest-rank percentile; ``None`` when there is nothing to measure."""
    if not values:
        return None
    ordered = sorted(values)
    rank = max(1, math.ceil(fraction * len(ordered)))
    return ordered[rank - 1]


def score_case(case_id, expected, must_not, returned, query=None):
    """Score one case against §5's four quality metrics. Never divides by zero."""
    expected = set(expected)
    must_not = set(must_not)
    returned = list(returned)
    evaluable = bool(expected)
    count = len(returned)
    hits5 = len(set(returned[:5]) & expected)
    hits8 = len(set(returned[:8]) & expected)
    first_rank = next(
        (rank for rank, row_id in enumerate(returned, start=1) if row_id in expected),
        None,
    )
    noise_hits = sum(1 for row_id in returned if row_id in must_not)
    result = {
        "case": case_id,
        "expected": len(expected),
        "returned": count,
        "must_not_match": len(must_not),
        "hits_at_5": hits5 if evaluable else None,
        "hits_at_8": hits8 if evaluable else None,
        "first_hit_rank": first_rank,
        "noise_hits": noise_hits,
        # No expected rows: recall/MRR are undefined for this case, not zero.
        "recall_at_5": (hits5 / len(expected)) if evaluable else None,
        "recall_at_8": (hits8 / len(expected)) if evaluable else None,
        "mrr": (1.0 / first_rank) if first_rank else (0.0 if evaluable else None),
        # Nothing returned: no noise happened, but nothing was measured either.
        "noise_rate": (noise_hits / count) if count else None,
    }
    if query is not None:
        result["query"] = query
    return result


def aggregate(cases, latencies):
    """Mean each metric across the cases that can express it, with its basis."""
    total = len(cases)
    values: dict[str, float | None] = {}
    basis: dict[str, str] = {}
    for metric in ("recall_at_5", "recall_at_8", "mrr"):
        picked = [case[metric] for case in cases if case[metric] is not None]
        values[metric] = round(sum(picked) / len(picked), 4) if picked else None
        basis[metric] = f"{len(picked)}/{total} evaluable cases"
    picked = [case["noise_rate"] for case in cases if case["noise_rate"] is not None]
    values["noise_rate"] = round(sum(picked) / len(picked), 4) if picked else None
    basis["noise_rate"] = f"{len(picked)}/{total} cases with results"
    for metric in ("latency_p50_ms", "latency_p95_ms"):
        point = percentile(latencies, 0.50 if metric.endswith("p50_ms") else 0.95)
        values[metric] = round(point, 2) if point is not None else None
        basis[metric] = f"{len(latencies)} search() samples"
    return {"values": values, "basis": basis}


# --- fixture suite ------------------------------------------------------------
def load_ranking_contract():
    """Import the ranking test module and reuse *its* fixture reader.

    The bench must read ``memory_recall_cases.json`` exactly the way
    ``tests/unit/test_memory_ranking.py`` does. Rather than growing a second,
    subtly different reader, it executes that module and calls its
    ``_candidates``: one parser, one interpretation of the shared contract.
    """
    if not CONTRACT_MODULE.exists():
        raise BenchError(f"ranking contract not found: {CONTRACT_MODULE}")
    spec = importlib.util.spec_from_file_location("memory_ranking_contract", CONTRACT_MODULE)
    if spec is None or spec.loader is None:
        raise BenchError(f"cannot load ranking contract: {CONTRACT_MODULE}")
    module = importlib.util.module_from_spec(spec)
    try:
        spec.loader.exec_module(module)
    except ModuleNotFoundError as exc:  # the test module imports pytest
        raise BenchError(
            f"{CONTRACT_MODULE.name} needs {exc.name!r}; run the bench in the test image "
            "(`docker compose --profile test run --rm test ...`)"
        ) from exc
    if not getattr(module, "CASES", None):
        raise BenchError(f"no cases loaded from {FIXTURE_PATH.name}")
    return module


def run_fixture_suite(mode, progress_log):
    """Score the shared fixture through the ranking contract's own reader."""
    contract = load_ranking_contract()
    progress_log(f"fixture: {len(contract.CASES)} cases from {FIXTURE_PATH.name} ({mode})")
    cases = []
    for case in contract.CASES:
        candidates = contract._candidates(case)
        # lexical = no query vector at all, so every pair takes §5.2's
        # without-vector weights; hybrid passes the case's vector exactly like
        # test_memory_ranking.py does.
        query_vector = case.get("query_vector") if mode == "hybrid" else None
        ranked = contract.service.rank(
            set(case["query_terms"]), candidates, query_vector=query_vector, limit=8
        )
        returned = [item["payload"] for item in ranked]
        expected = {case["expected"]} if case.get("expected") else set()
        candidate_ids = {memory["id"] for memory in case["memories"]}
        cases.append(
            score_case(
                case["id"],
                expected,
                candidate_ids - expected,
                returned,
                query=" ".join(case["query_terms"]),
            )
        )
        progress_log(
            f"fixture: {case['id']}: expected {len(expected)} · returned {len(returned)} "
            f"· first hit rank {cases[-1]['first_hit_rank']}"
        )
    payload = aggregate(cases, [])
    payload["counts"] = {
        "cases": len(cases),
        "evaluable": sum(1 for case in cases if case["recall_at_5"] is not None),
        "returned_rows": sum(case["returned"] for case in cases),
    }
    return payload, cases


# --- synthetic corpus ---------------------------------------------------------
def _tails(count, salt):
    """Deterministic tail words: same count + salt always gives the same words."""
    return [TAILS[(salt + step * 5) % len(TAILS)] for step in range(count)]


def build_corpus(size, seed):
    """Rows that answer the bench's queries, plus filler up to ``size``.

    Ages decide what is measurable: expected and distractor rows share one
    1..150 day band, filler sits at 180..899 days. ``search()`` only scores the
    newest ``CANDIDATE_LIMIT`` rows, so every row a query can answer with stays
    inside that window at any --size while the filler still has to be sorted by
    the database — which is exactly what the latency numbers are about.

    ``seed`` shifts that recency assignment (nothing else): content, ids and
    overlap are index-derived, so one seed means one corpus and a second seed
    means the same corpus with a different — still deterministic — draw of ages.
    """
    rows: list[dict] = []
    queries: list[dict] = []
    position = 0
    for index, (topic, words) in enumerate(TOPICS):
        g1, g2 = GENERIC[2 * index], GENERIC[2 * index + 1]
        t0, t1, t2 = words
        other = (index + 3) % len(TOPICS)
        third = (index + 5) % len(TOPICS)
        # (kind, core tokens, tail words). Expected rows span 3/3 → 1/3 of the
        # query; distractors are a cross-topic lexical decoy (2/3), a
        # same-topic but lexically disjoint row (0/3), and two single-word
        # partials. Both directions of "lexical says yes / topic says no" and
        # vice versa are in here on purpose: that is the argument between
        # lexical and hybrid.
        members = (
            ("expected", [g1, g2, t0], 0),
            ("expected", [g1, g2, t0], 9),
            ("expected", [g1, t0], 3),
            ("expected", [g2, t0], 5),
            ("expected", [t0], 2),
            ("distractor", [g1, g2, TOPICS[other][1][0], TOPICS[other][1][1]], 4),
            ("distractor", [g1, g2, TOPICS[third][1][1], TOPICS[third][1][2]], 6),
            ("distractor", [t1, t2], 1),
            ("distractor", [t1, t2], 7),
            ("distractor", [g1], 4),
            ("distractor", [g2], 6),
            ("distractor", [TOPICS[other][1][0]], 3),
        )
        expected: list[str] = []
        for offset, (kind, core, tail_count) in enumerate(members):
            row_id = f"bench-{index:02d}-{'exp' if kind == 'expected' else 'dis'}-{offset:02d}"
            content = " ".join([*core, *_tails(tail_count, salt=index * 11 + offset)])
            rows.append(
                {
                    "id": row_id,
                    "content": content,
                    "kind": kind,
                    "topic": index,
                    "age_days": 1 + ((position * 37 + seed) % SCORED_AGE_MAX),
                }
            )
            position += 1
            if kind == "expected":
                expected.append(row_id)
        queries.append(
            {
                "id": f"q{index:02d}-{topic}",
                "text": f"{g1} {g2} {t0}",
                "expected": expected,
            }
        )
    filler_needed = size - len(rows)
    if filler_needed < 0:
        raise BenchError(
            f"--size {size} is smaller than the {len(rows)} rows the queries need; "
            f"use --size {len(rows)} or more"
        )
    for offset in range(filler_needed):
        rows.append(
            {
                "id": f"bench-filler-{offset:05d}",
                "content": " ".join(_tails(1 + offset % 3, salt=offset * 7 + 3)),
                "kind": "filler",
                "topic": None,
                "age_days": FILLER_AGE_MIN + ((offset * 53 + seed) % FILLER_AGE_SPAN),
            }
        )
    return rows, queries


def topic_of(tokens):
    """Which topic these tokens belong to; ``None`` when none of them does."""
    best, best_hits = None, 0
    for index, (_name, words) in enumerate(TOPICS):
        hits = len(set(words) & tokens)
        if hits > best_hits:
            best, best_hits = index, hits
    return best


def build_stub(rows, queries):
    """Vocabulary index + dimension count for the deterministic stub embedding.

    Every token that occurs anywhere becomes one dimension (sorted, so the
    index is fixed), and every topic one more dimension carrying TOPIC_MASS.
    The vector of a text is its normalised token counts plus its topic mass:
    identical vocabulary in, identical vector out — no RNG, no API, no model.
    """
    tokens = {term for row in rows for term in service.terms(row["content"])}
    tokens |= {term for query in queries for term in service.terms(query["text"])}
    vocab = sorted(tokens)
    return {term: position for position, term in enumerate(vocab)}, len(vocab) + len(TOPICS)


def embed(tokens, vocab_index, dims, topic_index):
    """Normalised stub vector for ``tokens``; ``None`` when it is empty."""
    vector = [0.0] * dims
    for token in tokens:
        position = vocab_index.get(token)
        if position is not None:
            vector[position] += 1.0
    if topic_index is not None:
        vector[len(vocab_index) + topic_index] = TOPIC_MASS
    norm = math.sqrt(sum(value * value for value in vector))
    if norm <= 0.0:
        return None
    return [round(value / norm, 4) for value in vector]


def query_vector_for(text, mode, vocab_index, dims):
    """§5's query side: always ``None`` in lexical mode, the stub vector in hybrid."""
    if mode != "hybrid":
        return None
    tokens = service.terms(text)
    return embed(tokens, vocab_index, dims, topic_of(tokens))


def run_synthetic_suite(args, progress_log):
    """Build the corpus, run real searches, time them, score them, clean up."""
    from mix_agent.db.models import Memory
    from mix_agent.db.session import SessionLocal
    from sqlalchemy import delete

    rows, queries = build_corpus(args.size, args.seed)
    scored = sum(1 for row in rows if row["kind"] != "filler")
    window = min(len(rows), service.CANDIDATE_LIMIT)
    vocab_index = dims = None
    embedded_ids: set[str] = set()
    if args.mode == "hybrid":
        vocab_index, dims = build_stub(rows, queries)
        # Only rows `search()` can actually consider are worth embedding: a row
        # outside the candidate window is never fetched, so embedding it could
        # not change a single measured number.
        embedded_ids = {row["id"] for row in sorted(rows, key=lambda item: item["age_days"])[:window]}
        progress_log(
            f"corpus: stub embedding ready — vocab {len(vocab_index)} tokens + "
            f"{len(TOPICS)} topic dims = {dims}, carried by {len(embedded_ids)} of the "
            f"{window} rows inside the candidate window"
        )
    progress_log(
        f"corpus: {len(rows)} rows ({scored} scored, {len(rows) - scored} filler), mode={args.mode}"
    )
    vectors = {query["id"]: query_vector_for(query["text"], args.mode, vocab_index, dims)
               for query in queries}

    with SessionLocal() as db:
        db.execute(delete(Memory).where(Memory.owner_id == BENCH_OWNER))
        db.commit()
        batch: list[Memory] = []
        inserted = 0
        embedded = 0

        def flush():
            nonlocal inserted, batch
            db.add_all(batch)
            db.commit()
            inserted += len(batch)
            batch = []
            progress_log(f"corpus: inserted {inserted}/{len(rows)} rows")

        for row in rows:
            data = {
                "content": row["content"],
                "source": "auto",
                "source_run": "retrieval-bench",
                "pinned": False,
            }
            if row["id"] in embedded_ids:
                vector = embed(service.terms(row["content"]), vocab_index, dims, row["topic"])
                if vector is not None:
                    data["embedding"] = vector
                    data["embedding_model"] = STUB_MODEL
                    embedded += 1
            batch.append(
                Memory(
                    id=row["id"],
                    owner_id=BENCH_OWNER,
                    content=row["content"],
                    updated_at=datetime.now(UTC) - timedelta(days=row["age_days"]),
                    data=data,
                    lifecycle_state="established",
                    role="fact",
                    scope="user",
                    source_kind="agent",
                    confidence=0.7,
                    salience=0.5,
                )
            )
            if len(batch) >= 1000:
                flush()
        if batch:
            flush()
        # Drop the inserted instances from the identity map: they are not what
        # search() reads, and keeping 10k of them alive would only cost memory.
        db.expunge_all()

        # One untimed call, so the first measured sample is not measuring a cold
        # planner / an empty relation cache.
        service.search(db, BENCH_OWNER, queries[0]["text"], None, debug=True,
                       query_vector=vectors[queries[0]["id"]])

        latencies: list[float] = []
        returned_by_case: dict[str, list[str]] = {query["id"]: [] for query in queries}
        considered: list[int] = []
        expansion_rows = 0
        ranking_modes: dict[str, int] = {}
        all_ids = {row["id"] for row in rows}
        for repeat in range(args.repeat):
            for query in queries:
                started = time.perf_counter()
                outcome = service.search(
                    db, BENCH_OWNER, query["text"], None, debug=True,
                    query_vector=vectors[query["id"]],
                )
                latencies.append((time.perf_counter() - started) * 1000.0)
                debug = outcome["debug"]
                considered.append(debug["considered"])
                expansion_rows += debug["association_expansion_count"]
                mode = debug["ranking_mode"]
                ranking_modes[mode] = ranking_modes.get(mode, 0) + 1
                if repeat == 0:
                    returned_by_case[query["id"]] = [item["id"] for item in outcome["memories"]]
            progress_log(
                f"search: repeat {repeat + 1}/{args.repeat} done — {len(latencies)} timed "
                f"calls, p50 {percentile(latencies, 0.5):.1f} ms"
            )

        cases = [
            score_case(
                query["id"],
                query["expected"],
                all_ids - set(query["expected"]),
                returned_by_case[query["id"]],
                query=query["text"],
            )
            for query in queries
        ]

        if not args.keep:
            db.execute(delete(Memory).where(Memory.owner_id == BENCH_OWNER))
            db.commit()
            progress_log("corpus: removed the bench rows again (pass --keep to keep them)")

    payload = aggregate(cases, latencies)
    payload["counts"] = {
        "rows": len(rows),
        "scored_rows": scored,
        "filler_rows": len(rows) - scored,
        "embedded_rows": embedded,
        "queries": len(queries),
        "latency_samples": len(latencies),
        "candidates_min": min(considered) if considered else 0,
        "candidates_max": max(considered) if considered else 0,
        "association_rows_added": expansion_rows,
        "ranking_modes": ranking_modes,
        "candidate_limit": service.CANDIDATE_LIMIT,
        "result_limit": service.RESULT_LIMIT_DEFAULT,
        "min_score": service.MIN_SCORE,
    }
    return payload, cases


# --- report -------------------------------------------------------------------
def build_report(args, progress_log):
    started = datetime.now(UTC)
    metrics: dict[str, dict | None] = {}
    cases: dict[str, list] = {}
    counts: dict[str, object] = {}

    if args.suite in ("fixture", "both"):
        payload, case_rows = run_fixture_suite(args.mode, progress_log)
        metrics["fixture"] = payload
        cases["fixture"] = case_rows
        counts["fixture_cases"] = payload["counts"]["cases"]
        counts["fixture_evaluable"] = payload["counts"]["evaluable"]
    if args.suite in ("synthetic", "both"):
        payload, case_rows = run_synthetic_suite(args, progress_log)
        metrics["synthetic"] = payload
        cases["synthetic"] = case_rows
        counts["synthetic_rows"] = payload["counts"]["rows"]
        counts["synthetic_scored_rows"] = payload["counts"]["scored_rows"]
        counts["synthetic_queries"] = payload["counts"]["queries"]
        counts["embedded_rows"] = payload["counts"]["embedded_rows"]
        counts["candidates_min"] = payload["counts"]["candidates_min"]
        counts["candidates_max"] = payload["counts"]["candidates_max"]
        counts["latency_samples"] = payload["counts"]["latency_samples"]

    report = {
        "tool": "experiments/run_retrieval_bench.py",
        "spec": "docs/memory-spec.md §5",
        "generated_at": started.isoformat(timespec="seconds"),
        "environment": {
            "python": platform.python_version(),
            "implementation": platform.python_implementation(),
            "platform": platform.platform(),
        },
        "run": {
            "mode": args.mode,
            "size": args.size,
            "seed": args.seed,
            "suite": args.suite,
            "repeat": args.repeat,
        },
        "counts": counts,
        "metrics": metrics,
        "definitions": DEFINITIONS,
        "cases": cases,
        "thresholds": {
            flag(name): getattr(args, name)
            for name, _metric, _direction in THRESHOLDS
            if getattr(args, name) is not None
        },
    }
    return report


def flag(name):
    """argparse dest (``min_recall5``) → the option the user actually typed."""
    return f"--{name.replace('_', '-')}"


def format_value(metric, value):
    if value is None:
        return "n/a"
    if metric.startswith("latency_"):
        return f"{value:.2f} ms"
    return f"{value:.4f}"


def format_report(report):
    run = report["run"]
    env = report["environment"]
    counts = report["counts"]
    lines = [
        "retrieval bench — spec §5 ranking (docker edition)",
        f"python {env['python']} ({env['implementation']}) on {env['platform']}",
        (
            f"mode={run['mode']} · size={run['size']} · seed={run['seed']} · "
            f"suite={run['suite']} · repeat={run['repeat']}"
        ),
    ]
    row_bits = [f"fixture cases {counts.get('fixture_cases', 0)}"]
    if "synthetic_rows" in counts:
        row_bits.append(f"synthetic rows {counts['synthetic_rows']}")
        row_bits.append(f"scored {counts['synthetic_scored_rows']}")
        row_bits.append(f"embedded {counts['embedded_rows']}")
        row_bits.append(f"candidates {counts['candidates_min']}-{counts['candidates_max']}")
        row_bits.append(f"queries {counts['synthetic_queries']}")
        row_bits.append(f"search() samples {counts['latency_samples']}")
    lines.append("counts: " + " · ".join(row_bits))
    lines.append("")
    lines.append(f"{'suite':<10} {'metric':<16} {'value':>10}  basis")
    lines.append(f"{'-' * 10} {'-' * 16} {'-' * 10}  {'-' * 34}")
    for suite, payload in report["metrics"].items():
        if not payload:
            continue
        for metric, value in payload["values"].items():
            lines.append(
                f"{suite:<10} {metric:<16} {format_value(metric, value):>10}  "
                f"{payload['basis'][metric]}"
            )
    lines.append("")
    lines.append("definitions:")
    for metric, text in DEFINITIONS.items():
        lines.append(f"  {metric:<16} {text}")
    thresholds = report["thresholds"]
    lines.append("")
    lines.append(
        "thresholds: "
        + (", ".join(f"{k} {v}" for k, v in thresholds.items()) if thresholds else "none (measurement only)")
    )
    result = report.get("threshold_result")
    if result:
        lines.append(
            f"threshold result: {result['status']} "
            f"(suites checked: {', '.join(result['checked_suites']) or 'none'})"
        )
        for line in result["violations"]:
            lines.append(f"  violation  {line}")
        for line in result["not_measured"]:
            lines.append(f"  not measured  {line}")
    return "\n".join(lines)


def check_thresholds(report, args):
    """The only thing that may fail this bench: an explicitly given threshold.

    Every suite that ran is checked, so gating one suite means running one
    suite (``--suite synthetic``). Metrics a suite cannot express are skipped
    for it rather than counted as unmeasurable.
    """
    given = {}
    violations: list[str] = []
    unmeasured: list[str] = []
    checked = [suite for suite, payload in report["metrics"].items() if payload]
    for name, metric, direction in THRESHOLDS:
        limit = getattr(args, name)
        if limit is None:
            continue
        given[flag(name)] = limit
        applicable = [suite for suite in checked if metric in SUITE_METRICS.get(suite, ())]
        if not applicable:
            unmeasured.append(f"{metric} (none of the {checked} suites produces it)")
            continue
        for suite in applicable:
            value = report["metrics"][suite]["values"].get(metric)
            if value is None:
                unmeasured.append(f"{suite}.{metric}")
                continue
            bad = value < limit if direction == "min" else value > limit
            if bad:
                operator = ">=" if direction == "min" else "<="
                violations.append(f"{suite}.{metric}={value} fails {operator} {limit}")
    report["threshold_result"] = {
        "status": "regression" if violations else ("not-measured" if unmeasured else "ok"),
        "given": given,
        "checked_suites": checked,
        "violations": violations,
        "not_measured": unmeasured,
    }
    if violations:
        return EXIT_REGRESSION
    if unmeasured:
        return EXIT_ERROR
    return EXIT_OK


def emit_threshold_result(result):
    """Report a threshold verdict on stderr — stdout belongs to the report."""
    for line in result["violations"]:
        print(f"bench: REGRESSION {line}", file=sys.stderr, flush=True)
    for line in result["not_measured"]:
        print(f"bench: cannot measure {line} for the threshold given", file=sys.stderr, flush=True)
    if result["given"] and result["status"] == "ok":
        print(f"bench: thresholds held ({', '.join(result['given'])})", file=sys.stderr, flush=True)


def parse_args(argv=None):
    parser = argparse.ArgumentParser(
        description="Measure spec §5 retrieval quality (fixture + synthetic corpus). "
        "Exits 0 when it measured something, whatever the numbers are.",
    )
    parser.add_argument("--size", type=int, default=1000, help="synthetic corpus rows (default 1000)")
    parser.add_argument("--mode", choices=("lexical", "hybrid"), default="lexical",
                        help="lexical = no embeddings (0.85/0.15), hybrid = stub vector term")
    parser.add_argument("--seed", type=int, default=20261010, help="corpus seed (default 20261010)")
    parser.add_argument("--suite", choices=("fixture", "synthetic", "both"), default="both",
                        help="fixture needs no database; synthetic needs PostgreSQL")
    parser.add_argument("--repeat", type=int, default=3,
                        help="timed search() passes over every query (default 3)")
    parser.add_argument("--json", metavar="PATH", help="write the machine-readable report here")
    parser.add_argument("--keep", action="store_true", help="keep the synthetic rows after the run")
    parser.add_argument("--min-recall5", type=float, metavar="FLOAT",
                        help="exit 1 if recall@5 falls below this")
    parser.add_argument("--min-recall8", type=float, metavar="FLOAT",
                        help="exit 1 if recall@8 falls below this")
    parser.add_argument("--min-mrr", type=float, metavar="FLOAT",
                        help="exit 1 if MRR falls below this")
    parser.add_argument("--max-noise", type=float, metavar="FLOAT",
                        help="exit 1 if the noise rate rises above this")
    parser.add_argument("--max-p95-ms", type=float, metavar="FLOAT",
                        help="exit 1 if search() p95 rises above this (synthetic only)")
    args = parser.parse_args(argv)
    if args.size < 1:
        parser.error("--size must be >= 1")
    if args.repeat < 1:
        parser.error("--repeat must be >= 1")
    return args


def main(argv=None) -> int:
    args = parse_args(argv)
    try:
        report = build_report(args, progress)
        status = check_thresholds(report, args)
        print(format_report(report))
        emit_threshold_result(report["threshold_result"])
        if args.json:
            path = Path(args.json)
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
            progress(f"report written to {path}")
        return status
    except BenchError as exc:
        print(f"bench error: {exc}", file=sys.stderr, flush=True)
        return EXIT_ERROR
    except Exception as exc:  # noqa: BLE001 — a broken measurement must not look like a pass
        print(f"bench error: {type(exc).__name__}: {exc}", file=sys.stderr, flush=True)
        return EXIT_ERROR


if __name__ == "__main__":
    sys.exit(main())
