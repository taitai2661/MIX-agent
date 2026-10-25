//! Picks the provider + model that embeds Memory text.
//!
//! Phase 1 of the memory spec (`docs/memory-spec.md` §5) adds the `vector`
//! term to Memory ranking, and this module is what decides *which* transport
//! produces those vectors. Its answer is an optimisation, never a
//! precondition: `None` from here — no embeddable provider, an unreadable
//! provider row, a configuration the transport cannot serve — leaves retrieval
//! on exactly the lexical path it took before Phase 1.
//!
//! Two properties shape the design:
//!
//! * **It must not run on every recall.** The API key lives in the OS
//!   Keychain, which is a `spawn_blocking` round trip per read, so the resolved
//!   source is cached until a provider or settings command calls
//!   [`EmbeddingResolver::invalidate`]. The *negative* answer is cached too,
//!   otherwise an Anthropic-only installation — the one setup with no
//!   embeddings endpoint at all — would touch the Keychain on every search.
//! * **It must prefer the model the user is actually talking to.** The default
//!   model's provider is the one whose embedding model is most likely to be
//!   present (pulled locally, or provisioned on the account), so it is chosen
//!   over the first provider in the table.

use std::sync::{Arc, Mutex, PoisonError};

use mixd_core::retrieval::EmbeddingSource;
use mixd_providers::preset::{default_embedding_model, ProviderKind};
use mixd_storage::{SecretStore, Storage};

/// Model assumed for a row whose `data.kind` is missing or unknown.
///
/// [`crate::build_provider`] builds the OpenAI adapter for such a row, so it
/// speaks the same `/embeddings` wire format this default belongs to.
const OPENAI_FALLBACK_MODEL: &str = "text-embedding-3-small";

/// `(id, kind, base_url, model)` — everything needed to build an
/// [`EmbeddingSource`], separated from the database read so the candidate
/// rules can be unit-tested without a provider table.
type Plan = (String, String, String, String);

/// One row of [`POOL_PROVIDERS`]: what the table stores, before
/// [`embedding_plan`] decides whether the provider can embed at all.
type ProviderRow = (
    String,
    Option<String>,
    Option<String>,
    Option<i64>,
    Option<String>,
);

/// Resolves — and remembers — the embedding source used by Memory recall.
#[derive(Clone)]
pub struct EmbeddingResolver {
    inner: Arc<Inner>,
}

struct Inner {
    storage: Arc<Storage>,
    secrets: Arc<SecretStore>,
    /// `None` until the first [`EmbeddingResolver::resolve`], then the answer.
    /// The legitimate "nothing here can embed" answer is stored as `Some(None)`
    /// rather than forgotten, because re-deriving it costs a Keychain read.
    cached: Mutex<Option<Option<EmbeddingSource>>>,
}

impl EmbeddingResolver {
    pub fn new(storage: Arc<Storage>, secrets: Arc<SecretStore>) -> Self {
        Self {
            inner: Arc::new(Inner {
                storage,
                secrets,
                cached: Mutex::new(None),
            }),
        }
    }

    /// The source to embed queries and new Memory rows with, or `None` when
    /// embedding is unavailable.
    ///
    /// Every failure mode of this function is a supported state, not a broken
    /// one: callers are expected to pass the result straight into
    /// `retrieve_memories_hybrid`, where `None` means "rank lexically".
    pub async fn resolve(&self) -> Option<EmbeddingSource> {
        let cached = self.cached().clone();
        if let Some(resolved) = cached {
            return resolved;
        }
        let resolved = self.derive().await;
        *self.cached() = Some(resolved.clone());
        resolved
    }

    /// Drop the cached answer.
    ///
    /// Called after anything that can change the outcome — a provider added,
    /// edited, enabled, disabled or deleted, or the default model moving to a
    /// different provider — so the next recall re-reads the configuration
    /// instead of embedding with a stale adapter.
    pub fn invalidate(&self) {
        *self.cached() = None;
    }

    fn cached(&self) -> std::sync::MutexGuard<'_, Option<Option<EmbeddingSource>>> {
        // The lock is never held across an `.await`, so it cannot be poisoned
        // by a panic mid-critical-section; recovering anyway keeps one bad
        // recall from disabling embedding for the rest of the session.
        self.inner
            .cached
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Read the provider table and build the source, uncached.
    async fn derive(&self) -> Option<EmbeddingSource> {
        let rows: Vec<ProviderRow> = match sqlx::query_as(POOL_PROVIDERS)
            .fetch_all(self.inner.storage.pool())
            .await
        {
            Ok(rows) => rows,
            Err(err) => {
                tracing::warn!(%err, "reading providers for embedding failed");
                return None;
            }
        };

        // `default_model_id` is stored as `<provider_id>:<model>`; that
        // provider is preferred, but only when it can actually embed — an
        // Anthropic default (or a deleted provider) simply falls through to
        // the next candidate.
        let preferred = crate::commands::settings::read_settings(self.inner.storage.pool())
            .await
            .ok()
            .and_then(|settings| settings.default_model_id)
            .and_then(|model_id| {
                model_id
                    .split_once(':')
                    .map(|(provider, _)| provider.to_string())
            });

        let mut preferred_plan: Option<Plan> = None;
        let mut fallback_plan: Option<Plan> = None;
        for (id, kind, base_url, enabled, model) in rows {
            let Some((kind, base_url, model)) = embedding_plan(kind, base_url, enabled, model)
            else {
                continue;
            };
            let plan = (id, kind, base_url, model);
            if preferred.as_deref() == Some(plan.0.as_str()) {
                preferred_plan = Some(plan);
                break;
            }
            if fallback_plan.is_none() {
                fallback_plan = Some(plan);
            }
        }

        let (id, kind, base_url, model) = preferred_plan.or(fallback_plan)?;
        // A missing key is fine: local runtimes have none, and a remote one
        // without a key fails at the transport and falls back to lexical.
        let api_key = self
            .inner
            .secrets
            .get_string(&format!("provider/{}", id), "api_key")
            .await
            .ok();
        Some(EmbeddingSource::new(
            crate::build_provider(&kind, base_url, api_key),
            model,
        ))
    }
}

/// Provider rows, oldest first, with the fields embedding needs.
///
/// `embeddingModel` may be written at the top level or under `extra` — the
/// former is what the provider pane writes for new keys, the latter what a
/// hand-edited row predating it tends to carry. `CAST` keeps a value stored as
/// a number (a model id with no dots is still valid JSON) from failing the
/// whole query when the column decodes as text.
const POOL_PROVIDERS: &str = "SELECT id, \
     json_extract(data, '$.kind'), \
     json_extract(data, '$.baseUrl'), \
     json_extract(data, '$.enabled'), \
     CAST(COALESCE(json_extract(data, '$.embeddingModel'), \
                   json_extract(data, '$.extra.embeddingModel')) AS TEXT) \
     FROM providers ORDER BY created_at ASC";

/// What one provider row can embed with, or `None` when it cannot take part.
///
/// The rows this rejects are exactly the ones spec §5.2's "埋め込みが使えない"
/// branch is for — a disabled provider, one with no base URL, a transport with
/// no embeddings endpoint — and rejecting them here rather than letting the
/// call fail keeps a broken provider from shadowing a working one further down
/// the table.
fn embedding_plan(
    kind: Option<String>,
    base_url: Option<String>,
    enabled: Option<i64>,
    override_model: Option<String>,
) -> Option<(String, String, String)> {
    // Rows written before `enabled` existed carry no such key; they were all
    // enabled (this matches `crate::provider_row`).
    if enabled == Some(0) {
        return None;
    }
    let kind = kind?;
    // Without a base URL the row was never configured: `build_provider` would
    // produce a client that cannot address anything, which is the same
    // "unavailable" answer, reached later and noisier.
    let base_url = base_url?.trim().trim_end_matches('/').to_string();
    if base_url.is_empty() {
        return None;
    }
    let provider_kind = ProviderKind::parse(&kind);
    if provider_kind == Some(ProviderKind::Anthropic) {
        // Anthropic has no embeddings endpoint, and the adapter keeps the
        // trait's default `EmbeddingNotSupported`. An `embeddingModel` override
        // cannot conjure one, so a row that names a model still does not
        // qualify — it would otherwise be chosen, fail on every recall, and
        // hide the provider below it.
        return None;
    }
    let model = override_model
        .map(|model| model.trim().to_string())
        .filter(|model| !model.is_empty())
        .or_else(|| default_embedding_model(&kind).map(str::to_string))
        // Unknown kind: still the OpenAI adapter, still `/embeddings`.
        .unwrap_or_else(|| OPENAI_FALLBACK_MODEL.to_string());
    Some((kind, base_url, model))
}

#[cfg(test)]
mod tests {
    use super::embedding_plan;

    #[test]
    fn a_disabled_provider_never_embeds() {
        assert_eq!(
            embedding_plan(
                Some("openai".into()),
                Some("https://api.openai.com/v1".into()),
                Some(0),
                None
            ),
            None
        );
    }

    #[test]
    fn a_provider_without_a_base_url_is_skipped() {
        assert_eq!(
            embedding_plan(Some("openai".into()), None, Some(1), None),
            None
        );
        assert_eq!(
            embedding_plan(Some("openai".into()), Some("   ".into()), Some(1), None),
            None
        );
    }

    /// Anthropic keeps `EmbeddingNotSupported` (spec §5.2), so it must not be
    /// selected even when a hand-edited row claims it has an embedding model.
    #[test]
    fn anthropic_cannot_be_persuaded_into_embedding() {
        assert_eq!(
            embedding_plan(
                Some("anthropic".into()),
                Some("https://api.anthropic.com".into()),
                Some(1),
                Some("text-embedding-3-small".into())
            ),
            None
        );
    }

    /// Each transport names its own default, and an override wins over it —
    /// a local Ollama that pulled a different model stays usable.
    #[test]
    fn the_model_is_the_override_then_the_transport_default() {
        let ollama = embedding_plan(
            Some("ollama".into()),
            Some("http://127.0.0.1:11434/v1".into()),
            Some(1),
            None,
        );
        assert_eq!(
            ollama.map(|(_, _, model)| model).as_deref(),
            Some("nomic-embed-text")
        );
        let overridden = embedding_plan(
            Some("ollama".into()),
            Some("http://127.0.0.1:11434/v1".into()),
            Some(1),
            Some("  mxbai-embed-large  ".into()),
        );
        assert_eq!(
            overridden.map(|(_, _, model)| model).as_deref(),
            Some("mxbai-embed-large")
        );
    }

    /// A row predating `enabled` reads as `NULL`, which must mean "enabled"
    /// rather than "skip": otherwise every provider saved by an older build
    /// would silently lose its vectors.
    #[test]
    fn a_missing_enabled_flag_counts_as_enabled() {
        assert!(embedding_plan(
            Some("gemini".into()),
            Some("https://generativelanguage.googleapis.com".into()),
            None,
            None
        )
        .is_some());
    }

    /// An unknown kind still builds the OpenAI adapter, so it must still get
    /// the OpenAI default instead of being excluded.
    #[test]
    fn an_unknown_kind_falls_back_to_the_openai_model() {
        let plan = embedding_plan(
            Some("some-gateway".into()),
            Some("https://gateway.example.com/v1".into()),
            Some(1),
            None,
        );
        assert_eq!(
            plan.map(|(_, _, model)| model).as_deref(),
            Some("text-embedding-3-small")
        );
    }
}
