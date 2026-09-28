//! Pricing reference data (placeholder). Phase 1 reads per-token pricing from
//! the upstream `model-info` static JSON, fetched via `mixd-providers`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPrice {
    pub input_per_million: f64,
    pub output_per_million: f64,
}
