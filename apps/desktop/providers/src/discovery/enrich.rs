use super::*;

pub async fn fetch_ollama_show(
    client: &reqwest::Client,
    origin: &str,
    model: &str,
) -> Option<ModelDetails> {
    let response = client
        .post(format!("{origin}/api/show"))
        .json(&serde_json::json!({ "name": model }))
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let body: serde_json::Value = response.json().await.ok()?;
    Some(ollama_details_from_show(&body, None))
}

/// `name -> loaded context length` for models currently resident in Ollama.
pub async fn fetch_ollama_ps(client: &reqwest::Client, origin: &str) -> HashMap<String, u32> {
    let mut loaded = HashMap::new();
    let Some(response) = client.get(format!("{origin}/api/ps")).send().await.ok() else {
        return loaded;
    };
    if !response.status().is_success() {
        return loaded;
    }
    let Ok(body) = response.json::<serde_json::Value>().await else {
        return loaded;
    };
    for entry in body
        .get("models")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(name) = entry
            .get("name")
            .or_else(|| entry.get("model"))
            .and_then(serde_json::Value::as_str)
        else {
            continue;
        };
        let context = entry
            .get("context_length")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(0);
        loaded.insert(name.to_string(), context);
    }
    loaded
}

pub async fn enrich_ollama(client: &reqwest::Client, origin: &str, models: &mut [DiscoveredModel]) {
    let loaded = fetch_ollama_ps(client, origin).await;
    let targets: Vec<(usize, String, Option<u64>)> = models
        .iter()
        .enumerate()
        .map(|(index, model)| (index, model.id.clone(), model.size_bytes))
        .collect();

    // A loaded Ollama can hold dozens of tags. Four concurrent `/api/show`
    // calls keep a 20-model library inside a couple of seconds without
    // hammering the server, which is often busy serving a model.
    let details = futures_util::stream::iter(targets.into_iter().map(|(index, id, size_bytes)| {
        let client = client.clone();
        let origin = origin.to_string();
        async move {
            let detail = fetch_ollama_show(&client, &origin, &id).await;
            (
                index,
                detail.map(|mut detail| {
                    if detail.size_bytes.is_none() {
                        detail.size_bytes = size_bytes;
                    }
                    detail
                }),
            )
        }
    }))
    .buffer_unordered(4)
    .collect::<Vec<_>>()
    .await;

    for (index, detail) in details {
        if let Some(detail) = detail {
            detail.merge_into(&mut models[index]);
        }
    }
    for model in models.iter_mut() {
        if let Some(context) = loaded.get(&model.id) {
            model.loaded = true;
            if model.context_window == 0 && *context > 0 {
                model.context_window = *context;
            }
        }
    }
}

pub async fn enrich_lmstudio(
    client: &reqwest::Client,
    origin: &str,
    models: &mut [DiscoveredModel],
) {
    let api_entries = match client.get(format!("{origin}/api/v0/models")).send().await {
        Ok(response) if response.status().is_success() => {
            match response.json::<serde_json::Value>().await {
                Ok(body) => body
                    .get("data")
                    .and_then(serde_json::Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
                Err(_) => Vec::new(),
            }
        }
        _ => Vec::new(),
    };
    let cli_entries = fetch_lmstudio_cli_models().await;

    for model in models.iter_mut() {
        if let Some(entry) = api_entries.iter().find(|entry| {
            entry.get("id").and_then(serde_json::Value::as_str) == Some(model.id.as_str())
        }) {
            lmstudio_details(entry).merge_into(model);
        }
        if let Some(entry) = cli_entries.iter().find(|entry| {
            entry.get("modelKey").and_then(serde_json::Value::as_str) == Some(model.id.as_str())
        }) {
            lmstudio_cli_details(entry).merge_into(model);
        }
    }
}

pub async fn enrich_llamacpp(
    client: &reqwest::Client,
    origin: &str,
    models: &mut [DiscoveredModel],
) {
    let Some(response) = client.get(format!("{origin}/props")).send().await.ok() else {
        return;
    };
    if !response.status().is_success() {
        return;
    }
    let Ok(props) = response.json::<serde_json::Value>().await else {
        return;
    };
    let detail = llamacpp_details(&props);
    for model in models.iter_mut() {
        detail.clone().merge_into(model);
    }
}

/// Fill a model list with the runtime's native detail.
///
/// This is deliberately separate from the fast sweep: `/api/show` is a
/// per-model round trip and a large Ollama library would otherwise make every
/// "detect" click slow. Callers on an import path or an explicit refresh use
/// this; the sweep itself stays responsive.
pub async fn enrich_models(
    runtime: RuntimeId,
    base_url: &str,
    models: &mut [DiscoveredModel],
) -> anyhow::Result<()> {
    if models.is_empty() {
        return Ok(());
    }
    let client = http_client()?;
    let origin = origin_of(base_url);
    match runtime {
        RuntimeId::Ollama => enrich_ollama(&client, &origin, models).await,
        RuntimeId::LmStudio => enrich_lmstudio(&client, &origin, models).await,
        RuntimeId::LlamaCpp => enrich_llamacpp(&client, &origin, models).await,
        _ => {}
    }
    Ok(())
}

/// Same as [`enrich_models`], for a provider row that only stores a preset slug.
pub async fn enrich_models_by_slug(
    slug: &str,
    base_url: &str,
    models: &mut [DiscoveredModel],
) -> anyhow::Result<()> {
    match RuntimeId::from_slug(slug) {
        Some(runtime) => enrich_models(runtime, base_url, models).await,
        None => Ok(()),
    }
}
