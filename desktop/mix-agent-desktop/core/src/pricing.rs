//! Approximate per-token pricing, for the budget card and the Run summary.
//!
//! Deliberately a small static table rather than a fetched catalogue. The
//! number is presented as an estimate, and a stale-but-honest table that costs
//! nothing to consult beats a network dependency in the middle of a Run. An
//! unlisted model reports `None` rather than a guess — silently pricing a local
//! Ollama model as GPT-4o would be worse than showing no figure at all.

use mixd_providers::Usage;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ModelPrice {
    pub input_per_million: f64,
    pub output_per_million: f64,
}

/// `(model name fragment, input $/M, output $/M)`.
///
/// Matched as a substring of the wire model name, longest fragment first, so
/// `gpt-4o-mini` is not priced as `gpt-4o`.
const PRICES: &[(&str, f64, f64)] = &[
    ("claude-opus-4", 15.0, 75.0),
    ("claude-sonnet-4", 3.0, 15.0),
    ("claude-3-5-haiku", 0.80, 4.0),
    ("gpt-4o-mini", 0.15, 0.60),
    ("gpt-4o", 2.50, 10.0),
    ("gpt-4.1-mini", 0.40, 1.60),
    ("gpt-4.1", 2.00, 8.00),
    ("o4-mini", 1.10, 4.40),
    ("o3", 2.00, 8.00),
    ("gemini-2.5-pro", 1.25, 10.0),
    ("gemini-2.5-flash", 0.30, 2.50),
    ("deepseek", 0.27, 1.10),
];

/// The price for a model, or `None` when it is not in the table.
pub fn price_for(model: &str) -> Option<ModelPrice> {
    let name = model.to_ascii_lowercase();
    PRICES
        .iter()
        .filter(|(fragment, _, _)| name.contains(fragment))
        .max_by_key(|(fragment, _, _)| fragment.len())
        .map(|(_, input, output)| ModelPrice {
            input_per_million: *input,
            output_per_million: *output,
        })
}

/// Estimated cost of `usage` on `model`.
///
/// Cache reads and writes are billed at their own rates by each provider, which
/// this table does not model; they are counted at the plain input rate so the
/// figure errs towards over-reporting rather than under-reporting.
pub fn estimate_cost_usd(model: &str, usage: &Usage) -> Option<f64> {
    let price = price_for(model)?;
    let input = usage
        .input_tokens
        .saturating_add(usage.cache_read_tokens)
        .saturating_add(usage.cache_write_tokens) as f64;
    let output = usage.output_tokens as f64;
    Some(
        (input / 1_000_000.0) * price.input_per_million
            + (output / 1_000_000.0) * price.output_per_million,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(input: u64, output: u64) -> Usage {
        Usage {
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        }
    }

    #[test]
    fn a_known_model_is_priced_from_its_own_rows() {
        let cost = estimate_cost_usd("gpt-4o-mini", &usage(1_000_000, 1_000_000))
            .expect("a listed model must be priced");
        assert!((cost - 0.75).abs() < 1e-9, "got {cost}");
    }

    #[test]
    fn the_longest_fragment_wins() {
        let mini = estimate_cost_usd("gpt-4o-mini", &usage(1_000_000, 0)).unwrap();
        let full = estimate_cost_usd("gpt-4o", &usage(1_000_000, 0)).unwrap();
        assert!(mini < full, "mini must not be priced as the full model");
    }

    #[test]
    fn an_unlisted_model_has_no_estimate() {
        assert!(price_for("llama3.1:8b").is_none());
        assert!(estimate_cost_usd("llama3.1:8b", &usage(10, 10)).is_none());
    }

    #[test]
    fn cache_tokens_are_counted_rather_than_ignored() {
        let plain = estimate_cost_usd("claude-sonnet-4", &usage(1_000_000, 0)).unwrap();
        let cached = estimate_cost_usd(
            "claude-sonnet-4",
            &Usage {
                input_tokens: 0,
                output_tokens: 0,
                cache_read_tokens: 1_000_000,
                cache_write_tokens: 0,
            },
        )
        .unwrap();
        assert!(cached > 0.0 && cached == plain);
    }

    #[test]
    fn matching_ignores_case_and_a_provider_prefix() {
        assert!(price_for("openai:GPT-4o").is_some());
    }
}
