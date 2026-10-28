//! Contract between the prose the model reads and the tools it can call.
//!
//! The system prompt names tools by id. Nothing used to check those names
//! against the registry, so the prompt told the model to use `outline` and
//! `repo_map` — neither of which exists — and `files.read` for a tool whose id
//! is `files` and whose action is `read`. The model then called a tool that
//! could not be dispatched, spent a turn on the error, and plan mode refused
//! `symbols` because the classifier only knew the stale names.
//!
//! This test scans the prompts that are actually composed and fails when a
//! backticked tool id is not in the registry.

use std::collections::HashSet;

use mixd_core::mode::Mode;
use mixd_core::prompt::compose_system;
use mixd_tools::{EgressPolicy, ToolRegistry};

/// Backticked spans that are argument names, shell words or prose, not tool ids.
///
/// Keep this list explicit: adding an entry is a deliberate statement that the
/// span names something other than a tool.
const NON_TOOL_SPANS: &[&str] = &["offset", "limit", "edits", "cd"];

/// Inline-code spans in a Markdown prompt, in order.
fn backticked(text: &str) -> Vec<&str> {
    let mut spans = text.split('`');
    spans.next(); // the text before the first opening backtick
    spans
        .step_by(2)
        .map(str::trim)
        .filter(|span| !span.is_empty())
        .collect()
}

/// Tool ids are lower-case identifiers, including MCP-qualified names.
fn is_tool_like(span: &str) -> bool {
    span.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.' || c == '-')
}

#[test]
fn every_tool_named_in_a_prompt_is_registered() {
    let registry = ToolRegistry::new(EgressPolicy::default());
    let specs = registry.specs();
    let names: HashSet<&str> = specs.iter().map(|spec| spec.name.as_str()).collect();

    let mut checked = 0usize;
    for mode in [Mode::Chat, Mode::Thinking, Mode::Plan, Mode::Agent] {
        let Some(prompt) = compose_system(None, mode) else {
            // A tool-free chat deliberately has no built-in instructions.
            continue;
        };
        for span in backticked(&prompt) {
            if !is_tool_like(span) || NON_TOOL_SPANS.contains(&span) {
                continue;
            }
            checked += 1;
            assert!(
                names.contains(span),
                "the {:?} prompt names `{span}`, which no tool answers to. Registered: {:?}",
                mode,
                {
                    let mut sorted: Vec<&str> = names.iter().copied().collect();
                    sorted.sort_unstable();
                    sorted
                }
            );
        }
    }

    assert!(
        checked >= 6,
        "the scanner found only {checked} tool references; the prompt or this test is stale"
    );
}

#[test]
fn the_old_tool_names_stay_out_of_the_prompt() {
    // These are the names the prompt used before the registry was consulted.
    // They read plausibly, which is exactly why a future edit could bring them
    // back; the test above catches that only while they stay backticked, so
    // this pins the specific regression.
    let prompt = compose_system(None, Mode::Agent).expect("agent prompt");
    for stale in ["`outline`", "`repo_map`", "`files.read`"] {
        assert!(
            !prompt.contains(stale),
            "{stale} is not a registered tool id; use `symbols` / `files`"
        );
    }
    assert!(prompt.contains("`symbols`"));
    assert!(prompt.contains("`files`"));
}
