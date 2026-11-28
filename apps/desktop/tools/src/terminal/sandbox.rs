use super::*;

/// Normalise a sandbox root for kernel matching.
///
/// The kernel matches the *resolved* path: `/var` is a symlink to
/// `/private/var` on macOS, so an un-canonicalised `$TMPDIR` never matches.
/// A trailing slash also makes `subpath` miss its children. Pure function so
/// it is unit-testable on every platform (including Linux CI where
/// `sandbox-exec` does not exist).
#[cfg(any(target_os = "macos", test))]
pub(crate) fn normalize_sandbox_path(path: &std::path::Path) -> String {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    canonical
        .to_string_lossy()
        .trim_end_matches('/')
        .to_string()
}

/// Quote one path for a `sandbox-exec` profile. Callers validate the paths via
/// [`sandbox_path_is_printable`] first; a control character would either break
/// the profile syntax or be silently misinterpreted by the kernel.
#[cfg(target_os = "macos")]
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

/// A sandbox root with control characters cannot be represented safely in a
/// profile string. Reject it before building the profile instead of escaping
/// blindly (`\n` in a path must never become a new profile clause).
#[cfg(any(target_os = "macos", test))]
pub(crate) fn sandbox_path_is_printable(value: &str) -> bool {
    !value.chars().any(char::is_control)
}

#[cfg(target_os = "macos")]
fn push_sandbox_subpath(profile: &mut String, path: &std::path::Path) -> bool {
    let normalized = normalize_sandbox_path(path);
    if !sandbox_path_is_printable(&normalized) {
        tracing::warn!(path = %path.display(), "sandbox root has non-printable characters; refusing to build a profile");
        return false;
    }
    profile.push_str("  (subpath ");
    profile.push_str(&sandbox_string(&normalized));
    profile.push_str(")\n");
    true
}

/// Sensitive home paths denied for *reads* by the opt-in strict profile.
/// `.env` in the workspace is included as well: credentials are often checked
/// in under that name, and a read grant there defeats the whole point.
#[cfg(target_os = "macos")]
fn strict_read_denials(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = vec![root.join(".env")];
    if let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) {
        for rel in [
            ".ssh",
            ".aws",
            ".gnupg",
            ".config/gcloud",
            ".docker",
            ".kube",
        ] {
            out.push(home.join(rel));
        }
        out.push(home.join(".env"));
    }
    out
}

/// Wrap a login shell so the kernel enforces the sandbox profile.
///
/// The default profile is a *write confinement* profile: writes are allowed
/// only under the workspace root and temp/build-cache dirs, while reads stay
/// unrestricted (the agent has to read the toolchain, `$HOME` config and system
/// headers) and the network is left alone (installs and tests need it).
///
/// `strict` is the opt-in profile for untrusted commands: on top of write
/// confinement it denies reads of credential directories / `.env` files and
/// denies network access by default. Neither profile is a substitute for a VM.
#[cfg(target_os = "macos")]
pub(crate) fn sandbox_profile(
    root: &std::path::Path,
    deny_network: bool,
    strict: bool,
) -> Option<String> {
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
    let mut profile = String::from("(version 1)\n(allow default)\n");
    if strict {
        profile.push_str("(deny file-read*\n");
        for path in strict_read_denials(root) {
            if !push_sandbox_subpath(&mut profile, &path) {
                return None;
            }
        }
        profile.push_str(")\n");
    }
    profile.push_str("(deny file-write*)\n(allow file-write*\n");
    for path in roots {
        // See `normalize_sandbox_path`: canonicalise + strip trailing `/`.
        if !push_sandbox_subpath(&mut profile, &path) {
            return None;
        }
    }
    profile.push_str("  (subpath \"/dev\")\n)\n");
    // The relaxed write-confinement profile leaves the network alone because
    // package installs and tests need it. `deny_network` opts into blocking it;
    // the strict profile blocks it by default for untrusted commands.
    if deny_network || strict {
        profile.push_str("(deny network*)\n");
    }
    Some(profile)
}

#[cfg(target_os = "macos")]
pub(crate) fn sandboxed_shell_config(
    config: &ShellLaunchConfig,
    root: &std::path::Path,
    deny_network: bool,
    strict: bool,
) -> Option<ShellLaunchConfig> {
    let profile = sandbox_profile(root, deny_network, strict)?;
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
    deny_network: bool,
    strict: bool,
) -> Option<(String, Vec<String>)> {
    let profile = sandbox_profile(root, deny_network, strict)?;
    let mut out = vec!["-p".to_string(), profile, program.to_string()];
    out.extend(args.iter().cloned());
    Some(("/usr/bin/sandbox-exec".to_string(), out))
}

/// Is OS-level isolation available on this machine for *new* shells?
///
/// The settings pane uses this to avoid promising a sandbox that silently
/// degrades to the unsandboxed shell. On Windows the answer is always `true`
/// because the Job Object is part of the OS; on Linux it depends on a
/// bubblewrap/firejail installation.
pub fn sandbox_available() -> bool {
    #[cfg(target_os = "macos")]
    {
        std::path::Path::new("/usr/bin/sandbox-exec").exists()
    }
    #[cfg(target_os = "linux")]
    {
        linux_helper().is_some()
    }
    #[cfg(windows)]
    {
        true
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        false
    }
}

/// Why the sandbox is unavailable, for the settings pane.
pub fn sandbox_unavailable_reason() -> Option<&'static str> {
    if sandbox_available() {
        return None;
    }
    #[cfg(target_os = "linux")]
    {
        Some("bubblewrap (bwrap) が見つかりません。`sudo apt install bubblewrap` などで導入してください。")
    }
    #[cfg(target_os = "macos")]
    {
        Some("sandbox-exec が見つかりません。")
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Some("このプラットフォームではOSサンドボックスを利用できません。")
    }
}

// ---------------------------------------------------------------------------
// Linux — bubblewrap
// ---------------------------------------------------------------------------
//
// bwrap gives us the same shape the macOS profile does: the root filesystem is
// read-only, the workspace and the usual build caches stay writable, and
// `/tmp` is a private tmpfs. Network is left alone unless the user opts into
// the stricter profile. We prefer bwrap over a hand-rolled landlock helper
// because package managers, test runners and Node/Python toolchains spawn
// their own children; bwrap carries the mount namespace into every descendant.

#[cfg(target_os = "linux")]
#[derive(Clone, Copy, PartialEq, Eq)]
enum LinuxHelper {
    Bubblewrap,
    Firejail,
}

#[cfg(target_os = "linux")]
fn find_in_path(name: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

#[cfg(target_os = "linux")]
fn linux_helper() -> Option<LinuxHelper> {
    if find_in_path("bwrap").is_some() {
        return Some(LinuxHelper::Bubblewrap);
    }
    if find_in_path("firejail").is_some() {
        return Some(LinuxHelper::Firejail);
    }
    None
}

#[cfg(target_os = "linux")]
fn linux_command(
    program: &str,
    args: &[String],
    root: &std::path::Path,
    deny_network: bool,
    cwd: Option<&std::path::Path>,
) -> Option<(String, Vec<String>)> {
    let helper = linux_helper()?;
    let path_string = |path: &std::path::Path| path.to_string_lossy().into_owned();
    match helper {
        LinuxHelper::Bubblewrap => {
            let bwrap = find_in_path("bwrap")?;
            // Template: the root filesystem is read-only (`--ro-bind / /`),
            // the workspace is rebound writable over it, `/tmp` and
            // `/var/tmp` are private tmpfs mounts, and the network namespace
            // is optionally unshared (`--unshare-net`). Order matters: the
            // writable `--bind` entries must come *after* the read-only root
            // bind, or the whole tree stays read-only and the toolchain
            // fails in confusing ways.
            let mut out: Vec<String> = vec![
                "--die-with-parent".into(),
                "--new-session".into(),
                "--unshare-pid".into(),
                "--unshare-ipc".into(),
                "--unshare-uts".into(),
                "--ro-bind".into(),
                "/".into(),
                "/".into(),
                "--dev".into(),
                "/dev".into(),
                "--proc".into(),
                "/proc".into(),
                "--tmpfs".into(),
                "/tmp".into(),
                "--tmpfs".into(),
                "/var/tmp".into(),
                "--bind".into(),
                path_string(root),
                path_string(root),
            ];
            // Build caches are not secrets. Denying them makes `cargo`/`npm`
            // fail in ways that look like a broken project rather than a
            // sandbox (same rationale as the macOS profile).
            if let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) {
                for rel in [".cargo", ".rustup", ".npm", ".cache", "Library/Caches"] {
                    let dir = home.join(rel);
                    if dir.is_dir() {
                        out.push("--bind".into());
                        out.push(path_string(&dir));
                        out.push(path_string(&dir));
                    }
                }
            }
            if deny_network {
                out.push("--unshare-net".into());
            }
            if let Some(cwd) = cwd {
                out.push("--chdir".into());
                out.push(path_string(cwd));
            }
            out.push("--".into());
            out.push(program.to_string());
            out.extend(args.iter().cloned());
            Some((path_string(&bwrap), out))
        }
        LinuxHelper::Firejail => {
            let firejail = find_in_path("firejail")?;
            // Fallback helper when bwrap is absent. `--whitelist` alone only
            // *allows* the path; without a read-only root the rest of the
            // filesystem stays writable, so the root is explicitly mounted
            // read-only and only the workspace is writable. `--private-tmp`
            // hides the host `/tmp` (where the shell's status files live).
            let mut out = vec![
                "--quiet".into(),
                "--nonewprivs".into(),
                "--noroot".into(),
                "--private-tmp".into(),
                "--read-only=/".into(),
                format!("--read-write={}", path_string(root)),
                "--private-cwd".into(),
                format!("--whitelist={}", path_string(root)),
            ];
            if deny_network {
                out.push("--net=none".into());
            }
            out.push("--".into());
            out.push(program.to_string());
            out.extend(args.iter().cloned());
            Some((path_string(&firejail), out))
        }
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn sandboxed_shell_config(
    config: &ShellLaunchConfig,
    root: &std::path::Path,
    deny_network: bool,
    _strict: bool,
) -> Option<ShellLaunchConfig> {
    let program = config.program.to_string_lossy().into_owned();
    let (wrapper, args) = linux_command(&program, &config.args, root, deny_network, None)?;
    Some(ShellLaunchConfig {
        program: std::path::PathBuf::from(wrapper),
        args,
        is_posix: config.is_posix,
    })
}

#[cfg(target_os = "linux")]
pub(crate) fn sandboxed_exec(
    program: &str,
    args: &[String],
    root: &std::path::Path,
    deny_network: bool,
    _strict: bool,
) -> Option<(String, Vec<String>)> {
    // A direct exec has no persistent shell to steer back later, so start it
    // inside the workspace (`--chdir`); otherwise the sandbox would confine
    // writes while the command still *reads*/runs from wherever the parent
    // happened to be.
    linux_command(program, args, root, deny_network, Some(root))
}

// ---------------------------------------------------------------------------
// Windows — Job Object
// ---------------------------------------------------------------------------
//
// A Job Object cannot express "write only under the workspace" without an
// AppContainer token, and portable-pty does not expose the process-creation
// attributes needed to create one. What it *can* enforce without a helper
// process is process-tree containment: every descendant belongs to the job,
// the tree is killed when the app closes, and a runaway command cannot spawn
// an unbounded fork bomb. That is the isolation Windows gets here; the UI
// states the limitation rather than claiming full filesystem confinement.

#[cfg(windows)]
pub(crate) struct SandboxJob {
    /// Stored as an integer so `ShellSession` stays `Send + Sync`; a raw
    /// pointer is neither. `0` means "already closed".
    handle: isize,
}

#[cfg(windows)]
impl SandboxJob {
    pub(crate) fn close(&mut self) {
        if self.handle != 0 {
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(
                    self.handle as windows_sys::Win32::Foundation::HANDLE,
                );
            }
            self.handle = 0;
        }
    }
}

#[cfg(windows)]
impl Drop for SandboxJob {
    fn drop(&mut self) {
        self.close();
    }
}

/// Attach `pid` to a kill-on-close Job Object.
///
/// `max_processes` is a hard cap on the tree. The job handle is returned so the
/// session can close it (and therefore kill the tree) on eviction.
#[cfg(windows)]
pub(crate) fn attach_job_object(
    process_handle: windows_sys::Win32::Foundation::HANDLE,
    max_processes: u32,
) -> Option<SandboxJob> {
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_ACTIVE_PROCESS, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            tracing::warn!("could not create a Job Object for the terminal sandbox");
            return None;
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags =
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
        info.BasicLimitInformation.ActiveProcessLimit = max_processes;
        let ok = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const _,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        if ok == 0 {
            tracing::warn!("could not configure the terminal Job Object");
            windows_sys::Win32::Foundation::CloseHandle(job);
            return None;
        }
        if AssignProcessToJobObject(job, process_handle) == 0 {
            tracing::warn!("could not assign the shell to the terminal Job Object");
            windows_sys::Win32::Foundation::CloseHandle(job);
            return None;
        }
        Some(SandboxJob {
            handle: job as isize,
        })
    }
}

#[cfg(windows)]
pub(crate) fn sandboxed_shell_config(
    _config: &ShellLaunchConfig,
    _root: &std::path::Path,
    _deny_network: bool,
    _strict: bool,
) -> Option<ShellLaunchConfig> {
    // The Job Object is attached after spawn (the process handle is only
    // available then). Keeping the launch config untouched means the shell is
    // still the PTY's direct child.
    None
}

#[cfg(windows)]
pub(crate) fn sandboxed_exec(
    _program: &str,
    _args: &[String],
    _root: &std::path::Path,
    _deny_network: bool,
    _strict: bool,
) -> Option<(String, Vec<String>)> {
    None
}

// ---------------------------------------------------------------------------
// Windows — dangerous-path deny list
// ---------------------------------------------------------------------------
//
// A Job Object confines the process *tree* but not the filesystem: unlike
// macOS `sandbox-exec` or Linux bwrap there is no mount-level "workspace
// writes only" underneath it. The fail-closed complement is a deny list for
// the paths where a write is almost certainly destructive: the Windows
// directory (any drive spelling, verbatim `\\?\` included), the Program
// Files trees, the process-environment spellings (`%SystemRoot%`,
// `$env:Windir`), the device namespace (`\\.\`), and the machine registry
// hives (`HKLM:` / `HKLM\...`).
//
// Matching is a **best-effort** case-insensitive substring scan over the
// command text with `/` normalised to `\`, so both `C:\Windows\System32`
// (PowerShell) and `/c/Windows/System32` (Git Bash) spellings are caught.
// Wildcards (`*`, `?`) are collapsed first so `C:\Win?ows\System32` lands in
// the same bucket, and the verbatim `\\?\` namespace is denied outright.
// A text scan cannot see a path assembled at runtime (`$e="C:\Win"; ...`) or
// guarantee that a quoted command is really a filesystem write, so this is a
// cheap early rejection, not a security boundary — the Job Object's
// kill-on-close containment is the layer underneath. Legitimate toolchain
// reads do not name these paths, so refusing the command is safe; the caller
// reports the reason and does not run anything.
//
// Pure function on every platform so Linux/macOS CI can unit-test it.

/// Why a sandboxed Windows command is refused, or `None` when it names no
/// dangerous path.
pub(crate) fn windows_sandbox_denies_command(command: &str) -> Option<&'static str> {
    // Lowercase first (`System32` / `SYSTEM32` / `system32` all match), then
    // normalise Git Bash `/` spellings to `\` so one list covers both.
    let normalized = command.to_lowercase().replace('/', "\\");
    // PowerShell globs are legal path spellings (`C:\Win?ows\System32`,
    // `C:\Progra*1`). Collapsing wildcards makes them hit the literal list
    // instead of sailing through it; this is deliberately a text heuristic.
    let collapsed: String = normalized
        .chars()
        .filter(|ch| *ch != '*' && *ch != '?')
        .collect();
    // Ordered most-specific first so the reason names the real target.
    const DENIED: &[(&str, &str)] = &[
        (
            "\\\\?\\",
            "verbatim \\\\?\\ paths are denied in the sandbox (best-effort text gate)",
        ),
        (
            "\\system32",
            "writes under the Windows System32 directory are denied",
        ),
        (
            "\\syswow64",
            "writes under the Windows SysWOW64 directory are denied",
        ),
        (
            ":\\windows",
            "writes under the Windows system directory are denied",
        ),
        (":\\program files", "writes under Program Files are denied"),
        // 8.3 short names bypass the long-name spellings above
        // (`C:\PROGRA~1\…`, `C:\Windows\SYSTEM~1\…`), so they are denied too.
        (
            ":\\progra~1",
            "writes under Program Files via its short name are denied",
        ),
        (
            ":\\progra~2",
            "writes under Program Files via its short name are denied",
        ),
        (
            ":\\window~1",
            "writes under the Windows system directory via its short name are denied",
        ),
        (
            "\\system~1",
            "writes under the Windows System32 directory via its short name are denied",
        ),
        (
            "\\syswow~1",
            "writes under the Windows SysWOW64 directory via its short name are denied",
        ),
        (
            "%systemroot%",
            "references to %SystemRoot% are denied in the sandbox",
        ),
        (
            "%windir%",
            "references to %WinDir% are denied in the sandbox",
        ),
        (
            "$env:systemroot",
            "references to $env:SystemRoot are denied in the sandbox",
        ),
        (
            "$env:windir",
            "references to $env:WinDir are denied in the sandbox",
        ),
        (
            "\\\\.\\",
            "references to the Win32 device namespace (\\\\.\\) are denied",
        ),
        (
            "hklm:",
            "writes to the machine registry hive (HKLM:) are denied",
        ),
        (
            "hklm\\",
            "writes to the machine registry hive (HKLM) are denied",
        ),
    ];
    DENIED
        .iter()
        .find(|(pattern, _)| normalized.contains(pattern) || collapsed.contains(pattern))
        .map(|(_, reason)| *reason)
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
pub(crate) fn sandboxed_shell_config(
    _config: &ShellLaunchConfig,
    _root: &std::path::Path,
    _deny_network: bool,
    _strict: bool,
) -> Option<ShellLaunchConfig> {
    None
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
pub(crate) fn sandboxed_exec(
    _program: &str,
    _args: &[String],
    _root: &std::path::Path,
    _deny_network: bool,
    _strict: bool,
) -> Option<(String, Vec<String>)> {
    None
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn the_network_guard_is_opt_in() {
        let root = std::env::temp_dir();
        let write_only = sandbox_profile(&root, false, false).expect("profile");
        assert!(!write_only.contains("(deny network*)"));
        assert!(
            !write_only.contains("(deny file-read*"),
            "the default profile confines writes only"
        );
        let no_network = sandbox_profile(&root, true, false).expect("profile");
        assert!(no_network.contains("(deny network*)"));
        // The write rules must stay intact; the deny clause is appended after
        // the allow block, not spliced into it.
        assert!(no_network.contains("(deny file-write*)"));
        assert!(no_network.contains("(allow file-write*"));
    }

    #[test]
    fn the_strict_profile_denies_sensitive_reads_and_network() {
        let root = std::env::temp_dir();
        let strict = sandbox_profile(&root, false, true).expect("profile");
        assert!(
            strict.contains("(deny network*)"),
            "strict blocks network by default"
        );
        assert!(strict.contains("(deny file-read*"));
        assert!(strict.contains(".ssh"), "strict denies ~/.ssh reads");
        assert!(strict.contains(".aws"), "strict denies ~/.aws reads");
        assert!(strict.contains(".env"), "strict denies .env reads");
        assert!(
            strict.contains("(deny file-write*)"),
            "write confinement stays"
        );
    }
}

#[cfg(all(test, not(target_os = "macos")))]
mod cross_platform_tests {
    use super::{normalize_sandbox_path, sandbox_path_is_printable};

    #[test]
    fn trailing_slash_never_survives() {
        let dir = std::env::temp_dir();
        let with_slash = std::path::PathBuf::from(format!("{}/", dir.to_string_lossy()));
        let normalized = normalize_sandbox_path(&with_slash);
        assert!(!normalized.ends_with('/'), "{normalized}");
        assert!(!normalized.is_empty());
    }

    #[test]
    fn missing_path_falls_back_without_slash() {
        let missing = std::path::PathBuf::from("/definitely-not-a-mix-agent-dir-xyz/");
        let normalized = normalize_sandbox_path(&missing);
        assert_eq!(normalized, "/definitely-not-a-mix-agent-dir-xyz");
    }

    #[test]
    fn sandbox_paths_with_control_characters_are_rejected() {
        assert!(sandbox_path_is_printable("/Users/dev/project"));
        assert!(!sandbox_path_is_printable("/Users/dev/pro\nject"));
        assert!(!sandbox_path_is_printable("/tmp/\u{7}evil"));
    }

    #[test]
    fn sandbox_unavailable_reason_is_actionable() {
        // On Linux CI without bwrap/firejail this is Some(install hint);
        // on machines with a helper it is None. Either way it must not panic.
        let _ = super::sandbox_available();
        let _ = super::sandbox_unavailable_reason();
    }

    #[test]
    fn windows_dangerous_paths_are_denied() {
        use super::windows_sandbox_denies_command;
        // PowerShell spellings.
        for command in [
            "Copy-Item payload.dll C:\\Windows\\System32\\evil.dll",
            "takeown /f D:\\Windows\\System32\\drivers\\x.sys",
            "Remove-Item 'C:\\Program Files\\Vendor\\tool.exe'",
            "reg add HKLM\\SOFTWARE\\Vendor /v X /d 1",
            "New-Item HKLM:\\SOFTWARE\\Vendor",
            "$e = $env:SystemRoot; Write-Output $e",
            "Get-Content \\\\.\\PhysicalDrive0",
        ] {
            assert!(
                windows_sandbox_denies_command(command).is_some(),
                "{command} must be denied"
            );
        }
        // Git Bash `/` spellings normalise to the same deny list.
        assert!(windows_sandbox_denies_command("cp x /c/Windows/System32/x").is_some());
        // Case is irrelevant.
        assert!(windows_sandbox_denies_command("copy x C:\\WINDOWS\\system32\\x").is_some());
        // 8.3 short names must not bypass the long-name spellings.
        for command in [
            "rmdir C:\\PROGRA~1\\Vendor",
            "copy x C:\\Progra~2\\Vendor\\x",
            "del C:\\Windows\\SYSTEM~1\\x",
        ] {
            assert!(
                windows_sandbox_denies_command(command).is_some(),
                "{command} must be denied"
            );
        }
        // Wildcard spellings and the verbatim `\\?\` namespace are covered too.
        for command in [
            "del C:\\Win?ows\\System32\\x.sys",
            "del C:\\Progra*1\\Vendor\\x",
            "Get-Item \\\\?\\C:\\Windows\\System32\\x",
            "Get-Item \\\\?\\GLOBALROOT\\Device\\HarddiskVolume1\\x",
        ] {
            assert!(
                windows_sandbox_denies_command(command).is_some(),
                "{command} must be denied"
            );
        }
        // Ordinary workspace / toolchain commands pass.
        for command in [
            "cargo build",
            "npm test",
            "Copy-Item .\\a.txt .\\b.txt",
            "Get-ChildItem C:\\Users\\dev\\mix-project",
        ] {
            assert!(
                windows_sandbox_denies_command(command).is_none(),
                "{command} must be allowed"
            );
        }
    }
}
