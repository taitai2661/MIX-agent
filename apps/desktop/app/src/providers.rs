use super::*;

/// Put a stored provider into — or take it out of — the live `ProviderRegistry`.
///
/// The registry is what the Run Engine resolves a model's `provider:` prefix
/// against, and it is the *only* thing `execute` consults
/// (`runs::engine`: `providers.get(&request.provider_id)`). Boot hydration is
/// not enough on its own: a provider the user adds, edits, disables or fetches
/// models for during the session used to stay absent from the registry until
/// the next launch, which surfaced as `no provider configured` on the very
/// message they had just enabled a model for.
///
/// A disabled provider is removed rather than registered, so it cannot be
/// reached through any model id either.
/// Bridges the provider crate's credential-store seam to the desktop secret
/// store. Keeping the keychain access here lets `mixd-providers` stay free of
/// a dependency on the storage layer.
pub(crate) struct SecretCredentialStore {
    secrets: SecretStore,
    provider_id: String,
}

#[async_trait_impl::async_trait]
impl mixd_providers::chatgpt::CredentialStore for SecretCredentialStore {
    async fn load(&self) -> anyhow::Result<Option<String>> {
        // A missing slot and an unreadable store both mean "no credentials";
        // the UI derives `needs_login` from the empty blob either way.
        Ok(self
            .secrets
            .get_string(&format!("provider/{}", self.provider_id), "oauth")
            .await
            .ok())
    }

    async fn save(&self, raw: &str) -> anyhow::Result<()> {
        self.secrets
            .put(
                &format!("provider/{}", self.provider_id),
                "oauth",
                raw.as_bytes(),
            )
            .await
    }
}

pub(crate) fn chatgpt_credential_store(
    secrets: &SecretStore,
    provider_id: &str,
) -> Arc<dyn mixd_providers::chatgpt::CredentialStore> {
    Arc::new(SecretCredentialStore {
        secrets: secrets.clone(),
        provider_id: provider_id.to_string(),
    })
}

pub async fn register_adapter(
    registry: &ProviderRegistry,
    secrets: &SecretStore,
    id: &str,
    kind: &str,
    preset: Option<&str>,
    base_url: &str,
    enabled: bool,
) {
    if !enabled {
        registry.unregister(id).await;
        return;
    }
    // `preset` and `kind` are both user-supplied. Rows written before the
    // unified gate may carry `preset=chatgpt` with `kind=openai`, so checking
    // only one of them silently built an OpenAI adapter for a ChatGPT row.
    if mixd_providers::chatgpt::is_chatgpt(preset, kind) {
        let oauth = secrets
            .get_string(&format!("provider/{id}"), "oauth")
            .await
            .ok();
        registry
            .register(
                id.to_string(),
                Arc::new(ChatGptProvider::new(
                    id,
                    base_url.to_string(),
                    chatgpt_credential_store(secrets, id),
                    oauth.as_deref(),
                )),
            )
            .await;
        return;
    }
    let api_key = match secrets
        .get_string(&format!("provider/{id}"), "api_key")
        .await
    {
        Ok(key) => Some(key),
        Err(err) => {
            // A provider may legitimately have no key (Ollama, LM Studio), so
            // this stays non-fatal. Log it, though: a decryption/keychain
            // failure used to be swallowed here and surfaced much later as the
            // misleading "requires an API key" message.
            tracing::debug!(provider = id, %err, "no readable API key for provider");
            None
        }
    };
    registry
        .register(
            id.to_string(),
            build_provider(kind, base_url.to_string(), api_key),
        )
        .await;
}

/// The provider that will actually serve `model_id`.
///
/// Model ids are stored as `<provider_id>:<model>`, so the prefix names the
/// provider exactly. When the registry does not have it — a provider added
/// before a fix shipped, one whose adapter was built from stale credentials,
/// or one that was disabled and re-enabled — the row is read back from storage
/// and the adapter is built on the spot. Without that step the only way out was
/// "no provider configured" plus a restart, which is what made a correctly
/// configured provider look broken.
///
/// Falls back to the first registered provider only for an id that carries no
/// resolvable prefix (a bare model name). A prefixed id whose provider is gone
/// (deleted, disabled, or never existed) is an error — silently serving it
/// from whatever provider sorts first answers through the wrong model.
pub async fn resolve_provider_for_model(
    registry: &ProviderRegistry,
    storage: &Storage,
    secrets: &SecretStore,
    model_id: &str,
) -> anyhow::Result<String> {
    if let Some((prefix, _)) = model_id.split_once(':') {
        if registry.get(prefix).await.is_some() {
            return Ok(prefix.to_string());
        }
        if let Some((kind, preset, base_url, enabled)) = provider_row(storage, prefix).await? {
            if enabled {
                register_adapter(
                    registry,
                    secrets,
                    prefix,
                    &kind,
                    preset.as_deref(),
                    &base_url,
                    true,
                )
                .await;
                return Ok(prefix.to_string());
            }
            anyhow::bail!(
                "provider not found for model: {model_id} (provider `{prefix}` is disabled)"
            );
        }
        anyhow::bail!("provider not found for model: {model_id} (unknown provider `{prefix}`)");
    }
    registry
        .list()
        .await
        .first()
        .map(|h| h.id.clone())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "プロバイダーが未設定です。設定 → プロバイダー から追加してください。 \
                 (no provider configured for model: {model_id})"
            )
        })
}

/// `(kind, base_url, enabled)` for one stored provider, `None` when the id is
/// unknown.
pub(crate) async fn provider_row(
    storage: &Storage,
    id: &str,
) -> anyhow::Result<Option<(String, Option<String>, String, bool)>> {
    type Row = (Option<String>, Option<String>, Option<String>, Option<i64>);
    let row: Option<Row> = sqlx::query_as(
        "SELECT json_extract(data, '$.kind'), json_extract(data, '$.preset'), \
         json_extract(data, '$.baseUrl'), json_extract(data, '$.enabled') \
         FROM providers WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(storage.pool())
    .await?;
    Ok(row.map(|(kind, preset, base_url, enabled)| {
        (
            kind.unwrap_or_default(),
            preset,
            base_url.unwrap_or_default(),
            // `enabled` is absent from rows written before the field existed;
            // those were all enabled.
            enabled.unwrap_or(1) != 0,
        )
    }))
}

/// Build the adapter for a stored `kind` string.
///
/// Only the three major providers have a distinct wire format; Ollama, LM
/// Studio, and every other OpenAI-compatible runtime reuse the OpenAI client
/// with a base URL — and, for the local ones, no key. Shared by boot-time
/// registration and by provider creation, so a provider imported mid-session
/// behaves exactly like one that was there at launch.
pub fn build_provider(kind: &str, base_url: String, api_key: Option<String>) -> Arc<dyn Provider> {
    use mixd_providers::preset::ProviderKind;
    match ProviderKind::parse(kind) {
        Some(ProviderKind::Anthropic) => Arc::new(AnthropicProvider::new(base_url, api_key)),
        Some(ProviderKind::Gemini) => Arc::new(GeminiProvider::new(base_url, api_key)),
        Some(ProviderKind::Ollama) => Arc::new(mixd_providers::ollama::ollama(base_url)),
        Some(ProviderKind::LMStudio) => Arc::new(mixd_providers::lmstudio::lmstudio(base_url)),
        // This factory has no provider id or secret store. `register_adapter`
        // handles ChatGPT before calling here; this variant is deliberately
        // inert so a mistaken direct call fails with "login required" instead
        // of reaching the backend without an account id.
        Some(ProviderKind::ChatGpt) => Arc::new(ChatGptProvider::unauthenticated(base_url)),
        _ => Arc::new(OpenAiProvider::new(base_url, api_key)),
    }
}
