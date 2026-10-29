use super::*;

pub fn detect_shell() -> ShellLaunchConfig {
    detect_shell_inner(std::env::var("MIX_SHELL").ok().as_deref())
}

pub(crate) fn detect_shell_inner(mix_shell_override: Option<&str>) -> ShellLaunchConfig {
    if let Some(custom) = mix_shell_override {
        let trimmed = custom.trim();
        if !trimmed.is_empty() {
            let path = std::path::PathBuf::from(trimmed);
            let file_name = path
                .file_name()
                .map(|name| name.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            let is_powershell = file_name.contains("powershell") || file_name.contains("pwsh");
            let is_cmd = file_name.contains("cmd");
            // Keep a custom PowerShell override on the same startup path as
            // the automatic fallback below. Without these flags a machine-wide
            // profile can delay or block the first prompt, which looks like a
            // marker timeout to the persistent-shell protocol.
            let args = if is_powershell {
                vec!["-NoLogo".into(), "-NoProfile".into()]
            } else {
                Vec::new()
            };
            return ShellLaunchConfig {
                program: path,
                args,
                is_posix: !(is_powershell || is_cmd),
            };
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(bash) = find_windows_bash() {
            return ShellLaunchConfig {
                program: bash,
                args: vec!["--login".into(), "-i".into()],
                is_posix: true,
            };
        }

        // Fallback to PowerShell on Windows when no bash is found
        ShellLaunchConfig {
            program: std::path::PathBuf::from("powershell.exe"),
            args: vec!["-NoLogo".into(), "-NoProfile".into()],
            is_posix: false,
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(sh) = std::env::var("SHELL") {
            let trimmed = sh.trim();
            if !trimmed.is_empty() {
                return ShellLaunchConfig {
                    program: std::path::PathBuf::from(trimmed),
                    args: Vec::new(),
                    is_posix: true,
                };
            }
        }

        for candidate in ["/bin/zsh", "/bin/bash", "/bin/sh"] {
            let p = std::path::PathBuf::from(candidate);
            if p.is_file() {
                return ShellLaunchConfig {
                    program: p,
                    args: Vec::new(),
                    is_posix: true,
                };
            }
        }

        ShellLaunchConfig {
            program: std::path::PathBuf::from("/bin/sh"),
            args: Vec::new(),
            is_posix: true,
        }
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn find_windows_bash() -> Option<std::path::PathBuf> {
    let fixed_candidates = [
        r"C:\Program Files\Git\bin\bash.exe",
        r"C:\Program Files (x86)\Git\bin\bash.exe",
        r"C:\tools\msys64\usr\bin\bash.exe",
        r"C:\msys64\usr\bin\bash.exe",
    ];
    for candidate in fixed_candidates {
        let p = std::path::PathBuf::from(candidate);
        if p.is_file() {
            return Some(p);
        }
    }

    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        let user_git = std::path::PathBuf::from(local_app_data).join(r"Programs\Git\bin\bash.exe");
        if user_git.is_file() {
            return Some(user_git);
        }
    }

    if let Ok(user_profile) = std::env::var("USERPROFILE") {
        let scoop_git =
            std::path::PathBuf::from(user_profile).join(r"scoop\apps\git\current\bin\bash.exe");
        if scoop_git.is_file() {
            return Some(scoop_git);
        }
    }

    find_executable_in_path("bash.exe")
}

#[cfg(target_os = "windows")]
pub(crate) fn find_executable_in_path(name: &str) -> Option<std::path::PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Strip the Win32 verbatim prefix returned by `std::fs::canonicalize`.
/// Git Bash and MSYS tools expect ordinary drive/UNC paths instead.
pub(crate) fn normalize_windows_path(path: &std::path::Path) -> PathBuf {
    #[cfg(windows)]
    {
        let raw = path.to_string_lossy();
        if let Some(rest) = raw.strip_prefix(r"\\?\") {
            if let Some(unc) = rest.strip_prefix("UNC\\") {
                return PathBuf::from(format!(r"\\{unc}"));
            }
            return PathBuf::from(rest);
        }
    }
    path.to_path_buf()
}

pub(crate) fn canonicalize_portable(path: &std::path::Path) -> std::io::Result<PathBuf> {
    std::fs::canonicalize(path).map(|canonical| normalize_windows_path(&canonical))
}

pub(crate) fn native_path_string(path: &std::path::Path) -> String {
    normalize_windows_path(path).to_string_lossy().into_owned()
}

pub(crate) fn posix_path_expression(path: &std::path::Path) -> String {
    if cfg!(windows) {
        let windows_path = native_path_string(path).replace('\\', "/");
        format!("\"$(cygpath -u {})\"", shell_quote(&windows_path))
    } else {
        shell_quote(&path.to_string_lossy())
    }
}

pub(crate) fn posix_cwd_expression() -> &'static str {
    if cfg!(windows) {
        "$(cygpath -w \"$PWD\")"
    } else {
        "$PWD"
    }
}
