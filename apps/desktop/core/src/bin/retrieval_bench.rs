//! Retrieval-quality bench for `desktop/docs/memory-spec.md` §5 (Desktop edition).
//!
//! This is the Rust counterpart of the Docker edition's
//! `experiments/run_retrieval_bench.py`. The two share the *metric
//! definitions* — recall@5 / recall@8, MRR, noise rate, latency p50/p95 mean
//! the same thing in both — while each edition runs its own retrieval code
//! over its own corpus plumbing, so the numbers are comparable in meaning,
//! not expected to be equal.
//!
//! Two suites:
//!
//! fixture
//!     Reads `tests/fixtures/memory_recall_cases.json` through
//!     `mixd_core::recall_fixture` — the same module `tests/recall_fixture.rs`
//!     asserts through, so the bench and the ranking contract can never drift
//!     apart. Pure: no database, no network.
//!
//! synthetic
//!     Builds a deterministic corpus of `--size` rows (seeded; the only
//!     randomness is arithmetic on the seed) in a temporary SQLite store and
//!     scores real `retrieve_memories()` / `retrieve_memories_hybrid()` calls,
//!     timing each one. `--size 10000` gives the p50/p95 of a 10k store.
//!
//! Modes:
//!
//! lexical (default)
//!     No embeddings anywhere, so §5.2's "embedding unavailable" weights
//!     (0.85 / 0.15) decide — the pre-embedding ranking.
//!
//! `--mode hybrid`
//!     A deterministic, vocabulary-derived stub embedding goes through §5's
//!     `vector` term. No external API, no model download, no randomness: the
//!     same vocabulary always produces the same vector.
//!
//! This program is an *observation device*. It never changes ranking,
//! extraction or linking behaviour, and by default it only measures: the exit
//! code turns non-zero solely when an explicitly supplied threshold is
//! violated (1) or when a supplied threshold cannot be measured at all (2).
//!
//! Run it directly (no test harness; a 10k measurement has no business inside
//! `cargo test --workspace`):
//!
//!     cargo run -p mixd-core --bin retrieval-bench -- --size 1000 --mode lexical
//!
//! Progress goes to stderr, the human report to stdout, `--json <path>` writes
//! the machine-readable twin of the same report.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use chrono::{Duration, Utc};
use mixd_core::recall_fixture;
use mixd_core::retrieval::{
    retrieve_memories, retrieve_memories_debug, retrieve_memories_hybrid, EmbeddingSource,
    MIN_SCORE, RESULT_LIMIT_DEFAULT,
};
use mixd_providers::{Provider, ProviderCapabilities, ProviderId, ProviderRequest, StreamBox};
use mixd_storage::Storage;
use serde::Serialize;
use serde_json::json;

const EXIT_OK: i32 = 0;
const EXIT_REGRESSION: i32 = 1;
const EXIT_ERROR: i32 = 2;

// §5.2 / §5.1, stated so the report carries the contract it measures against.
// Definitions kept in the same words as the Docker edition's bench: a number
// here has to mean the same thing there.
const DEFINITIONS: [(&str, &str); 6] = [
    (
        "recall_at_5",
        "mean over cases with a non-empty expected set of \
         |expected ∩ returned[:5]| / |expected|",
    ),
    (
        "recall_at_8",
        "mean over cases with a non-empty expected set of \
         |expected ∩ returned[:8]| / |expected|",
    ),
    (
        "mrr",
        "mean over the same cases of 1 / rank of the first expected row in the \
         returned order; 0 when a case returns none of its expected rows",
    ),
    (
        "noise_rate",
        "mean over cases that returned anything of |returned ∩ must_not_match| / \
         |returned|; must_not_match is every candidate the case does not expect \
         (fixture: its candidates minus expected, synthetic: the corpus minus expected)",
    ),
    (
        "latency_p50_ms",
        "nearest-rank p50 of one retrieve_memories() / retrieve_memories_hybrid() \
         call, wall clock",
    ),
    (
        "latency_p95_ms",
        "nearest-rank p95 of one retrieve_memories() / retrieve_memories_hybrid() \
         call, wall clock",
    ),
];

/// flag, metric, direction — the only things that can fail this bench, and
/// only when the user explicitly asks for them.
const THRESHOLDS: [(&str, &str, &str); 5] = [
    ("min-recall5", "recall_at_5", "min"),
    ("min-recall8", "recall_at_8", "min"),
    ("min-mrr", "mrr", "min"),
    ("max-noise", "noise_rate", "max"),
    ("max-p95-ms", "latency_p95_ms", "max"),
];

// ---------------------------------------------------------------------------
// Synthetic corpus — the same design the Docker edition's bench generates, so
// the two corpora argue about the same things.
//
// Eight topics, three topic words each, all distinct and all latin tokens, so
// the bench's own tokeniser yields exactly the tokens the generator put in the
// text and |Q ∩ T| / |Q| is arithmetic rather than a guess.
// ---------------------------------------------------------------------------
const TOPICS: [(&str, [&str; 3]); 8] = [
    ("database", ["postgres", "schema", "migration"]),
    ("frontend", ["react", "render", "component"]),
    ("network", ["proxy", "timeout", "retry"]),
    ("security", ["token", "session", "audit"]),
    ("storage", ["backup", "restore", "archive"]),
    ("billing", ["invoice", "refund", "quota"]),
    ("retrieval", ["index", "analyzer", "ranking"]),
    ("deploy", ["release", "rollback", "canary"]),
];

/// Two per topic (16 = 2 * len(TOPICS)): a query is "two shared words + one
/// topic word", which is what lets a *different* topic still match 2/3 of it.
const GENERIC: [&str; 16] = [
    "config", "error", "setup", "request", "result", "cache", "limit", "report", "status",
    "record", "process", "value", "field", "option", "detail", "event",
];

/// Neutral words for document tails and filler: disjoint from every query
/// term, so filler can only ever rank on recency.
const TAILS: [&str; 32] = [
    "notebook", "summary", "draft", "agenda", "remind", "review", "sample", "lesson", "budget",
    "travel", "recipe", "weather", "garden", "music", "camera", "window", "bottle", "ticket",
    "pocket", "letter", "bridge", "forest", "island", "river", "mountain", "valley", "harbor",
    "desert", "canyon", "meadow", "cabin", "village",
];

/// How much a shared topic outweighs shared tokens in the stub embedding. A
/// documented constant, not a tuned one — it only has to be big enough for the
/// vector term to express "same topic" at all.
const TOPIC_MASS: f64 = 4.0;
/// The stub's model id: rows and query vectors only compare inside one model
/// family (spec §2.2), so both sides name this one.
const STUB_MODEL: &str = "bench-bow-v1";

const SCORED_AGE_MAX: i64 = 150; // expected/distractor rows, days
const FILLER_AGE_MIN: i64 = 180; // filler rows, days: always older than any scored row
const FILLER_AGE_SPAN: i64 = 720;
/// 8 topics * 12 members: the rows the queries need before filler starts.
const SCORED_ROWS: usize = TOPICS.len() * 12;
const DEFAULT_SIZE: usize = 1000;
const DEFAULT_SEED: i64 = 20_261_010;
const DEFAULT_REPEAT: usize = 3;

const USAGE: &str = "\
retrieval-bench — measure spec §5 retrieval quality (fixture + synthetic corpus)

usage: cargo run -p mixd-core --bin retrieval-bench -- [options]

options:
  --size N          synthetic corpus rows (default 1000; minimum 96)
  --mode MODE       lexical (default) | hybrid
  --seed N          corpus seed (default 20261010)
  --suite SUITE     fixture | synthetic | both (default both)
  --repeat N        timed recall passes over every query (default 3)
  --json PATH       write the machine-readable report here
  --keep            keep the temporary SQLite store after the run
  --min-recall5 F   exit 1 if recall@5 falls below this
  --min-recall8 F   exit 1 if recall@8 falls below this
  --min-mrr F       exit 1 if MRR falls below this
  --max-noise F     exit 1 if the noise rate rises above this
  --max-p95-ms F    exit 1 if recall p95 rises above this (synthetic only)
  -h, --help        show this help

Exits 0 when it measured something, whatever the numbers are.";

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Lexical,
    Hybrid,
}

impl Mode {
    fn as_str(self) -> &'static str {
        match self {
            Mode::Lexical => "lexical",
            Mode::Hybrid => "hybrid",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "lexical" => Ok(Mode::Lexical),
            "hybrid" => Ok(Mode::Hybrid),
            other => Err(format!("--mode must be lexical or hybrid, got {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Suite {
    Fixture,
    Synthetic,
    Both,
}

impl Suite {
    fn as_str(self) -> &'static str {
        match self {
            Suite::Fixture => "fixture",
            Suite::Synthetic => "synthetic",
            Suite::Both => "both",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "fixture" => Ok(Suite::Fixture),
            "synthetic" => Ok(Suite::Synthetic),
            "both" => Ok(Suite::Both),
            other => Err(format!(
                "--suite must be fixture, synthetic or both, got {other}"
            )),
        }
    }

    fn includes_fixture(self) -> bool {
        matches!(self, Suite::Fixture | Suite::Both)
    }

    fn includes_synthetic(self) -> bool {
        matches!(self, Suite::Synthetic | Suite::Both)
    }
}

#[derive(Debug)]
struct Args {
    size: usize,
    mode: Mode,
    seed: i64,
    suite: Suite,
    repeat: usize,
    json: Option<PathBuf>,
    keep: bool,
    thresholds: BTreeMap<String, f64>,
    help: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            size: DEFAULT_SIZE,
            mode: Mode::Lexical,
            seed: DEFAULT_SEED,
            suite: Suite::Both,
            repeat: DEFAULT_REPEAT,
            json: None,
            keep: false,
            thresholds: BTreeMap::new(),
            help: false,
        }
    }
}

fn next_value<I: Iterator<Item = String>>(argv: &mut I, flag: &str) -> Result<String, String> {
    argv.next().ok_or_else(|| format!("{flag} needs a value"))
}

impl Args {
    fn parse<I: Iterator<Item = String>>(mut argv: I) -> Result<Self, String> {
        let mut args = Self::default();
        while let Some(flag) = argv.next() {
            match flag.as_str() {
                "--size" => {
                    let value = next_value(&mut argv, &flag)?;
                    args.size = value
                        .parse()
                        .map_err(|_| format!("--size needs a whole number, got {value}"))?;
                }
                "--mode" => {
                    let value = next_value(&mut argv, &flag)?;
                    args.mode = Mode::parse(&value)?;
                }
                "--seed" => {
                    let value = next_value(&mut argv, &flag)?;
                    args.seed = value
                        .parse()
                        .map_err(|_| format!("--seed needs a whole number, got {value}"))?;
                }
                "--suite" => {
                    let value = next_value(&mut argv, &flag)?;
                    args.suite = Suite::parse(&value)?;
                }
                "--repeat" => {
                    let value = next_value(&mut argv, &flag)?;
                    args.repeat = value
                        .parse()
                        .map_err(|_| format!("--repeat needs a whole number, got {value}"))?;
                }
                "--json" => {
                    args.json = Some(PathBuf::from(next_value(&mut argv, &flag)?));
                }
                "--keep" => args.keep = true,
                "--help" | "-h" => args.help = true,
                other => {
                    if let Some((name, _, _)) = THRESHOLDS
                        .iter()
                        .find(|(name, _, _)| Some(*name) == other.strip_prefix("--"))
                    {
                        let value = next_value(&mut argv, other)?;
                        let parsed: f64 = value
                            .parse()
                            .map_err(|_| format!("{other} needs a number, got {value}"))?;
                        args.thresholds.insert((*name).to_string(), parsed);
                    } else {
                        return Err(format!("unknown option {other}"));
                    }
                }
            }
        }
        if args.size < SCORED_ROWS {
            return Err(format!(
                "--size {} is smaller than the {SCORED_ROWS} rows the queries need; \
                 use --size {SCORED_ROWS} or more",
                args.size
            ));
        }
        if args.repeat < 1 {
            return Err("--repeat must be >= 1".to_string());
        }
        Ok(args)
    }
}

// ---------------------------------------------------------------------------
// Metrics — the Docker edition's definitions, spelled in Rust.
// ---------------------------------------------------------------------------

/// One case scored against §5's four quality metrics. Never divides by zero:
/// a case without an expected set reports `None` for recall/MRR (undefined,
/// not zero), and a case that returned nothing reports `None` for noise.
#[derive(Debug, Serialize)]
struct CaseScore {
    case: String,
    expected: usize,
    returned: usize,
    must_not_match: usize,
    hits_at_5: Option<usize>,
    hits_at_8: Option<usize>,
    first_hit_rank: Option<usize>,
    noise_hits: usize,
    recall_at_5: Option<f64>,
    recall_at_8: Option<f64>,
    mrr: Option<f64>,
    noise_rate: Option<f64>,
    query: String,
}

fn score_case(
    case_id: &str,
    expected: &HashSet<String>,
    must_not: &HashSet<String>,
    returned: &[String],
    query: &str,
) -> CaseScore {
    let evaluable = !expected.is_empty();
    let count = returned.len();
    // A hit is a distinct expected row; §4 expansions cannot double-count.
    let mut seen: HashSet<&String> = HashSet::new();
    let hits_at_5 = returned
        .iter()
        .take(5)
        .filter(|id| expected.contains(*id) && seen.insert(*id))
        .count();
    let mut seen: HashSet<&String> = HashSet::new();
    let hits_at_8 = returned
        .iter()
        .take(8)
        .filter(|id| expected.contains(*id) && seen.insert(*id))
        .count();
    let first_hit_rank = returned
        .iter()
        .position(|id| expected.contains(id))
        .map(|index| index + 1);
    let noise_hits = returned.iter().filter(|id| must_not.contains(*id)).count();
    CaseScore {
        case: case_id.to_string(),
        expected: expected.len(),
        returned: count,
        must_not_match: must_not.len(),
        hits_at_5: evaluable.then_some(hits_at_5),
        hits_at_8: evaluable.then_some(hits_at_8),
        first_hit_rank,
        noise_hits,
        recall_at_5: evaluable.then(|| hits_at_5 as f64 / expected.len() as f64),
        recall_at_8: evaluable.then(|| hits_at_8 as f64 / expected.len() as f64),
        mrr: match first_hit_rank {
            Some(rank) => Some(1.0 / rank as f64),
            None if evaluable => Some(0.0),
            None => None,
        },
        noise_rate: (count > 0).then(|| noise_hits as f64 / count as f64),
        query: query.to_string(),
    }
}

/// Nearest-rank percentile; `None` when there is nothing to measure.
fn percentile(values: &[f64], fraction: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut ordered = values.to_vec();
    ordered.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    let rank = ((fraction * ordered.len() as f64).ceil() as usize).max(1);
    ordered.get(rank - 1).copied()
}

fn round(value: f64, digits: usize) -> f64 {
    let factor = 10f64.powi(digits as i32);
    (value * factor).round() / factor
}

fn mean(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    Some(round(values.iter().sum::<f64>() / values.len() as f64, 4))
}

/// A quality metric's name and its per-case extractor.
type QualityMetric = (&'static str, fn(&CaseScore) -> Option<f64>);

/// Mean each metric across the cases that can express it, with its basis.
fn aggregate(
    cases: &[CaseScore],
    latencies: &[f64],
) -> (BTreeMap<String, Option<f64>>, BTreeMap<String, String>) {
    let total = cases.len();
    let mut values = BTreeMap::new();
    let mut basis = BTreeMap::new();
    let quality: [QualityMetric; 3] = [
        ("recall_at_5", |case| case.recall_at_5),
        ("recall_at_8", |case| case.recall_at_8),
        ("mrr", |case| case.mrr),
    ];
    for (metric, pick) in quality {
        let picked: Vec<f64> = cases.iter().filter_map(pick).collect();
        values.insert(metric.to_string(), mean(&picked));
        basis.insert(
            metric.to_string(),
            format!("{}/{} evaluable cases", picked.len(), total),
        );
    }
    let picked: Vec<f64> = cases.iter().filter_map(|case| case.noise_rate).collect();
    values.insert("noise_rate".to_string(), mean(&picked));
    basis.insert(
        "noise_rate".to_string(),
        format!("{}/{} cases with results", picked.len(), total),
    );
    for (metric, fraction) in [("latency_p50_ms", 0.5), ("latency_p95_ms", 0.95)] {
        let point = percentile(latencies, fraction);
        values.insert(metric.to_string(), point.map(|value| round(value, 2)));
        basis.insert(
            metric.to_string(),
            format!("{} timed recall samples", latencies.len()),
        );
    }
    (values, basis)
}

#[derive(Debug, Serialize)]
struct SuitePayload {
    values: BTreeMap<String, Option<f64>>,
    basis: BTreeMap<String, String>,
    counts: BTreeMap<String, serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Synthetic corpus
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RowKind {
    Expected,
    Distractor,
    Filler,
}

impl RowKind {
    fn id_tag(self) -> &'static str {
        match self {
            RowKind::Expected => "exp",
            RowKind::Distractor | RowKind::Filler => "dis",
        }
    }
}

#[derive(Debug, Clone)]
struct CorpusRow {
    id: String,
    kind: RowKind,
    content: String,
    age_days: i64,
    /// Which topic's mass this row's stub vector carries — `None` for filler.
    topic: Option<usize>,
    embedding: Option<Vec<f64>>,
}

#[derive(Debug, Clone)]
struct CorpusQuery {
    id: String,
    text: String,
    expected: Vec<String>,
}

/// Deterministic tail words: same count + salt always gives the same words.
fn tails(count: usize, salt: usize) -> impl Iterator<Item = &'static str> {
    (0..count).map(move |step| TAILS[(salt + step * 5) % TAILS.len()])
}

/// Rows that answer the bench's queries, plus filler up to `size`.
///
/// Ages decide what is measurable: expected and distractor rows share one
/// 1..150 day band, filler sits at 180..899 days, so a query's own candidates
/// always outrank filler on recency alone while the filler still has to be
/// scanned and sorted — which is exactly what the latency numbers are about.
///
/// `seed` shifts that recency assignment (nothing else): content, ids and
/// overlap are index-derived, so one seed means one corpus and a second seed
/// means the same corpus with a different — still deterministic — draw of
/// ages.
fn build_corpus(size: usize, seed: i64) -> anyhow::Result<(Vec<CorpusRow>, Vec<CorpusQuery>)> {
    if size < SCORED_ROWS {
        anyhow::bail!(
            "--size {size} is smaller than the {SCORED_ROWS} rows the queries need; \
             use --size {SCORED_ROWS} or more"
        );
    }
    let mut rows: Vec<CorpusRow> = Vec::new();
    let mut queries: Vec<CorpusQuery> = Vec::new();
    let mut position: i64 = 0;
    for (index, (topic, words)) in TOPICS.iter().enumerate() {
        let g1 = GENERIC[2 * index];
        let g2 = GENERIC[2 * index + 1];
        let (t0, t1, t2) = (words[0], words[1], words[2]);
        let other = (index + 3) % TOPICS.len();
        let third = (index + 5) % TOPICS.len();
        // (kind, core tokens, tail words). Expected rows span 3/3 → 1/3 of the
        // query; distractors are a cross-topic lexical decoy (2/3), two
        // single-word partials, and a same-topic but lexically disjoint row
        // (0/3). Both directions of "lexical says yes / topic says no" and
        // vice versa are in here on purpose: that is the argument between
        // lexical and hybrid.
        let members: [(RowKind, Vec<&'static str>, usize); 12] = [
            (RowKind::Expected, vec![g1, g2, t0], 0),
            (RowKind::Expected, vec![g1, g2, t0], 9),
            (RowKind::Expected, vec![g1, t0], 3),
            (RowKind::Expected, vec![g2, t0], 5),
            (RowKind::Expected, vec![t0], 2),
            (
                RowKind::Distractor,
                vec![g1, g2, TOPICS[other].1[0], TOPICS[other].1[1]],
                4,
            ),
            (
                RowKind::Distractor,
                vec![g1, g2, TOPICS[third].1[1], TOPICS[third].1[2]],
                6,
            ),
            (RowKind::Distractor, vec![t1, t2], 1),
            (RowKind::Distractor, vec![t1, t2], 7),
            (RowKind::Distractor, vec![g1], 4),
            (RowKind::Distractor, vec![g2], 6),
            (RowKind::Distractor, vec![TOPICS[other].1[0]], 3),
        ];
        let mut expected: Vec<String> = Vec::new();
        for (offset, (kind, core, tail_count)) in members.into_iter().enumerate() {
            let id = format!("bench-{index:02}-{}-{offset:02}", kind.id_tag());
            let content = core
                .iter()
                .copied()
                .chain(tails(tail_count, index * 11 + offset))
                .collect::<Vec<_>>()
                .join(" ");
            let age_days = 1 + (position * 37 + seed).rem_euclid(SCORED_AGE_MAX);
            rows.push(CorpusRow {
                id: id.clone(),
                kind,
                content,
                age_days,
                topic: Some(index),
                embedding: None,
            });
            position += 1;
            if kind == RowKind::Expected {
                expected.push(id);
            }
        }
        queries.push(CorpusQuery {
            id: format!("q{index:02}-{topic}"),
            text: format!("{g1} {g2} {t0}"),
            expected,
        });
    }
    let filler_needed = size - rows.len();
    for offset in 0..filler_needed {
        let content = tails(1 + offset % 3, offset * 7 + 3)
            .collect::<Vec<_>>()
            .join(" ");
        rows.push(CorpusRow {
            id: format!("bench-filler-{offset:05}"),
            kind: RowKind::Filler,
            content,
            age_days: FILLER_AGE_MIN + (offset as i64 * 53 + seed).rem_euclid(FILLER_AGE_SPAN),
            topic: None,
            embedding: None,
        });
    }
    Ok((rows, queries))
}

/// The bench's own tokeniser: lowercase, split on anything non-alphanumeric.
///
/// The corpus is latin tokens by construction, so this yields exactly the
/// words the generator put in the text — which is what makes the stub vector
/// a function of the vocabulary rather than of a splitting guess.
fn bench_tokens(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .collect()
}

/// Which topic these tokens belong to; `None` when none of them does.
fn topic_of(tokens: &[String]) -> Option<usize> {
    let mut best: Option<usize> = None;
    let mut best_hits = 0usize;
    for (index, (_name, words)) in TOPICS.iter().enumerate() {
        let hits = words
            .iter()
            .filter(|word| tokens.iter().any(|token| token == *word))
            .count();
        if hits > best_hits {
            best = Some(index);
            best_hits = hits;
        }
    }
    best
}

/// Vocabulary index + dimension count for the deterministic stub embedding.
///
/// Every token that occurs anywhere becomes one dimension (sorted, so the
/// index is fixed), and every topic one more dimension carrying
/// [`TOPIC_MASS`]. The vector of a text is its normalised token counts plus
/// its topic mass: identical vocabulary in, identical vector out — no RNG, no
/// API, no model.
struct Stub {
    vocab: HashMap<String, usize>,
    dims: usize,
}

impl Stub {
    fn new(rows: &[CorpusRow], queries: &[CorpusQuery]) -> Self {
        let mut terms: BTreeSet<String> = BTreeSet::new();
        for row in rows {
            terms.extend(bench_tokens(&row.content));
        }
        for query in queries {
            terms.extend(bench_tokens(&query.text));
        }
        let vocab: HashMap<String, usize> = terms
            .into_iter()
            .enumerate()
            .map(|(position, term)| (term, position))
            .collect();
        let dims = vocab.len() + TOPICS.len();
        Stub { vocab, dims }
    }

    /// Normalised stub vector for `tokens`; `None` when it is empty.
    fn embed_tokens(&self, tokens: &[String], topic: Option<usize>) -> Option<Vec<f64>> {
        let mut vector = vec![0.0; self.dims];
        for token in tokens {
            if let Some(position) = self.vocab.get(token) {
                vector[*position] += 1.0;
            }
        }
        if let Some(topic) = topic {
            vector[self.vocab.len() + topic] = TOPIC_MASS;
        }
        let norm = vector.iter().map(|value| value * value).sum::<f64>().sqrt();
        if norm <= 0.0 {
            return None;
        }
        Some(
            vector
                .into_iter()
                .map(|value| (value / norm * 10_000.0).round() / 10_000.0)
                .collect(),
        )
    }

    /// The query side: tokens plus the topic the tokens themselves name.
    fn embed_text(&self, text: &str) -> Option<Vec<f64>> {
        let tokens = bench_tokens(text);
        self.embed_tokens(&tokens, topic_of(&tokens))
    }
}

/// The provider half of the stub: `EmbeddingSource` fetches the *query's*
/// vector through the real provider trait, so hybrid mode exercises §5's
/// production path end to end while the embedding itself stays a pure
/// function of the vocabulary.
struct StubEmbedder {
    name: String,
    stub: Arc<Stub>,
}

#[async_trait]
impl Provider for StubEmbedder {
    fn id(&self) -> &ProviderId {
        &self.name
    }

    fn label(&self) -> &str {
        "retrieval bench stub embedder"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            vision: false,
            tool_calling: false,
            reasoning: false,
            streaming: false,
        }
    }

    async fn stream(&self, _req: ProviderRequest) -> anyhow::Result<StreamBox> {
        anyhow::bail!("the retrieval bench never streams")
    }

    async fn embed(&self, _model: &str, texts: &[String]) -> anyhow::Result<Vec<Vec<f64>>> {
        texts
            .iter()
            .map(|text| {
                self.stub.embed_text(text).ok_or_else(|| {
                    anyhow::anyhow!("the stub has no vector for an empty token stream")
                })
            })
            .collect()
    }
}

fn progress(message: &str) {
    eprintln!("bench: {message}");
}

/// Insert the corpus in batches so a 10k build reports rather than sits
/// silent. Returns how many rows carry an embedding.
async fn insert_rows(storage: &Storage, rows: &[CorpusRow], batch: usize) -> anyhow::Result<usize> {
    let mut tx = storage.pool().begin().await?;
    let mut inserted = 0usize;
    let mut embedded = 0usize;
    for row in rows {
        let mut data = json!({
            "title": "bench",
            "content": row.content,
            "source": "retrieval-bench",
        });
        if let Some(vector) = &row.embedding {
            data["embedding"] = json!(vector);
            data["embedding_model"] = json!(STUB_MODEL);
            embedded += 1;
        }
        let updated_at = (Utc::now() - Duration::days(row.age_days)).to_rfc3339();
        sqlx::query(
            "INSERT INTO memories (id, owner_id, data, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&row.id)
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(data.to_string())
        .bind(&updated_at)
        .bind(&updated_at)
        .execute(&mut *tx)
        .await?;
        inserted += 1;
        if inserted % batch == 0 {
            tx.commit().await?;
            progress(&format!("corpus: inserted {inserted}/{} rows", rows.len()));
            tx = storage.pool().begin().await?;
        }
    }
    tx.commit().await?;
    progress(&format!("corpus: inserted {inserted}/{} rows", rows.len()));
    Ok(embedded)
}

// ---------------------------------------------------------------------------
// Suites
// ---------------------------------------------------------------------------

/// Score the shared fixture through the ranking contract's own reader.
fn run_fixture_suite(mode: Mode) -> anyhow::Result<(SuitePayload, Vec<CaseScore>)> {
    let raw = recall_fixture::load_value()?;
    let cases = raw["cases"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("fixture has no cases array"))?
        .clone();
    progress(&format!(
        "fixture: {} cases from the shared recall fixture ({})",
        cases.len(),
        mode.as_str()
    ));
    let now = Utc::now();
    let mut scored_cases: Vec<CaseScore> = Vec::new();
    for case in &cases {
        let id = case["id"].as_str().unwrap_or("?");
        // lexical = no query vector at all, so every pair takes §5.2's
        // without-vector weights; hybrid passes the case's own vector exactly
        // like tests/recall_fixture.rs does.
        let query_vector = match mode {
            Mode::Lexical => None,
            Mode::Hybrid => recall_fixture::optional_vector(&case["query_vector"])?,
        };
        let ranked = recall_fixture::rank(case, query_vector.as_deref(), now, RESULT_LIMIT_DEFAULT);
        let returned: Vec<String> = ranked.iter().map(|item| item.id.clone()).collect();
        let expected: HashSet<String> = case["expected"]
            .as_str()
            .map(|winner| HashSet::from([winner.to_string()]))
            .unwrap_or_default();
        let candidates: HashSet<String> =
            recall_fixture::candidate_ids(case)?.into_iter().collect();
        let must_not: HashSet<String> = candidates.difference(&expected).cloned().collect();
        let query = recall_fixture::terms(&case["query_terms"])?.join(" ");
        let score = score_case(id, &expected, &must_not, &returned, &query);
        progress(&format!(
            "fixture: {id}: expected {} · returned {} · first hit rank {:?}",
            score.expected, score.returned, score.first_hit_rank
        ));
        scored_cases.push(score);
    }
    let (values, basis) = aggregate(&scored_cases, &[]);
    let mut counts = BTreeMap::new();
    counts.insert("cases".to_string(), json!(scored_cases.len()));
    counts.insert(
        "evaluable".to_string(),
        json!(scored_cases
            .iter()
            .filter(|case| case.recall_at_5.is_some())
            .count()),
    );
    counts.insert(
        "returned_rows".to_string(),
        json!(scored_cases.iter().map(|case| case.returned).sum::<usize>()),
    );
    Ok((
        SuitePayload {
            values,
            basis,
            counts,
        },
        scored_cases,
    ))
}

/// Build the corpus, run real recalls, time them, score them, clean up.
async fn run_synthetic_suite(args: &Args) -> anyhow::Result<(SuitePayload, Vec<CaseScore>)> {
    let (mut rows, queries) = build_corpus(args.size, args.seed)?;
    let scored_rows = rows
        .iter()
        .filter(|row| row.kind != RowKind::Filler)
        .count();

    // §5's vector term, only in hybrid mode: a deterministic stub embedding
    // derived from the corpus vocabulary, stored on the rows like a backfill
    // would and offered for the query through the real provider trait.
    let stub = if args.mode == Mode::Hybrid {
        let stub = Arc::new(Stub::new(&rows, &queries));
        for row in &mut rows {
            let tokens = bench_tokens(&row.content);
            row.embedding = stub.embed_tokens(&tokens, row.topic);
        }
        progress(&format!(
            "corpus: stub embedding ready — vocab {} tokens + {} topic dims = {} dimensions",
            stub.vocab.len(),
            TOPICS.len(),
            stub.dims
        ));
        Some(stub)
    } else {
        None
    };

    let dir = std::env::temp_dir().join(format!("mixd-retrieval-bench-{}", uuid::Uuid::new_v4()));
    let storage = Storage::open(&dir).await?;
    progress(&format!(
        "corpus: {} rows ({} scored, {} filler), mode={}, store {}",
        rows.len(),
        scored_rows,
        rows.len() - scored_rows,
        args.mode.as_str(),
        dir.display()
    ));
    let embedded = insert_rows(&storage, &rows, 1000).await?;

    let source = stub.clone().map(|stub| {
        let embedder = StubEmbedder {
            name: "retrieval-bench".to_string(),
            stub,
        };
        let provider: Arc<dyn Provider> = Arc::new(embedder);
        EmbeddingSource::new(provider, STUB_MODEL.to_string())
    });

    // One untimed recall, so the first measured sample is not measuring a cold
    // page cache. `activate = false` (spec §3's probe form) so the warm-up
    // cannot rewrite the recency the measurements are about.
    retrieve_memories_debug(&storage, &queries[0].text, RESULT_LIMIT_DEFAULT, false).await?;
    progress(&format!(
        "search: warm-up done, {} queries × {} repeat(s) timed",
        queries.len(),
        args.repeat
    ));

    let mut latencies: Vec<f64> = Vec::new();
    let mut returned_by_case: HashMap<String, Vec<String>> = HashMap::new();
    for repeat in 0..args.repeat {
        for query in &queries {
            let started = Instant::now();
            let hits = match &source {
                Some(source) => {
                    retrieve_memories_hybrid(
                        &storage,
                        &query.text,
                        RESULT_LIMIT_DEFAULT,
                        Some(source),
                    )
                    .await?
                }
                None => retrieve_memories(&storage, &query.text, RESULT_LIMIT_DEFAULT).await?,
            };
            latencies.push(started.elapsed().as_secs_f64() * 1000.0);
            // Quality is scored on the first pass, before §3's reinforcement
            // has touched anything; every pass feeds the latency numbers.
            if repeat == 0 {
                returned_by_case.insert(
                    query.id.clone(),
                    hits.iter().map(|hit| hit.id.clone()).collect(),
                );
            }
        }
        progress(&format!(
            "search: repeat {}/{} done — {} timed calls, p50 {:.1} ms",
            repeat + 1,
            args.repeat,
            latencies.len(),
            percentile(&latencies, 0.5).unwrap_or(0.0)
        ));
    }

    let all_ids: HashSet<String> = rows.iter().map(|row| row.id.clone()).collect();
    let cases: Vec<CaseScore> = queries
        .iter()
        .map(|query| {
            let expected: HashSet<String> = query.expected.iter().cloned().collect();
            let must_not: HashSet<String> = all_ids.difference(&expected).cloned().collect();
            let empty: Vec<String> = Vec::new();
            let returned = returned_by_case.get(&query.id).unwrap_or(&empty);
            score_case(&query.id, &expected, &must_not, returned, &query.text)
        })
        .collect();

    if !args.keep {
        drop(storage);
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => progress("corpus: removed the bench store again (pass --keep to keep it)"),
            Err(err) => progress(&format!(
                "corpus: could not remove {}: {err} (pass --keep to keep it)",
                dir.display()
            )),
        }
    } else {
        progress(&format!(
            "corpus: kept the bench store at {}",
            dir.display()
        ));
    }

    let (values, basis) = aggregate(&cases, &latencies);
    let mut counts = BTreeMap::new();
    counts.insert("rows".to_string(), json!(rows.len()));
    counts.insert("scored_rows".to_string(), json!(scored_rows));
    counts.insert("filler_rows".to_string(), json!(rows.len() - scored_rows));
    counts.insert("embedded_rows".to_string(), json!(embedded));
    counts.insert("queries".to_string(), json!(queries.len()));
    counts.insert("latency_samples".to_string(), json!(latencies.len()));
    counts.insert("result_limit".to_string(), json!(RESULT_LIMIT_DEFAULT));
    counts.insert("min_score".to_string(), json!(MIN_SCORE));
    Ok((
        SuitePayload {
            values,
            basis,
            counts,
        },
        cases,
    ))
}

// ---------------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct Environment {
    os: &'static str,
    arch: &'static str,
    profile: &'static str,
}

#[derive(Debug, Serialize)]
struct RunInfo {
    mode: String,
    size: usize,
    seed: i64,
    suite: String,
    repeat: usize,
}

#[derive(Debug, Serialize)]
struct ThresholdResult {
    status: String,
    given: BTreeMap<String, f64>,
    violations: Vec<String>,
    not_measured: Vec<String>,
}

#[derive(Debug, Serialize)]
struct Report {
    tool: &'static str,
    spec: &'static str,
    generated_at: String,
    environment: Environment,
    run: RunInfo,
    counts: BTreeMap<String, serde_json::Value>,
    metrics: BTreeMap<String, SuitePayload>,
    definitions: BTreeMap<&'static str, &'static str>,
    cases: BTreeMap<String, Vec<CaseScore>>,
    thresholds: BTreeMap<String, f64>,
    threshold_result: ThresholdResult,
}

/// The only thing that may fail this bench: an explicitly given threshold.
///
/// A threshold is checked against every suite that measured its metric; a
/// suite that cannot express the metric at all (latency on the pure fixture
/// suite, a degenerate corpus) is skipped rather than counted as a failure,
/// and a metric no suite measured makes the run "not-measured" (exit 2)
/// rather than a silent pass.
fn check_thresholds(args: &Args, metrics: &BTreeMap<String, SuitePayload>) -> ThresholdResult {
    let mut violations: Vec<String> = Vec::new();
    let mut not_measured: Vec<String> = Vec::new();
    for (flag, metric, direction) in THRESHOLDS {
        let Some(limit) = args.thresholds.get(flag) else {
            continue;
        };
        let mut measured = false;
        for (suite, payload) in metrics {
            let Some(Some(value)) = payload.values.get(metric) else {
                continue;
            };
            measured = true;
            let operator = if direction == "min" { ">=" } else { "<=" };
            let bad = if direction == "min" {
                value < limit
            } else {
                value > limit
            };
            if bad {
                violations.push(format!("{suite}.{metric}={value} fails {operator} {limit}"));
            }
        }
        if !measured {
            not_measured.push(format!("no suite measured {metric}"));
        }
    }
    let status = if !violations.is_empty() {
        "regression"
    } else if !not_measured.is_empty() {
        "not-measured"
    } else {
        "ok"
    };
    ThresholdResult {
        status: status.to_string(),
        given: args.thresholds.clone(),
        violations,
        not_measured,
    }
}

async fn build_report(args: &Args) -> anyhow::Result<Report> {
    let started = Utc::now();
    let mut metrics: BTreeMap<String, SuitePayload> = BTreeMap::new();
    let mut cases: BTreeMap<String, Vec<CaseScore>> = BTreeMap::new();
    let mut counts: BTreeMap<String, serde_json::Value> = BTreeMap::new();

    if args.suite.includes_fixture() {
        let (payload, case_rows) = run_fixture_suite(args.mode)?;
        counts.insert(
            "fixture_cases".to_string(),
            json!(payload.counts.get("cases").cloned().unwrap_or(json!(0))),
        );
        counts.insert(
            "fixture_evaluable".to_string(),
            json!(payload.counts.get("evaluable").cloned().unwrap_or(json!(0))),
        );
        metrics.insert("fixture".to_string(), payload);
        cases.insert("fixture".to_string(), case_rows);
    }
    if args.suite.includes_synthetic() {
        let (payload, case_rows) = run_synthetic_suite(args).await?;
        counts.insert(
            "synthetic_rows".to_string(),
            json!(payload.counts.get("rows").cloned().unwrap_or(json!(0))),
        );
        counts.insert(
            "synthetic_scored_rows".to_string(),
            json!(payload
                .counts
                .get("scored_rows")
                .cloned()
                .unwrap_or(json!(0))),
        );
        counts.insert(
            "synthetic_queries".to_string(),
            json!(payload.counts.get("queries").cloned().unwrap_or(json!(0))),
        );
        counts.insert(
            "embedded_rows".to_string(),
            json!(payload
                .counts
                .get("embedded_rows")
                .cloned()
                .unwrap_or(json!(0))),
        );
        counts.insert(
            "latency_samples".to_string(),
            json!(payload
                .counts
                .get("latency_samples")
                .cloned()
                .unwrap_or(json!(0))),
        );
        metrics.insert("synthetic".to_string(), payload);
        cases.insert("synthetic".to_string(), case_rows);
    }

    let threshold_result = check_thresholds(args, &metrics);
    Ok(Report {
        tool: "core/src/bin/retrieval_bench.rs",
        spec: "desktop/docs/memory-spec.md §5",
        generated_at: started.to_rfc3339(),
        environment: Environment {
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            profile: if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            },
        },
        run: RunInfo {
            mode: args.mode.as_str().to_string(),
            size: args.size,
            seed: args.seed,
            suite: args.suite.as_str().to_string(),
            repeat: args.repeat,
        },
        counts,
        metrics,
        definitions: DEFINITIONS.into_iter().collect(),
        cases,
        thresholds: args.thresholds.clone(),
        threshold_result,
    })
}

fn format_value(metric: &str, value: Option<f64>) -> String {
    match value {
        None => "n/a".to_string(),
        Some(value) if metric.starts_with("latency_") => format!("{value:.2} ms"),
        Some(value) => format!("{value:.4}"),
    }
}

fn format_report(report: &Report) -> String {
    let run = &report.run;
    let mut lines: Vec<String> = vec![
        "retrieval bench — spec §5 ranking (desktop edition)".to_string(),
        format!(
            "rust build ({}) on {}/{} · generated {}",
            report.environment.profile,
            report.environment.os,
            report.environment.arch,
            report.generated_at
        ),
        format!(
            "mode={} · size={} · seed={} · suite={} · repeat={}",
            run.mode, run.size, run.seed, run.suite, run.repeat
        ),
    ];
    let mut count_bits: Vec<String> = Vec::new();
    let order = [
        "fixture_cases",
        "fixture_evaluable",
        "synthetic_rows",
        "synthetic_scored_rows",
        "embedded_rows",
        "synthetic_queries",
        "latency_samples",
    ];
    for key in order {
        if let Some(value) = report.counts.get(key) {
            count_bits.push(format!("{key} {value}"));
        }
    }
    lines.push(format!("counts: {}", count_bits.join(" · ")));
    lines.push(String::new());
    lines.push(format!(
        "{:<10} {:<16} {:>10}  basis",
        "suite", "metric", "value"
    ));
    lines.push(format!(
        "{} {} {}  {}",
        "-".repeat(10),
        "-".repeat(16),
        "-".repeat(10),
        "-".repeat(34)
    ));
    for (suite, payload) in &report.metrics {
        for (metric, value) in &payload.values {
            let basis = payload.basis.get(metric).map(String::as_str).unwrap_or("");
            lines.push(format!(
                "{suite:<10} {metric:<16} {:>10}  {basis}",
                format_value(metric, *value)
            ));
        }
    }
    lines.push(String::new());
    lines.push("definitions:".to_string());
    for (metric, text) in DEFINITIONS {
        lines.push(format!("  {metric:<16} {text}"));
    }
    lines.push(String::new());
    lines.push(format!(
        "thresholds: {}",
        if report.thresholds.is_empty() {
            "none (measurement only)".to_string()
        } else {
            report
                .thresholds
                .iter()
                .map(|(name, value)| format!("--{name} {value}"))
                .collect::<Vec<_>>()
                .join(", ")
        }
    ));
    let result = &report.threshold_result;
    if !result.given.is_empty() {
        lines.push(format!("threshold result: {}", result.status));
        for violation in &result.violations {
            lines.push(format!("  violation  {violation}"));
        }
        for missing in &result.not_measured {
            lines.push(format!("  not measured  {missing}"));
        }
    }
    lines.join("\n")
}

fn emit_threshold_result(result: &ThresholdResult) {
    // Report the verdict on stderr — stdout belongs to the report.
    for violation in &result.violations {
        eprintln!("bench: REGRESSION {violation}");
    }
    for missing in &result.not_measured {
        eprintln!("bench: cannot measure {missing} for the threshold given");
    }
    if !result.given.is_empty() && result.status == "ok" {
        let given = result
            .given
            .iter()
            .map(|(name, value)| format!("--{name} {value}"))
            .collect::<Vec<_>>()
            .join(", ");
        eprintln!("bench: thresholds held ({given})");
    }
}

async fn run() -> i32 {
    let args = match Args::parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("bench error: {message}");
            eprintln!("{USAGE}");
            return EXIT_ERROR;
        }
    };
    if args.help {
        println!("{USAGE}");
        return EXIT_OK;
    }
    match build_report(&args).await {
        Ok(report) => {
            println!("{}", format_report(&report));
            if let Some(path) = &args.json {
                let payload = serde_json::to_string_pretty(&report).expect("report serialises");
                let write = path
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .map(std::fs::create_dir_all)
                    .unwrap_or(Ok(()))
                    .and_then(|()| std::fs::write(path, format!("{payload}\n")));
                match write {
                    Ok(()) => progress(&format!("report written to {}", path.display())),
                    Err(err) => {
                        eprintln!("bench error: cannot write {}: {err}", path.display());
                        return EXIT_ERROR;
                    }
                }
            }
            emit_threshold_result(&report.threshold_result);
            match report.threshold_result.status.as_str() {
                "regression" => EXIT_REGRESSION,
                "not-measured" => EXIT_ERROR,
                _ => EXIT_OK,
            }
        }
        Err(err) => {
            eprintln!("bench error: {err:#}");
            EXIT_ERROR
        }
    }
}

#[tokio::main]
async fn main() {
    let code = run().await;
    std::process::exit(code);
}
