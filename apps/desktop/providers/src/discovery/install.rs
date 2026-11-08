use super::*;

/// Build the `DetectedRuntime` for a runtime that is not serving, from the
/// install-path and `PATH` evidence. `online` stays `false`, so the UI renders
/// it as "start the server" rather than as a usable provider.
pub fn installed_only(id: RuntimeId) -> DetectedRuntime {
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

pub fn default_port(id: RuntimeId) -> u16 {
    PORT_HINTS
        .iter()
        .find(|h| h.fallback == id)
        .map(|h| h.port)
        .unwrap_or(1234)
}

pub fn default_base_url(id: RuntimeId) -> String {
    // Prefer the catalogue's URL so an imported provider matches the preset
    // exactly, including any future change to it.
    if let Some(preset) = crate::preset::by_slug(id.slug()) {
        if !preset.default_url.is_empty() {
            return preset.default_url.to_string();
        }
    }
    format!("http://127.0.0.1:{}/v1", default_port(id))
}

/// How to start a runtime from the app, or `None` when it has no supported
/// launcher. The command is always an absolute program plus an argument array;
/// no shell is involved.
pub fn launch_spec(id: RuntimeId) -> Option<RuntimeLaunchSpec> {
    match id {
        RuntimeId::Ollama => {
            let program = find_on_path("ollama")?;
            let port = default_port(RuntimeId::Ollama);
            Some(RuntimeLaunchSpec {
                program,
                args: vec!["serve".to_string()],
                env: vec![("OLLAMA_HOST".to_string(), format!("127.0.0.1:{port}"))],
                stop: RuntimeStop::Child,
            })
        }
        RuntimeId::LmStudio => {
            let program = find_on_path("lms")?;
            let port = default_port(RuntimeId::LmStudio);
            Some(RuntimeLaunchSpec {
                stop: RuntimeStop::Cli {
                    program: program.clone(),
                    args: vec!["server".to_string(), "stop".to_string()],
                },
                program,
                args: vec![
                    "server".to_string(),
                    "start".to_string(),
                    "--port".to_string(),
                    port.to_string(),
                ],
                env: Vec::new(),
            })
        }
        _ => None,
    }
}

/// The GUI app bundle to open for runtimes with no CLI launcher.
pub fn open_app_target(id: RuntimeId) -> Option<PathBuf> {
    let roots = install_roots();
    for relative in id.install_dirs() {
        let candidate = PathBuf::from(relative);
        let full = if candidate.is_absolute() {
            candidate
        } else {
            match first_existing(roots.iter().map(|root| root.join(&candidate)).collect()) {
                Some(path) => path,
                None => continue,
            }
        };
        if full.exists() {
            return Some(full);
        }
    }
    None
}

/// `true` when a file or directory exists, treated as "installed".
pub fn exists(path: &Path) -> bool {
    path.exists()
}

pub fn first_existing(candidates: Vec<PathBuf>) -> Option<PathBuf> {
    candidates.into_iter().find(|path| path.exists())
}

/// Roots a relative install path may sit under, most specific first.
pub fn install_roots() -> Vec<PathBuf> {
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
pub fn executable_dirs() -> Vec<PathBuf> {
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
pub fn locate_install(id: RuntimeId) -> (bool, Option<String>, Option<String>) {
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
