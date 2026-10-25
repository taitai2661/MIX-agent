//! Reliability helpers.
//!
//! The retry policy and error classification live with the adapters, in
//! `mixd_providers::reliability`, because that is where the HTTP status is
//! actually observed. This re-export exists so the engine can talk about
//! backoff without depending on a provider implementation.

pub use mixd_providers::reliability::{
    backoff, max_attempts, should_retry, wait_for, ProviderErrorKind, MAX_ATTEMPTS,
};
