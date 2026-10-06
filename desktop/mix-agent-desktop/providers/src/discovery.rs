//! Local runtime discovery.
//!
//! The user should not have to remember that LM Studio listens on `1234` while
//! Ollama listens on `11434`. This module sweeps the loopback interface for the
//! ports local inference runtimes are known to bind, fingerprints whatever
//! answers, and reports the model list it exposes.
//!
//! Two signals are collected, and they answer different questions:
//!
//! - **Reachability** — a TCP connect to a candidate port. This is the only
//!   signal that says "a server is running *right now*". One refused connect is
//!   a single syscall on loopback, so the whole sweep is a few microseconds per
//!   closed port and the probes run concurrently.
//! - **Installed** — an app bundle, a well-known install directory, or an
//!   executable on `PATH`. This says "you have the software, start the server".
//!   Without it the UI can only ever show an empty list, which is exactly the
//!   dead end the Docker edition's `ollama_tags` discovery was built to avoid.
//!
//! Ports are shared between runtimes (LocalAI and llama.cpp both default to
//! `8080`, Jan and GPT4All both to `1337`), so the port alone cannot identify a
//! server. Every open port is therefore fingerprinted: a handful of cheap
//! vendor-specific endpoints are queried in parallel and the first positive
//! match wins. Ports with no positive match fall back to the runtime the port
//! most commonly belongs to.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use futures_util::future::join_all;
use serde::{Deserialize, Serialize};

use crate::preset::ProviderKind;

/// A closed loopback port is answered by the kernel immediately, so the connect
/// budget only has to cover a loaded machine. The request budget is larger
/// because a loaded model server can take a while to answer a model list.
const CONNECT_TIMEOUT: Duration = Duration::from_millis(400);
const REQUEST_TIMEOUT: Duration = Duration::from_millis(2500);
const TCP_CONNECT_TIMEOUT: Duration = Duration::from_millis(300);

/// Loopback hosts to sweep. `127.0.0.1` covers the overwhelming majority of
/// runtimes, but a server bound only to the IPv6 wildcard (`llama-server
/// --host ::`, some container bridges) is invisible there.
const LOOPBACK_HOSTS: &[&str] = &["127.0.0.1", "[::1]"];

/// A local inference runtime this app knows how to recognise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeId {
    LmStudio,
    Ollama,
    Vllm,
    LlamaCpp,
    LocalAi,
    Jan,
    KoboldCpp,
    TextGenerationWebui,
    Gpt4All,
    /// A server that answered but matched no fingerprint. This is a *label*,
    /// not a thing to go looking for on disk, so it is deliberately absent from
    /// [`RuntimeId::DETECTABLE`] — the conversation endpoint is OpenAI-compatible
    /// either way, so it needs no distinct adapter.
    OpenAiCompat,
}

impl RuntimeId {
    /// Runtimes that can be looked for on this machine. `OpenAiCompat` is
    /// excluded because there is no such program to install.
    pub const DETECTABLE: &'static [RuntimeId] = &[
        RuntimeId::LmStudio,
        RuntimeId::Ollama,
        RuntimeId::Vllm,
        RuntimeId::LlamaCpp,
        RuntimeId::LocalAi,
        RuntimeId::Jan,
        RuntimeId::KoboldCpp,
        RuntimeId::TextGenerationWebui,
        RuntimeId::Gpt4All,
    ];

    /// Slug of the matching entry in [`crate::preset::PRESETS`], so an imported
    /// provider inherits the catalogue's default URL and key policy.
    pub fn slug(self) -> &'static str {
        match self {
            RuntimeId::LmStudio => "lmstudio",
            RuntimeId::Ollama => "ollama",
            RuntimeId::Vllm => "vllm",
            RuntimeId::LlamaCpp => "llama-cpp",
            RuntimeId::LocalAi => "localai",
            RuntimeId::Jan => "jan",
            RuntimeId::KoboldCpp => "koboldcpp",
            RuntimeId::TextGenerationWebui => "text-generation-webui",
            RuntimeId::Gpt4All => "custom",
            RuntimeId::OpenAiCompat => "custom",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            RuntimeId::LmStudio => "LM Studio",
            RuntimeId::Ollama => "Ollama",
            RuntimeId::Vllm => "vLLM",
            RuntimeId::LlamaCpp => "llama.cpp server",
            RuntimeId::LocalAi => "LocalAI",
            RuntimeId::Jan => "Jan",
            RuntimeId::KoboldCpp => "KoboldCpp",
            RuntimeId::TextGenerationWebui => "text-generation-webui",
            RuntimeId::Gpt4All => "GPT4All",
            RuntimeId::OpenAiCompat => "OpenAI 互換サーバー",
        }
    }

    pub fn kind(self) -> ProviderKind {
        match self {
            RuntimeId::LmStudio => ProviderKind::LMStudio,
            RuntimeId::Ollama => ProviderKind::Ollama,
            _ => ProviderKind::Compatible,
        }
    }

    /// Executables that indicate the runtime is installed but not serving.
    /// Portables (llama.cpp, KoboldCpp, vLLM) ship as a single file with no
    /// fixed location, so `PATH` is the only place to look for them.
    pub fn executables(self) -> &'static [&'static str] {
        match self {
            RuntimeId::LmStudio => &["lms"],
            RuntimeId::Ollama => &["ollama"],
            RuntimeId::Vllm => &["vllm"],
            RuntimeId::LlamaCpp => &["llama-server", "server", "llamafile"],
            RuntimeId::LocalAi => &["local-ai"],
            RuntimeId::Jan => &["jan"],
            RuntimeId::KoboldCpp => &["koboldcpp", "koboldcpp.exe"],
            RuntimeId::TextGenerationWebui => &["text-generation-webui"],
            RuntimeId::Gpt4All => &["llamafile"],
            RuntimeId::OpenAiCompat => &[],
        }
    }

    /// Installed directories relative to a per-OS root, tried in order. These
    /// catch a GUI app that ships no launcher binary on `PATH` (LM Studio's
    /// `lms` CLI is optional) and a runtime installed outside `PATH`.
    pub fn install_dirs(self) -> &'static [&'static str] {
        match self {
            RuntimeId::LmStudio => &["Applications/LM Studio.app", "Program Files/LM Studio"],
            RuntimeId::Ollama => &[
                "Applications/Ollama.app",
                ".ollama",
                "AppData/Local/Programs/Ollama",
            ],
            RuntimeId::Vllm => &[],
            RuntimeId::LlamaCpp => &[],
            RuntimeId::LocalAi => &["Program Files/LocalAI"],
            RuntimeId::Jan => &["Applications/Jan.app", "AppData/Local/Programs/Jan"],
            RuntimeId::KoboldCpp => &[],
            RuntimeId::TextGenerationWebui => &[],
            RuntimeId::Gpt4All => &["Applications/GPT4All.app", "AppData/Local/Programs/GPT4All"],
            RuntimeId::OpenAiCompat => &[],
        }
    }

    /// The command that starts this runtime's server, for the UI to show when
    /// the app is installed but not serving. `None` for a GUI-only runtime,
    /// where the answer is "launch the app" rather than a shell command.
    pub fn launch_command(self, executable: &str) -> Option<String> {
        let command = match self {
            RuntimeId::LmStudio => format!("{executable} server start"),
            RuntimeId::Ollama => format!("{executable} serve"),
            RuntimeId::Vllm => format!("{executable} serve <モデル>"),
            RuntimeId::LlamaCpp => format!("{executable} -m <model.gguf> --port 8080"),
            RuntimeId::LocalAi => format!("{executable} run"),
            RuntimeId::KoboldCpp => format!("{executable} --port 5001"),
            RuntimeId::Jan | RuntimeId::TextGenerationWebui | RuntimeId::Gpt4All => {
                return None;
            }
            RuntimeId::OpenAiCompat => return None,
        };
        Some(command)
    }

    /// `true` when the runtime has a known launcher, so the UI can say
    /// "start it" instead of leaving the user with a dead end.
    pub fn has_launcher(self) -> bool {
        !self.executables().is_empty()
    }
}

/// A loopback port worth probing, and the runtime it most commonly belongs to.
///
/// The fallback only decides the reported label when no fingerprint matches;
/// it never decides reachability.
struct PortHint {
    port: u16,
    fallback: RuntimeId,
}

const PORT_HINTS: &[PortHint] = &[
    PortHint {
        port: 1234,
        fallback: RuntimeId::LmStudio,
    },
    PortHint {
        port: 11434,
        fallback: RuntimeId::Ollama,
    },
    PortHint {
        port: 8000,
        fallback: RuntimeId::Vllm,
    },
    PortHint {
        port: 8080,
        fallback: RuntimeId::LlamaCpp,
    },
    PortHint {
        port: 8090,
        fallback: RuntimeId::LocalAi,
    },
    PortHint {
        port: 1337,
        fallback: RuntimeId::Jan,
    },
    PortHint {
        port: 5000,
        fallback: RuntimeId::TextGenerationWebui,
    },
    PortHint {
        port: 5001,
        fallback: RuntimeId::KoboldCpp,
    },
    PortHint {
        port: 7860,
        fallback: RuntimeId::TextGenerationWebui,
    },
    PortHint {
        port: 4891,
        fallback: RuntimeId::Gpt4All,
    },
];

/// A vendor-specific endpoint whose presence identifies a runtime.
///
/// Each probe is issued only against ports that already answered a TCP connect,
/// and all of them are issued concurrently, so identification costs one round
/// trip rather than one round trip per candidate.
#[derive(Clone, Copy)]
struct Fingerprint {
    runtime: RuntimeId,
    path: &'static str,
    /// Dotted path that must resolve in the response body. A generic
    /// OpenAI-compatible server answers 200 on many of these paths with an
    /// unrelated body, so a bare status check would misidentify it.
    marker: &'static str,
}

const FINGERPRINTS: &[Fingerprint] = &[
    // Ollama's native tag list. Unmistakable: the response carries `details`
    // with a model family and quantisation, which no other server emits.
    Fingerprint {
        runtime: RuntimeId::Ollama,
        path: "/api/tags",
        marker: "models.0.details.family",
    },
    // vLLM's engine version, and the single most reliable positive in the set.
    Fingerprint {
        runtime: RuntimeId::Vllm,
        path: "/version",
        marker: "version",
    },
    // llama.cpp's server props. `default_generation_settings` is unique to it.
    Fingerprint {
        runtime: RuntimeId::LlamaCpp,
        path: "/props",
        marker: "default_generation_settings",
    },
    // KoboldCpp's model-name endpoint. The body is `{"result": "..."}`.
    Fingerprint {
        runtime: RuntimeId::KoboldCpp,
        path: "/api/v1/model",
        marker: "result",
    },
    // LM Studio's own (non-OpenAI) server API, served alongside `/v1`.
    Fingerprint {
        runtime: RuntimeId::LmStudio,
        path: "/api/v0/models",
        marker: "data",
    },
];

/// Resolve a dotted path such as `models.0.details` against a JSON body.
///
/// A path, not a bare field name, because the fields that actually identify a
/// runtime are rarely at the top level — Ollama's `details` sits inside
/// `models[0]`, and a flat lookup would silently stop recognising Ollama.
fn path_get<'a>(body: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredModel {
    pub id: String,
    pub label: String,
    /// `0` when the server does not publish a context length. Ollama needs a
    /// `POST /api/show` per model to learn it, which is far too slow for a
    /// sweep, so it stays unknown rather than guessed.
    pub context_window: u32,
    /// Human-readable parameter size, when the runtime publishes it
    /// (`"7B"` from Ollama's tag details).
    pub parameter_size: Option<String>,
    pub quantization: Option<String>,
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
    /// The port serves an OpenAI-compatible model list, i.e. this is a runtime
    /// the app can actually talk to.
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
fn occupied_port_note(port: u16) -> Option<String> {
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

fn http_client() -> anyhow::Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .no_proxy()
        .build()
        .map_err(|e| anyhow::anyhow!("http client: {e}"))
}

/// Join a base URL and a path without doubling the API version segment.
///
/// Local base URLs are inconsistent: the preset catalogue stores
/// `http://127.0.0.1:1234/v1`, LM Studio's own docs show the bare origin, and a
/// user pasting from either place should get the same result. Naive
/// concatenation produces `/v1/v1/models`, which 404s.
pub fn join_api(base_url: &str, path: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');
    let path = path.trim_start_matches('/');
    // Only the version segment is de-duplicated; a caller asking for
    // `/v1/models` against a base that already ends in `/v1` gets one `/v1`.
    let path = if base.ends_with("/v1") {
        path.strip_prefix("v1/").unwrap_or(path)
    } else {
        path
    };
    format!("{}/{}", base, path)
}

/// True for loopback, private-network, and link-local hosts, where an API key
/// is usually not configured. Shared with the OpenAI adapter's own key gate so
/// both agree on what "local" means.
pub fn is_loopback_host(host: &str) -> bool {
    let host = host.trim_matches(|c| c == '[' || c == ']');
    if host.eq_ignore_ascii_case("localhost") || host == "::1" || host == "0.0.0.0" {
        return true;
    }
    if host.starts_with("127.") || host.starts_with("10.") || host.starts_with("192.168.") {
        return true;
    }
    host.starts_with("172.")
        && matches!(
            host.split('.').nth(1).and_then(|o| o.parse::<u8>().ok()),
            Some(second) if (16..=31).contains(&second)
        )
}

/// Extract the host component of a base URL, without a URL-parser dependency.
pub fn host_of(base_url: &str) -> &str {
    let after_scheme = base_url.split("://").nth(1).unwrap_or(base_url);
    let authority = after_scheme.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    // Keep IPv6 literals intact: the only `:` in those separates the port from
    // a bracketed address, never the address itself.
    if let Some(end) = authority.find(']') {
        return &authority[..=end];
    }
    match authority.split_once(':') {
        Some((host, _port)) => host,
        None => authority,
    }
}

/// Pull a `context_length`-style field out of a model entry, tolerating the
/// several names the ecosystem uses.
fn context_of(value: &serde_json::Value) -> Option<u32> {
    const KEYS: &[&str] = &[
        "context_length",
        "context_window",
        "contextLength",
        "max_model_len",
        "max_model_length",
        "n_ctx",
        "input_token_limit",
    ];
    KEYS.iter()
        .find_map(|key| value.get(*key).and_then(serde_json::Value::as_u64))
        .and_then(|n| u32::try_from(n).ok())
}

fn label_of(id: &str, value: &serde_json::Value) -> String {
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
fn parse_openai_models(body: &serde_json::Value) -> Vec<DiscoveredModel> {
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
                parameter_size: None,
                quantization: None,
            })
        })
        .collect()
}

/// Ollama's native tag list: `{"models": [{"name": "llama3:8b", "details": {...}}]}`.
///
/// Richer than the OpenAI view — it carries the parameter size and
/// quantisation — so it is preferred when the server is recognised as Ollama.
fn parse_ollama_tags(body: &serde_json::Value) -> Vec<DiscoveredModel> {
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
            })
        })
        .collect()
}

/// Version string from whichever probe body carries one.
fn version_of(body: &serde_json::Value) -> Option<String> {
    for key in ["version", "ollama_version", "server_version"] {
        if let Some(value) = body.get(key).and_then(serde_json::Value::as_str) {
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// `true` when the port accepted a TCP connection.
async fn tcp_open(host: &str, port: u16) -> bool {
    // `host` is already in the form `TcpStream::connect` wants, including the
    // brackets an IPv6 literal needs.
    tokio::time::timeout(
        TCP_CONNECT_TIMEOUT,
        tokio::net::TcpStream::connect((host, port)),
    )
    .await
    .is_ok_and(|r| r.is_ok())
}

/// Result of running every fingerprint against one open port.
struct Identified {
    runtime: Option<RuntimeId>,
    version: Option<String>,
    /// Present when the Ollama tag list was readable, which is the richer model
    /// source.
    ollama_tags: Option<serde_json::Value>,
}

async fn fingerprint(client: &reqwest::Client, origin: &str) -> Identified {
    // `join_all` preserves input order, so the position of a match in
    // `FINGERPRINTS` decides the winner. The unambiguous probes come first, and
    // the weaker `/api/v0/models` probe comes last.
    let probes: Vec<Option<(&Fingerprint, serde_json::Value)>> = join_all(
        FINGERPRINTS
            .iter()
            .map(|fp| async move {
                let url = format!("{origin}{}", fp.path);
                let body: serde_json::Value = match client.get(&url).send().await {
                    Ok(response) if response.status().is_success() => {
                        response.json().await.unwrap_or_default()
                    }
                    _ => return None,
                };
                // A 200 is not enough: most OpenAI-compatible servers answer 200
                // on paths they do not implement, with an error object. Require
                // the marker path so only a genuine match counts.
                path_get(&body, fp.marker)?;
                Some((fp, body))
            })
            .collect::<Vec<_>>(),
    )
    .await;

    match probes.into_iter().flatten().next() {
        Some((fp, body)) => {
            let ollama_tags = (fp.runtime == RuntimeId::Ollama).then(|| body.clone());
            Identified {
                runtime: Some(fp.runtime),
                version: version_of(&body),
                ollama_tags,
            }
        }
        None => Identified {
            runtime: None,
            version: None,
            ollama_tags: None,
        },
    }
}

/// Probe one `host:port` and build the runtime it turned out to be.
async fn probe_port(
    client: &reqwest::Client,
    host: &str,
    port: u16,
    fallback: RuntimeId,
) -> DetectedRuntime {
    let origin = format!("http://{host}:{port}");
    let base_url = format!("{origin}/v1");
    let mut runtime = DetectedRuntime {
        slug: fallback.slug().to_string(),
        label: fallback.label().to_string(),
        kind: fallback.kind().as_str().to_string(),
        base_url: base_url.clone(),
        host: host.to_string(),
        port,
        // The caller only gets here after a successful TCP connect.
        listening: true,
        online: false,
        latency_ms: None,
        version: None,
        installed: false,
        install_hint: None,
        launcher: None,
        models: Vec::new(),
        error: None,
    };

    // The two probes that matter — the model list and the fingerprint set — are
    // issued together, so identifying a server costs one round trip.
    let started = Instant::now();
    let models_fut = async {
        let url = join_api(&base_url, "/v1/models");
        let response = client.get(&url).send().await.ok()?;
        if !response.status().is_success() {
            return None;
        }
        let body: serde_json::Value = response.json().await.ok()?;
        Some(parse_openai_models(&body))
    };
    let (models, identified) = tokio::join!(models_fut, fingerprint(client, &origin));
    runtime.latency_ms = Some(started.elapsed().as_millis() as u64);

    // Whichever way the identity was settled — a fingerprint match, or the
    // runtime the port most commonly belongs to — the install evidence and the
    // label have to come from the same one, so a row never claims to be
    // LM Studio while reporting some other program's install path.
    let id = identified.runtime.unwrap_or(fallback);
    runtime.slug = id.slug().to_string();
    runtime.label = id.label().to_string();
    runtime.kind = id.kind().as_str().to_string();
    let (installed, install_hint, executable) = locate_install(id);
    runtime.installed = installed;
    runtime.install_hint = install_hint;
    runtime.launcher = executable.as_deref().and_then(|exe| id.launch_command(exe));
    runtime.version = identified.version;

    if let Some(tags) = identified.ollama_tags {
        let parsed = parse_ollama_tags(&tags);
        if !parsed.is_empty() {
            runtime.models = parsed;
        }
    }
    if runtime.models.is_empty() {
        match models {
            Some(list) if !list.is_empty() => runtime.models = list,
            Some(_) => runtime.error = Some("モデルが 1 つもロードされていません".into()),
            None => runtime.error = Some("OpenAI 互換のモデル一覧を取得できませんでした".into()),
        }
    }
    // A fingerprint without a model list is not a usable provider. A port that
    // answered something else — AirPlay Receiver on macOS's `5000`, a stray dev
    // server — leaves the row out of the report entirely rather than showing up
    // as a runtime with nothing to import.
    runtime.online = !runtime.models.is_empty();
    runtime
}

/// Build the `DetectedRuntime` for a runtime that is not serving, from the
/// install-path and `PATH` evidence. `online` stays `false`, so the UI renders
/// it as "start the server" rather than as a usable provider.
fn installed_only(id: RuntimeId) -> DetectedRuntime {
    let (installed, install_hint, executable) = locate_install(id);
    DetectedRuntime {
        slug: id.slug().to_string(),
        label: id.label().to_string(),
        kind: id.kind().as_str().to_string(),
        base_url: default_base_url(id),
        host: "127.0.0.1".to_string(),
        port: default_port(id),
        listening: false,
        online: false,
        latency_ms: None,
        version: None,
        installed,
        install_hint,
        launcher: executable.as_deref().and_then(|exe| id.launch_command(exe)),
        models: Vec::new(),
        error: None,
    }
}

fn default_port(id: RuntimeId) -> u16 {
    PORT_HINTS
        .iter()
        .find(|h| h.fallback == id)
        .map(|h| h.port)
        .unwrap_or(1234)
}

fn default_base_url(id: RuntimeId) -> String {
    // Prefer the catalogue's URL so an imported provider matches the preset
    // exactly, including any future change to it.
    if let Some(preset) = crate::preset::by_slug(id.slug()) {
        if !preset.default_url.is_empty() {
            return preset.default_url.to_string();
        }
    }
    format!("http://127.0.0.1:{}/v1", default_port(id))
}

/// `true` when a file or directory exists, treated as "installed".
fn exists(path: &Path) -> bool {
    path.exists()
}

fn first_existing(candidates: Vec<PathBuf>) -> Option<PathBuf> {
    candidates.into_iter().find(|path| path.exists())
}

/// Roots a relative install path may sit under, most specific first.
fn install_roots() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    let mut roots = Vec::new();
    if let Some(home) = home {
        roots.push(home.clone());
        roots.push(home.join("Applications"));
        roots.push(home.join("AppData/Local"));
    }
    roots.push(PathBuf::from("/Applications"));
    roots.push(PathBuf::from("/usr/local"));
    roots.push(PathBuf::from("/opt"));
    if let Some(program_files) = std::env::var_os("ProgramFiles") {
        roots.push(PathBuf::from(program_files));
    }
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        roots.push(PathBuf::from(local_app_data));
    }
    roots
}

/// Find `name` on `PATH`, then in the conventional bin directories an installer
/// may have populated without telling the shell about it.
///
/// Portable runtimes (llama.cpp, KoboldCpp, vLLM) install as a single file, so
/// `PATH` is the only general install signal. LM Studio's `lms` is the common
/// exception: it lives in `~/.lmstudio/bin` and is not on `PATH` by default.
pub fn find_on_path(name: &str) -> Option<PathBuf> {
    // Windows `PATHEXT` has no `PATH`-adjacent default, and a bare name would
    // otherwise only match an extensionless file.
    #[cfg(windows)]
    let names: Vec<String> = std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".EXE;.CMD;.BAT".into())
        .split(';')
        .filter(|e| !e.is_empty())
        .map(|e| name.to_string() + e)
        .collect();
    #[cfg(not(windows))]
    let names: Vec<String> = vec![name.to_string()];

    let path_dirs: Vec<PathBuf> =
        std::env::var_os("PATH").map_or(Vec::new(), |path| std::env::split_paths(&path).collect());
    let extra_dirs = executable_dirs();
    for dir in path_dirs.iter().chain(extra_dirs.iter()) {
        for candidate in &names {
            let full = dir.join(candidate);
            if full.is_file() {
                return Some(full);
            }
        }
    }
    None
}

/// Bin directories worth a look even when they are not on `PATH`.
fn executable_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/bin"),
    ];
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        let home = PathBuf::from(home);
        for relative in [".local/bin", ".lmstudio/bin", ".ollama/bin", ".cargo/bin"] {
            dirs.push(home.join(relative));
        }
    }
    dirs
}

/// Collect the "installed but not serving" evidence for a runtime.
fn locate_install(id: RuntimeId) -> (bool, Option<String>, Option<String>) {
    let roots = install_roots();
    let dir_hit = id.install_dirs().iter().find_map(|relative| {
        let candidate = PathBuf::from(relative);
        let full = if candidate.is_absolute() {
            candidate
        } else {
            first_existing(roots.iter().map(|r| r.join(&candidate)).collect())?
        };
        exists(&full).then_some(full)
    });
    if let Some(path) = dir_hit {
        // The app bundle is the useful hint, but a launcher may still exist to
        // start the server without opening a window.
        let launcher = id
            .executables()
            .iter()
            .find(|exe| find_on_path(exe).is_some())
            .map(|exe| (*exe).to_string());
        return (true, Some(path.display().to_string()), launcher);
    }
    // An executable on `PATH` is itself both the install evidence and the
    // launcher.
    let exe_hit = id
        .executables()
        .iter()
        .find_map(|exe| find_on_path(exe).map(|path| (path, *exe)));
    match exe_hit {
        Some((path, exe)) => (
            true,
            Some(path.display().to_string()),
            Some(exe.to_string()),
        ),
        None => (false, None, None),
    }
}

/// Sweep the loopback interface for local inference runtimes.
///
/// Online runtimes come first, then installed-but-stopped ones, so the UI can
/// lead with something actionable. Runtimes that are neither online nor
/// installed are omitted: a list of nine "not found" rows is noise. Ports that
/// answer but serve no model list are reported as [`OccupiedPort`] instead,
/// since silently claiming "port 5000 is text-generation-webui" when it is
/// macOS's AirPlay Receiver would be worse than saying nothing.
pub async fn scan_local_runtimes() -> anyhow::Result<ScanReport> {
    let started = Instant::now();
    let client = http_client()?;

    let mut targets: Vec<(String, u16, RuntimeId)> = Vec::new();
    for host in LOOPBACK_HOSTS {
        for hint in PORT_HINTS {
            targets.push(((*host).to_string(), hint.port, hint.fallback));
        }
    }

    let results = join_all(targets.iter().map(|(host, port, fallback)| {
        let client = client.clone();
        async move {
            if tcp_open(host, *port).await {
                Some(probe_port(&client, host, *port, *fallback).await)
            } else {
                None
            }
        }
    }))
    .await;

    let probed: Vec<DetectedRuntime> = results.into_iter().flatten().collect();
    let mut runtimes: Vec<DetectedRuntime> = probed.iter().filter(|r| r.online).cloned().collect();
    let mut occupied: Vec<OccupiedPort> = probed
        .iter()
        .filter(|r| !r.online)
        .map(|r| OccupiedPort {
            port: r.port,
            note: occupied_port_note(r.port),
        })
        .collect();
    // A port that answers on both loopback hosts is one occupied port, not two.
    occupied.sort_by_key(|o| o.port);
    occupied.dedup_by_key(|o| o.port);

    // Online first, then lowest port, so the row a user is most likely to want
    // leads. Within a host pair IPv4 sorts first, which keeps the reported base
    // URL on the `127.0.0.1` literal rather than the IPv6 one.
    runtimes.sort_by(|a, b| {
        b.online
            .cmp(&a.online)
            .then_with(|| a.host.cmp(&b.host))
            .then_with(|| a.port.cmp(&b.port))
    });

    for id in RuntimeId::DETECTABLE {
        // A runtime already answered on its own port, or on the port this one
        // would be probed at, so there is nothing to add.
        if runtimes
            .iter()
            .any(|r| r.online && (r.slug == id.slug() || r.port == default_port(*id)))
        {
            continue;
        }
        let entry = installed_only(*id);
        if entry.installed {
            runtimes.push(entry);
        }
    }

    Ok(ScanReport {
        runtimes,
        occupied_ports: occupied,
        hosts: LOOPBACK_HOSTS.iter().map(|h| (*h).to_string()).collect(),
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

/// Probe one arbitrary base URL, for a runtime on another machine or on a
/// non-default port. Local-network inference boxes are common, and they are
/// indistinguishable from a loopback server by URL alone.
pub async fn probe_base_url(base_url: &str) -> anyhow::Result<DetectedRuntime> {
    let base_url = base_url.trim();
    if base_url.is_empty() {
        return Err(anyhow::anyhow!("Base URL が空です"));
    }
    let with_scheme = if base_url.contains("://") {
        base_url.to_string()
    } else {
        format!("http://{base_url}")
    };
    let host = host_of(&with_scheme).to_string();
    if host.is_empty() {
        return Err(anyhow::anyhow!("ホストを解析できません: {base_url}"));
    }
    // The port has to survive being split off for the TCP check; the base URL
    // keeps it for the HTTP request.
    let port = with_scheme
        .split("://")
        .nth(1)
        .and_then(|rest| rest.split(['/', '?', '#']).next())
        .and_then(|authority| authority.rsplit_once(':'))
        .and_then(|(_, port)| port.parse::<u16>().ok())
        .unwrap_or(if with_scheme.starts_with("https") {
            443
        } else {
            80
        });

    let client = http_client()?;
    let started = Instant::now();
    let models_fut = async {
        let url = join_api(&with_scheme, "/v1/models");
        let response = client.get(&url).send().await.ok()?;
        if !response.status().is_success() {
            return None;
        }
        let body: serde_json::Value = response.json().await.ok()?;
        Some(parse_openai_models(&body))
    };
    let origin = format!(
        "{}://{host}:{port}",
        with_scheme.split("://").next().unwrap_or("http")
    );
    let (models, identified) = tokio::join!(models_fut, fingerprint(&client, &origin));
    // An unrecognised server is still a usable provider — the OpenAI-compatible
    // client talks to it fine — it just gets a generic label.
    let mut runtime = installed_only(identified.runtime.unwrap_or(RuntimeId::OpenAiCompat));
    runtime.base_url = with_scheme.trim_end_matches('/').to_string();
    runtime.host = host;
    runtime.port = port;
    runtime.latency_ms = Some(started.elapsed().as_millis() as u64);
    runtime.version = identified.version;
    if let Some(tags) = identified.ollama_tags {
        let parsed = parse_ollama_tags(&tags);
        if !parsed.is_empty() {
            runtime.models = parsed;
        }
    }
    if runtime.models.is_empty() {
        runtime.models = models.unwrap_or_default();
    }
    // The caller asked about this specific URL, so the server is reachable by
    // construction. Whether it is *usable* is a separate question: a service
    // that answers but serves no model list is reachable and still not a
    // provider.
    runtime.listening = true;
    runtime.online = !runtime.models.is_empty();
    if runtime.models.is_empty() && runtime.error.is_none() {
        runtime.error = Some("OpenAI 互換のモデル一覧を取得できませんでした".into());
    }
    // The install fields describe *this* machine, which is irrelevant to a URL
    // that may well point at another one. Default to false since we cannot
    // confirm local installation from a URL probe alone.
    runtime.installed = false;
    runtime.install_hint = None;
    runtime.launcher = None;
    Ok(runtime)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_api_does_not_double_the_version_segment() {
        assert_eq!(
            join_api("http://127.0.0.1:1234/v1", "/v1/models"),
            "http://127.0.0.1:1234/v1/models"
        );
        assert_eq!(
            join_api("http://127.0.0.1:1234/v1/", "/v1/models"),
            "http://127.0.0.1:1234/v1/models"
        );
        assert_eq!(
            join_api("http://127.0.0.1:1234", "/v1/models"),
            "http://127.0.0.1:1234/v1/models"
        );
        assert_eq!(
            join_api("http://127.0.0.1:11434/v1", "/models"),
            "http://127.0.0.1:11434/v1/models"
        );
    }

    #[test]
    fn host_of_handles_ports_credentials_and_ipv6() {
        assert_eq!(host_of("http://127.0.0.1:1234/v1"), "127.0.0.1");
        assert_eq!(host_of("https://api.openai.com/v1"), "api.openai.com");
        assert_eq!(host_of("http://localhost"), "localhost");
        assert_eq!(host_of("http://[::1]:1234/v1"), "[::1]");
        assert_eq!(host_of("http://user:pw@10.0.0.2:8000/v1"), "10.0.0.2");
    }

    #[test]
    fn loopback_hosts_are_recognised() {
        for url in [
            "http://localhost:11434/v1",
            "http://127.0.0.1:1234/v1",
            "http://192.168.1.20:11434/v1",
            "http://[::1]:1234/v1",
        ] {
            assert!(is_loopback_host(host_of(url)), "{url} should be local");
        }
        for url in [
            "https://api.openai.com/v1",
            "https://api.anthropic.com",
            "https://172.15.0.1/v1",
            "https://172.32.0.1/v1",
        ] {
            assert!(!is_loopback_host(host_of(url)), "{url} should not be local");
        }
    }

    #[test]
    fn marker_paths_reach_into_nested_bodies() {
        let ollama = serde_json::json!({
            "models": [{
                "name": "llama3.1:8b",
                "details": {"family": "llama", "parameter_size": "8.0B"},
            }],
        });
        assert!(path_get(&ollama, "models.0.details.family").is_some());
        // A path that does not exist must not resolve, or a plain OpenAI list
        // would be mistaken for an Ollama tag list.
        assert!(path_get(&ollama, "details.family").is_none());
        assert!(path_get(&ollama, "models.1.details").is_none());

        let vllm = serde_json::json!({"version": "0.6.3"});
        assert!(path_get(&vllm, "version").is_some());
    }

    #[test]
    fn parses_openai_model_lists() {
        let body = serde_json::json!({
            "object": "list",
            "data": [
                {"id": "qwen2.5-7b-instruct", "object": "model"},
                {"id": "gpt-oss-20b", "name": "GPT OSS 20B", "max_model_len": 131072},
            ],
        });
        let models = parse_openai_models(&body);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "qwen2.5-7b-instruct");
        assert_eq!(models[0].label, "qwen2.5-7b-instruct");
        assert_eq!(models[0].context_window, 0);
        assert_eq!(models[1].label, "GPT OSS 20B");
        assert_eq!(models[1].context_window, 131072);
    }

    #[test]
    fn parses_ollama_tag_details() {
        let body = serde_json::json!({
            "models": [{
                "name": "llama3.1:8b",
                "details": {
                    "family": "llama",
                    "parameter_size": "8.0B",
                    "quantization_level": "Q4_K_M",
                },
            }],
        });
        let models = parse_ollama_tags(&body);
        assert_eq!(models[0].id, "llama3.1:8b");
        assert_eq!(models[0].parameter_size.as_deref(), Some("8.0B"));
        assert_eq!(models[0].quantization.as_deref(), Some("Q4_K_M"));
    }

    #[test]
    fn every_runtime_maps_to_a_preset_slug() {
        for id in RuntimeId::DETECTABLE {
            assert!(
                crate::preset::by_slug(id.slug()).is_some(),
                "{} has no preset",
                id.slug()
            );
        }
    }

    #[test]
    fn port_hints_have_unique_primary_ports() {
        // Ports are shared on purpose, but the *primary* port of each runtime
        // must be distinct or the "not online" row would claim a port another
        // runtime already answered on.
        let primaries: Vec<u16> = RuntimeId::DETECTABLE
            .iter()
            .map(|id| default_port(*id))
            .collect();
        let mut sorted = primaries.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), primaries.len());
    }

    #[test]
    fn default_base_url_comes_from_the_preset_catalogue() {
        assert_eq!(
            default_base_url(RuntimeId::Ollama),
            "http://127.0.0.1:11434/v1"
        );
        assert_eq!(
            default_base_url(RuntimeId::LmStudio),
            "http://127.0.0.1:1234/v1"
        );
    }
}
