use super::*;

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

    /// Resolve the slug stored on a provider row back to a runtime.
    ///
    /// The catalogue already uses these slugs as provider preset ids, so a
    /// provider imported before this module existed can still be enriched
    /// without a second scan. The generic `custom` preset is deliberately not
    /// resolved: it names no specific runtime and carries no reliable
    /// information about how to start or stop it. GPT4All shares that slug
    /// (it is GUI-only, so there is nothing to manage), which is why the
    /// exclusion has to be explicit rather than left to the catalogue.
    pub fn from_slug(slug: &str) -> Option<Self> {
        // Explicit rather than relying on the catalogue: `custom` must stay
        // unresolved even if a future RuntimeId is given that slug.
        if slug == "custom" {
            return None;
        }
        Self::DETECTABLE
            .iter()
            .copied()
            .find(|id| id.slug() == slug)
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
    ///
    /// Each entry is joined against every root in [`install_roots`], so a
    /// leading OS directory would be doubled (`/Applications/Applications/…`,
    /// `C:\Program Files\Program Files\…`). The app-bundle names are therefore
    /// bare, and the Windows-local trees stay relative to the user's home.
    pub fn install_dirs(self) -> &'static [&'static str] {
        match self {
            RuntimeId::LmStudio => &[
                "LM Studio.app",
                "LM Studio",
                "AppData/Local/Programs/LM Studio",
            ],
            RuntimeId::Ollama => &["Ollama.app", ".ollama", "AppData/Local/Programs/Ollama"],
            RuntimeId::Vllm => &[],
            RuntimeId::LlamaCpp => &[],
            RuntimeId::LocalAi => &["LocalAI"],
            RuntimeId::Jan => &["Jan.app", "AppData/Local/Programs/Jan"],
            RuntimeId::KoboldCpp => &[],
            RuntimeId::TextGenerationWebui => &[],
            RuntimeId::Gpt4All => &["GPT4All.app", "GPT4All"],
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

/// How the app can stop a runtime it started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeStop {
    /// The runtime is a direct child process; send SIGTERM (or TerminateProcess
    /// on Windows) and wait for it to exit.
    Child,
    /// The runtime is a daemon controlled by its own CLI, e.g. LM Studio's
    /// `lms server stop`.
    Cli { program: PathBuf, args: Vec<String> },
    /// The runtime cannot be stopped by this app (GUI-only or unsupported).
    Unsupported,
}

/// Everything needed to start one runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeLaunchSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub stop: RuntimeStop,
}

/// A loopback port worth probing, and the runtime it most commonly belongs to.
///
/// The fallback only decides the reported label when no fingerprint matches;
/// it never decides reachability.
pub struct PortHint {
    pub port: u16,
    pub fallback: RuntimeId,
}

pub const PORT_HINTS: &[PortHint] = &[
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
pub struct Fingerprint {
    pub runtime: RuntimeId,
    pub path: &'static str,
    /// Dotted path that must resolve in the response body. A generic
    /// OpenAI-compatible server answers 200 on many of these paths with an
    /// unrelated body, so a bare status check would misidentify it.
    pub marker: &'static str,
}

pub const FINGERPRINTS: &[Fingerprint] = &[
    // Ollama's native tag list. Unmistakable: the response carries `details`
    // with a model family and quantisation, which no other server emits.
    Fingerprint {
        runtime: RuntimeId::Ollama,
        path: "/api/tags",
        marker: "models.0.details.family",
    },
    // Ollama with an empty library has no tag details for the marker above, so
    // a fresh install could not be identified at all. `/api/version` is stable
    // across versions and distinct from vLLM's `/version`.
    Fingerprint {
        runtime: RuntimeId::Ollama,
        path: "/api/version",
        marker: "version",
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
