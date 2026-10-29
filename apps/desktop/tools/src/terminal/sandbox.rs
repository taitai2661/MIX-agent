use super::*;

pub(crate) fn sandbox_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        if matches!(ch, '"' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('"');
    out
}

/// Wrap a login shell so the kernel enforces "workspace + temp writes only".
///
/// Reads stay unrestricted (the agent has to read the toolchain, `$HOME` config
/// and system headers), and network is left alone: the tool-level egress policy
/// still governs `web_fetch`, while installs and tests need the network.
#[cfg(target_os = "macos")]
pub(crate) fn sandbox_profile(root: &std::path::Path) -> Option<String> {
    const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";
    if !std::path::Path::new(SANDBOX_EXEC).exists() {
        return None;
    }
    let mut roots = vec![
        root.to_path_buf(),
        std::env::temp_dir(),
        std::path::PathBuf::from("/tmp"),
        std::path::PathBuf::from("/private/tmp"),
    ];
    if let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) {
        // Build caches are not secrets, and denying them makes `cargo`/`npm`
        // fail in ways that look like a broken project rather than a sandbox.
        for rel in [".cargo", ".rustup", ".npm", ".cache", "Library/Caches"] {
            roots.push(home.join(rel));
        }
    }
    let mut profile =
        String::from("(version 1)\n(allow default)\n(deny file-write*)\n(allow file-write*\n");
    for path in roots {
        // The kernel matches the *resolved* path: `/var` is a symlink to
        // `/private/var` on macOS, so an un-canonicalised `$TMPDIR` never
        // matched. A trailing slash also makes `subpath` miss its children.
        let canonical = std::fs::canonicalize(&path).unwrap_or(path);
        let rendered = canonical.to_string_lossy();
        let normalized = rendered.trim_end_matches('/');
        profile.push_str("  (subpath ");
        profile.push_str(&sandbox_string(normalized));
        profile.push_str(")\n");
    }
    profile.push_str("  (subpath \"/dev\")\n)\n");
    Some(profile)
}

#[cfg(target_os = "macos")]
pub(crate) fn sandboxed_shell_config(
    config: &ShellLaunchConfig,
    root: &std::path::Path,
) -> Option<ShellLaunchConfig> {
    let profile = sandbox_profile(root)?;
    let mut args = vec![
        "-p".to_string(),
        profile,
        config.program.to_string_lossy().to_string(),
    ];
    args.extend(config.args.iter().cloned());
    Some(ShellLaunchConfig {
        program: std::path::PathBuf::from("/usr/bin/sandbox-exec"),
        args,
        is_posix: config.is_posix,
    })
}

/// Wrap a direct exec (`terminal` with `args`) in the same profile.
#[cfg(target_os = "macos")]
pub(crate) fn sandboxed_exec(
    program: &str,
    args: &[String],
    root: &std::path::Path,
) -> Option<(String, Vec<String>)> {
    let profile = sandbox_profile(root)?;
    let mut out = vec!["-p".to_string(), profile, program.to_string()];
    out.extend(args.iter().cloned());
    Some(("/usr/bin/sandbox-exec".to_string(), out))
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn sandboxed_shell_config(
    _config: &ShellLaunchConfig,
    _root: &std::path::Path,
) -> Option<ShellLaunchConfig> {
    None
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn sandboxed_exec(
    _program: &str,
    _args: &[String],
    _root: &std::path::Path,
) -> Option<(String, Vec<String>)> {
    None
}
