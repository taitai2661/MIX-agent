use super::*;

/// Resolve a dotted path such as `models.0.details` against a JSON body.
///
/// A path, not a bare field name, because the fields that actually identify a
/// runtime are rarely at the top level — Ollama's `details` sits inside
/// `models[0]`, and a flat lookup would silently stop recognising Ollama.
pub fn path_get<'a>(body: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut current = body;
    for segment in path.split('.') {
        current = match segment.parse::<usize>() {
            Ok(index) => current.get(index)?,
            Err(_) => current.get(segment)?,
        };
    }
    Some(current)
}

/// A model as reported by a detected runtime.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiscoveredModel {
    pub id: String,
    pub label: String,
    /// `0` when the server does not publish a context length. The fast sweep
    /// leaves this at `0` for Ollama; [`enrich_models`] fills it from
    /// `POST /api/show` on the detailed scan/import path.
    pub context_window: u32,
    /// Human-readable parameter size, when the runtime publishes it
    /// (`"7B"` from Ollama's tag details).
    pub parameter_size: Option<String>,
    pub quantization: Option<String>,
    /// On-disk size, when the runtime publishes it (`/api/tags` or LM Studio).
    #[serde(default)]
    pub size_bytes: Option<u64>,
    /// Model family / architecture (`llama`, `qwen3`, `qwen2`, …).
    #[serde(default)]
    pub family: Option<String>,
    /// `true` for a model currently resident in the runtime.
    #[serde(default)]
    pub loaded: bool,
    /// `llm`, `vlm`, `embedding`, … when the runtime classifies models.
    #[serde(default)]
    pub model_type: Option<String>,
    /// Native capability answer. `None` means "not published"; callers must
    /// not read that as "unsupported".
    #[serde(default)]
    pub capabilities: Option<ModelCapabilities>,
}

/// Capability flags a runtime publishes for one model.
///
/// Tri-state on purpose: `None` is "the runtime did not say", which is
/// different from `Some(false)` ("the runtime says no"). The model table only
/// stores booleans, so a `None` must not overwrite a previously probed value.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCapabilities {
    #[serde(default)]
    pub chat: Option<bool>,
    #[serde(default)]
    pub tools: Option<bool>,
    #[serde(default)]
    pub vision: Option<bool>,
    #[serde(default)]
    pub reasoning: Option<bool>,
    #[serde(default)]
    pub embedding: Option<bool>,
}

impl ModelCapabilities {
    /// `true` when at least one flag was actually reported.
    pub fn any(&self) -> bool {
        self.chat.is_some()
            || self.tools.is_some()
            || self.vision.is_some()
            || self.reasoning.is_some()
            || self.embedding.is_some()
    }
}

/// Native detail for a single model, merged into [`DiscoveredModel`].
#[derive(Debug, Clone, Default)]
pub struct ModelDetails {
    pub context_window: Option<u32>,
    pub parameter_size: Option<String>,
    pub quantization: Option<String>,
    pub family: Option<String>,
    pub size_bytes: Option<u64>,
    pub loaded: bool,
    pub model_type: Option<String>,
    pub capabilities: ModelCapabilities,
}

impl ModelDetails {
    pub fn merge_into(self, model: &mut DiscoveredModel) {
        if let Some(value) = self.context_window.filter(|value| *value > 0) {
            // Prefer the larger of the two: a tag list sometimes carries a
            // runtime default while `/api/show` carries the model maximum.
            model.context_window = model.context_window.max(value);
        }
        if let Some(value) = self.parameter_size {
            model.parameter_size = Some(value);
        }
        if let Some(value) = self.quantization {
            model.quantization = Some(value);
        }
        if let Some(value) = self.family {
            model.family = Some(value);
        }
        if let Some(value) = self.size_bytes {
            model.size_bytes = Some(value);
        }
        if let Some(value) = self.model_type {
            model.model_type = Some(value);
        }
        if self.loaded {
            model.loaded = true;
        }
        if self.capabilities.any() {
            // Field-wise merge, not replacement: the LM Studio REST payload
            // and the `lms ls --json` payload each know a different subset of
            // capabilities, and the second pass must not erase what the first
            // learned.
            match &mut model.capabilities {
                Some(existing) => existing.merge(self.capabilities),
                None => model.capabilities = Some(self.capabilities),
            }
        }
    }
}

impl ModelCapabilities {
    /// Overwrite only the flags the other side actually answered.
    ///
    /// `None` means "not published"; treating it as `false` would turn an
    /// incomplete second source into a capability regression.
    pub fn merge(&mut self, other: Self) {
        if other.chat.is_some() {
            self.chat = other.chat;
        }
        if other.tools.is_some() {
            self.tools = other.tools;
        }
        if other.vision.is_some() {
            self.vision = other.vision;
        }
        if other.reasoning.is_some() {
            self.reasoning = other.reasoning;
        }
        if other.embedding.is_some() {
            self.embedding = other.embedding;
        }
    }
}

/// The outcome of probing a single candidate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedRuntime {
    pub slug: String,
    pub label: String,
    pub kind: String,
    /// The OpenAI-compatible base URL to register as a provider.
    pub base_url: String,
    pub host: String,
    pub port: u16,
    /// The port accepted a TCP connection. Distinct from `online`: a known LLM
    /// port is very often held by something else entirely — macOS binds its
    /// AirPlay Receiver to `5000` out of the box, which is one of this module's
    /// probe targets.
    pub listening: bool,
    /// The runtime answered and is identified (or serves a model list), so the
    /// app can talk to it. An identified runtime with an empty library is
    /// online: model presence is not a health signal.
    pub online: bool,
    pub latency_ms: Option<u64>,
    /// Version string when the runtime publishes one on a probe path.
    pub version: Option<String>,
    /// Installed on this machine (app bundle, install dir, or executable on
    /// `PATH`). `false` for a runtime the app has no way to look for.
    pub installed: bool,
    /// Where the install was found, for the UI to show verbatim.
    pub install_hint: Option<String>,
    /// The command that starts this runtime's server. `None` when the runtime
    /// is a GUI app with no CLI, or when no launcher was found on disk.
    pub launcher: Option<String>,
    pub models: Vec<DiscoveredModel>,
    /// Set when the port is open but no OpenAI-compatible model list came back.
    pub error: Option<String>,
}

impl DetectedRuntime {
    /// Why the runtime is not usable, in a form the UI can show without
    /// inspecting the rest of the struct.
    pub fn unavailable_reason(&self) -> Option<&str> {
        if self.online {
            return self.error.as_deref();
        }
        if self.listening {
            return Some("このポートは他のアプリケーションが使用中です");
        }
        if self.installed {
            Some("サーバーが起動していません")
        } else {
            None
        }
    }
}

/// A probe target whose port answered but which is not an LLM server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OccupiedPort {
    pub port: u16,
    /// Set when a well-known non-LLM service is known to hold this port, so the
    /// UI can explain the conflict rather than just reporting a number.
    pub note: Option<String>,
}

/// Why a port in the sweep is often already taken by something that is not an
/// inference server. macOS binds its AirPlay Receiver to `5000` on every default
/// install, and `5000` is a probe target — so this is not hypothetical.
pub fn occupied_port_note(port: u16) -> Option<String> {
    match port {
        5000 => Some("macOS の AirPlay レシーバーが既定で 5000 番を使用します".to_string()),
        _ => None,
    }
}

/// A single scan across the whole sweep.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanReport {
    pub runtimes: Vec<DetectedRuntime>,
    /// Ports that answered but serve no model list. Reported separately so a
    /// silent row never masquerades as a runtime the user can import.
    pub occupied_ports: Vec<OccupiedPort>,
    /// Hosts that were swept.
    pub hosts: Vec<String>,
    /// Milliseconds for the whole sweep, for the UI's "検出に 84ms" line.
    pub elapsed_ms: u64,
}

/// Pull a `context_length`-style field out of a model entry, tolerating the
/// several names the ecosystem uses.
pub fn context_of(value: &serde_json::Value) -> Option<u32> {
    const KEYS: &[&str] = &[
        "context_length",
        "context_window",
        "contextLength",
        "max_context_length",
        "maxContextLength",
        "loaded_context_length",
        "max_model_len",
        "max_model_length",
        "n_ctx",
        "input_token_limit",
    ];
    KEYS.iter()
        .find_map(|key| value.get(*key).and_then(serde_json::Value::as_u64))
        .and_then(|n| u32::try_from(n).ok())
}

pub fn label_of(id: &str, value: &serde_json::Value) -> String {
    value
        .get("name")
        .or_else(|| value.get("display_name"))
        .or_else(|| value.get("title"))
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(id)
        .to_string()
}

/// OpenAI-shaped model list: `{"object": "list", "data": [{"id": "..."}]}`.
pub fn parse_openai_models(body: &serde_json::Value) -> Vec<DiscoveredModel> {
    let Some(entries) = body.get("data").and_then(serde_json::Value::as_array) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| {
            let id = entry
                .get("id")
                .or_else(|| entry.get("name"))
                .and_then(serde_json::Value::as_str)?;
            if id.is_empty() {
                return None;
            }
            Some(DiscoveredModel {
                label: label_of(id, entry),
                id: id.to_string(),
                context_window: context_of(entry).unwrap_or(0),
                size_bytes: entry
                    .get("size_bytes")
                    .or_else(|| entry.get("size"))
                    .and_then(serde_json::Value::as_u64),
                ..DiscoveredModel::default()
            })
        })
        .collect()
}

/// Ollama's native tag list: `{"models": [{"name": "llama3:8b", "details": {...}}]}`.
///
/// Richer than the OpenAI view — it carries the parameter size and
/// quantisation — so it is preferred when the server is recognised as Ollama.
pub fn parse_ollama_tags(body: &serde_json::Value) -> Vec<DiscoveredModel> {
    let Some(entries) = body.get("models").and_then(serde_json::Value::as_array) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| {
            let id = entry
                .get("name")
                .or_else(|| entry.get("model"))
                .and_then(serde_json::Value::as_str)?;
            if id.is_empty() {
                return None;
            }
            let details = entry.get("details");
            Some(DiscoveredModel {
                label: label_of(id, entry),
                id: id.to_string(),
                context_window: context_of(entry).unwrap_or(0),
                parameter_size: details
                    .and_then(|d| d.get("parameter_size"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
                quantization: details
                    .and_then(|d| d.get("quantization_level"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
                size_bytes: entry
                    .get("size")
                    .or_else(|| entry.get("size_bytes"))
                    .and_then(serde_json::Value::as_u64),
                family: details
                    .and_then(|d| d.get("family"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
                ..DiscoveredModel::default()
            })
        })
        .collect()
}

/// Version string from whichever probe body carries one.
pub fn version_of(body: &serde_json::Value) -> Option<String> {
    for key in ["version", "ollama_version", "server_version"] {
        if let Some(value) = body.get(key).and_then(serde_json::Value::as_str) {
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// Read a dotted `model_info` context key, e.g. `llama.context_length`.
pub fn context_from_model_info(body: &serde_json::Value) -> Option<u32> {
    let info = body.get("model_info")?.as_object()?;
    info.iter()
        .filter(|(key, _)| key.ends_with(".context_length") || key.as_str() == "context_length")
        .filter_map(|(_, value)| value.as_u64())
        .max()
        .and_then(|value| u32::try_from(value).ok())
}

/// Turn Ollama's `capabilities` array into tri-state flags.
pub fn ollama_capabilities(body: &serde_json::Value) -> ModelCapabilities {
    let Some(values) = body
        .get("capabilities")
        .and_then(serde_json::Value::as_array)
    else {
        return ModelCapabilities::default();
    };
    // Ollama publishes the complete list for the model, so absence is a real
    // "no". Starting from `false` lets a re-sync clear a stale `true` from an
    // older probe instead of only ever adding capabilities.
    let mut capabilities = ModelCapabilities {
        chat: Some(false),
        tools: Some(false),
        vision: Some(false),
        reasoning: Some(false),
        embedding: Some(false),
    };
    for value in values.iter().filter_map(serde_json::Value::as_str) {
        match value {
            "completion" => capabilities.chat = Some(true),
            "tools" => capabilities.tools = Some(true),
            "vision" => capabilities.vision = Some(true),
            "thinking" => capabilities.reasoning = Some(true),
            "embedding" => capabilities.embedding = Some(true),
            _ => {}
        }
    }
    capabilities
}

/// Parse `POST /api/show`. Ollama publishes context, family, quantisation and
/// (on newer versions) the capability list for exactly the model asked about.
pub fn ollama_details_from_show(body: &serde_json::Value, size_bytes: Option<u64>) -> ModelDetails {
    let details = body.get("details");
    let capabilities = ollama_capabilities(body);
    let model_type = if capabilities.embedding == Some(true) {
        Some("embedding".to_string())
    } else if capabilities.vision == Some(true) {
        Some("vlm".to_string())
    } else if capabilities.any() {
        Some("llm".to_string())
    } else {
        None
    };
    ModelDetails {
        context_window: context_from_model_info(body)
            .or_else(|| context_of(body))
            .or_else(|| {
                details
                    .and_then(|d| d.get("context_length"))
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|value| u32::try_from(value).ok())
            }),
        parameter_size: details
            .and_then(|d| d.get("parameter_size"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        quantization: details
            .and_then(|d| d.get("quantization_level"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        family: details
            .and_then(|d| d.get("family"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                body.pointer("/model_info/general.architecture")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            }),
        size_bytes: size_bytes.or_else(|| body.get("size").and_then(serde_json::Value::as_u64)),
        loaded: false,
        model_type,
        capabilities,
    }
}

/// Parse one entry from LM Studio's `/api/v0/models`.
pub fn lmstudio_details(entry: &serde_json::Value) -> ModelDetails {
    let model_type = entry
        .get("type")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let mut capabilities = ModelCapabilities::default();
    match model_type.as_deref() {
        Some("llm") => capabilities.chat = Some(true),
        Some("vlm") => {
            capabilities.chat = Some(true);
            capabilities.vision = Some(true);
        }
        Some("embeddings") => {
            capabilities.chat = Some(false);
            capabilities.embedding = Some(true);
        }
        _ => {}
    }
    // LM Studio's native list carries a capability array on models that
    // support it (`tool_use`, `vision`, …). When the array is present it is
    // authoritative for tools: absence means no tool support, not unknown.
    if let Some(values) = entry
        .get("capabilities")
        .and_then(serde_json::Value::as_array)
    {
        capabilities.tools = Some(false);
        for value in values.iter().filter_map(serde_json::Value::as_str) {
            match value {
                "tool_use" | "tools" => capabilities.tools = Some(true),
                "vision" => capabilities.vision = Some(true),
                "reasoning" | "thinking" => capabilities.reasoning = Some(true),
                "embedding" => capabilities.embedding = Some(true),
                _ => {}
            }
        }
    }
    ModelDetails {
        context_window: context_of(entry),
        parameter_size: None,
        quantization: entry
            .get("quantization")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        family: entry
            .get("arch")
            .or_else(|| entry.get("architecture"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        size_bytes: entry
            .get("size_bytes")
            .or_else(|| entry.get("size"))
            .and_then(serde_json::Value::as_u64),
        loaded: entry
            .get("state")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|state| state.eq_ignore_ascii_case("loaded")),
        model_type,
        capabilities,
    }
}

/// Parse one entry from `lms ls --json`.
///
/// The REST list carries the live `state`, but the CLI is the only source for
/// on-disk size, parameter count and the trained-for-tool-use flag. Neither
/// alone is enough, so the detailed pass merges both.
pub fn lmstudio_cli_details(entry: &serde_json::Value) -> ModelDetails {
    let model_type = entry
        .get("type")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let is_embedding = model_type.as_deref() == Some("embedding");
    let mut capabilities = ModelCapabilities {
        chat: Some(!is_embedding),
        embedding: is_embedding.then_some(true),
        ..ModelCapabilities::default()
    };
    if let Some(vision) = entry.get("vision").and_then(serde_json::Value::as_bool) {
        capabilities.vision = Some(vision);
    }
    if let Some(tools) = entry
        .get("trainedForToolUse")
        .and_then(serde_json::Value::as_bool)
    {
        capabilities.tools = Some(tools);
    }
    ModelDetails {
        context_window: context_of(entry),
        parameter_size: entry
            .get("paramsString")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        quantization: entry
            .get("quantization")
            .and_then(|value| {
                value
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| value.as_str())
            })
            .map(str::to_string),
        family: entry
            .get("architecture")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        size_bytes: entry.get("sizeBytes").and_then(serde_json::Value::as_u64),
        loaded: false,
        model_type,
        capabilities,
    }
}

/// `lms ls --json`, when the CLI is installed. A missing or broken CLI is not
/// an error: the REST list already produced a usable model set.
pub async fn fetch_lmstudio_cli_models() -> Vec<serde_json::Value> {
    // A detailed scan runs on every refresh and on every model operation. The
    // CLI takes hundreds of milliseconds to start a Node process, so a short
    // cache keeps the sweep cheap without hiding a model the user just pulled.
    const TTL: Duration = Duration::from_secs(15);
    let cache = lmstudio_cli_cache();
    if let Ok(cache) = cache.lock() {
        if let Some((at, models)) = cache.as_ref() {
            if at.elapsed() < TTL {
                return models.clone();
            }
        }
    }

    let models = match find_on_path("lms") {
        Some(program) => {
            let output = tokio::time::timeout(
                Duration::from_secs(3),
                tokio::process::Command::new(program)
                    .args(["ls", "--json"])
                    .output(),
            )
            .await;
            match output {
                Ok(Ok(output)) if output.status.success() => {
                    serde_json::from_slice::<serde_json::Value>(&output.stdout)
                        .ok()
                        .and_then(|body| {
                            body.as_array().cloned().or_else(|| {
                                body.get("data")
                                    .and_then(serde_json::Value::as_array)
                                    .cloned()
                            })
                        })
                        .unwrap_or_default()
                }
                _ => Vec::new(),
            }
        }
        None => Vec::new(),
    };
    if let Ok(mut cache) = cache.lock() {
        *cache = Some((Instant::now(), models.clone()));
    }
    models
}

pub type LmsModelCache = std::sync::Mutex<Option<(Instant, Vec<serde_json::Value>)>>;

pub fn lmstudio_cli_cache() -> &'static LmsModelCache {
    static CACHE: std::sync::OnceLock<LmsModelCache> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(None))
}

/// Drop the `lms ls` cache after a pull/delete so the next scan sees the new
/// model without waiting for the TTL.
pub fn invalidate_lmstudio_cli_cache() {
    if let Ok(mut cache) = lmstudio_cli_cache().lock() {
        *cache = None;
    }
}

/// llama.cpp publishes the loaded model's context in `/props`.
pub fn llamacpp_details(props: &serde_json::Value) -> ModelDetails {
    let context_window = props
        .pointer("/default_generation_settings/n_ctx")
        .or_else(|| props.get("n_ctx"))
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u32::try_from(value).ok());
    let family = props
        .get("model_path")
        .and_then(serde_json::Value::as_str)
        .and_then(|path| Path::new(path).file_stem())
        .and_then(|stem| stem.to_str())
        .map(str::to_string);
    ModelDetails {
        context_window,
        family,
        model_type: Some("llm".to_string()),
        ..ModelDetails::default()
    }
}
