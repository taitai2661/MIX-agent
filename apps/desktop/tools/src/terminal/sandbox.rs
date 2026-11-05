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
pub(crate) fn sandbox_profile(root: &std::path::Path, deny_network: bool) -> Option<String> {
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
    // The write sandbox deliberately leaves the network alone because package
    // installs and tests need it. Users who run untrusted code can opt into
    // the stricter profile, which is still not a substitute for a VM.
    if deny_network {
        profile.push_str("(deny network*)\n");
    }
    Some(profile)
}

#[cfg(target_os = "macos")]
pub(crate) fn sandboxed_shell_config(
    config: &ShellLaunchConfig,
    root: &std::path::Path,
    deny_network: bool,
) -> Option<ShellLaunchConfig> {
    let profile = sandbox_profile(root, deny_network)?;
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
) -> Option<(String, Vec<String>)> {
    let profile = sandbox_profile(root, deny_network)?;
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
            let mut out = vec![
                "--quiet".into(),
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
) -> Option<(String, Vec<String>)> {
    linux_command(program, args, root, deny_network, None)
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
) -> Option<(String, Vec<String>)> {
    None
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
pub(crate) fn sandboxed_shell_config(
    _config: &ShellLaunchConfig,
    _root: &std::path::Path,
    _deny_network: bool,
) -> Option<ShellLaunchConfig> {
    None
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
pub(crate) fn sandboxed_exec(
    _program: &str,
    _args: &[String],
    _root: &std::path::Path,
    _deny_network: bool,
) -> Option<(String, Vec<String>)> {
    None
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn the_network_guard_is_opt_in() {
        let root = std::env::temp_dir();
        let write_only = sandbox_profile(&root, false).expect("profile");
        assert!(!write_only.contains("(deny network*)"));
        let no_network = sandbox_profile(&root, true).expect("profile");
        assert!(no_network.contains("(deny network*)"));
        // The write rules must stay intact; the deny clause is appended after
        // the allow block, not spliced into it.
        assert!(no_network.contains("(deny file-write*)"));
        assert!(no_network.contains("(allow file-write*"));
    }
}
