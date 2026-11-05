//! Launch-at-login support.
//!
//! The scheduler only runs while the process is alive. Closing the window
//! already keeps it in the tray (see `lib.rs`), but after a reboot nothing
//! starts it again — so a daily schedule silently never fired until the user
//! opened the app. This module installs the smallest OS-native autostart entry
//! and starts the app hidden, which is what makes "scheduled task" mean the
//! same thing as it does on the Docker edition.
//!
//! No Tauri plugin: the three platforms need three tiny files/keys, and the
//! plugin would add an IPC surface (`autostart:default`) the WebView does not
//! need. The commands in `commands/autostart.rs` are the only entry points.

use std::path::PathBuf;

/// Marker argument consumed in `run()` to keep the main window hidden.
pub const HIDDEN_ARG: &str = "--hidden";

#[cfg(target_os = "macos")]
const BUNDLE_ID: &str = "io.github.taitai2661.mix-agent-desktop";

pub fn executable() -> Result<PathBuf, String> {
    std::env::current_exe().map_err(|err| format!("could not resolve the app binary: {err}"))
}

pub fn enabled() -> bool {
    platform::enabled()
}

pub fn set_enabled(value: bool) -> Result<(), String> {
    platform::set_enabled(value)
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;

    fn plist_path() -> Result<PathBuf, String> {
        let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
        Ok(PathBuf::from(home)
            .join("Library/LaunchAgents")
            .join(format!("{BUNDLE_ID}.plist")))
    }

    fn escape_xml(value: &str) -> String {
        value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    }

    pub fn enabled() -> bool {
        plist_path().map(|path| path.is_file()).unwrap_or(false)
    }

    pub fn set_enabled(value: bool) -> Result<(), String> {
        let path = plist_path()?;
        if !value {
            if path.is_file() {
                let _ = std::process::Command::new("launchctl")
                    .args(["unload", "-w"])
                    .arg(&path)
                    .status();
                std::fs::remove_file(&path)
                    .map_err(|err| format!("could not remove {}: {err}", path.display()))?;
            }
            return Ok(());
        }
        let exe = executable()?;
        let dir = path.parent().ok_or("invalid LaunchAgents path")?;
        std::fs::create_dir_all(dir)
            .map_err(|err| format!("could not create {}: {err}", dir.display()))?;
        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{label}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe}</string>
    <string>{hidden}</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><false/>
</dict>
</plist>
"#,
            label = escape_xml(BUNDLE_ID),
            exe = escape_xml(&exe.to_string_lossy()),
            hidden = HIDDEN_ARG,
        );
        std::fs::write(&path, plist)
            .map_err(|err| format!("could not write {}: {err}", path.display()))?;
        // Best effort: the file alone is enough at the next login, and
        // `launchctl` can refuse inside managed environments.
        let _ = std::process::Command::new("launchctl")
            .args(["load", "-w"])
            .arg(&path)
            .status();
        Ok(())
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use super::*;
    use std::process::Command;

    const APP_NAME: &str = "MIX-agent Desktop";
    const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

    pub fn enabled() -> bool {
        Command::new("reg")
            .args(["query", RUN_KEY, "/v", APP_NAME])
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false)
    }

    pub fn set_enabled(value: bool) -> Result<(), String> {
        if !value {
            // A missing value makes `reg delete` fail; that is the state we
            // want, not an error worth surfacing.
            let _ = Command::new("reg")
                .args(["delete", RUN_KEY, "/v", APP_NAME, "/f"])
                .status();
            return Ok(());
        }
        let exe = executable()?;
        let command = format!("\"{}\" {}", exe.display(), HIDDEN_ARG);
        let status = Command::new("reg")
            .args([
                "add", RUN_KEY, "/v", APP_NAME, "/t", "REG_SZ", "/d", &command, "/f",
            ])
            .status()
            .map_err(|err| format!("could not run reg.exe: {err}"))?;
        if !status.success() {
            return Err("reg.exe could not write the Run key".to_string());
        }
        Ok(())
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod platform {
    use super::*;

    const APP_NAME: &str = "MIX-agent Desktop";

    fn desktop_path() -> Result<PathBuf, String> {
        let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
        Ok(PathBuf::from(home)
            .join(".config/autostart")
            .join("mix-agent-desktop.desktop"))
    }

    fn quote_exec(value: &str) -> String {
        // Desktop Entry Exec quoting: wrap when the path contains whitespace
        // and escape embedded quotes/backslashes.
        if value
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '"' | '\'' | '\\'))
        {
            format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
        } else {
            value.to_string()
        }
    }

    pub fn enabled() -> bool {
        desktop_path().map(|path| path.is_file()).unwrap_or(false)
    }

    pub fn set_enabled(value: bool) -> Result<(), String> {
        let path = desktop_path()?;
        if !value {
            if path.is_file() {
                std::fs::remove_file(&path)
                    .map_err(|err| format!("could not remove {}: {err}", path.display()))?;
            }
            return Ok(());
        }
        let exe = executable()?;
        let dir = path.parent().ok_or("invalid autostart path")?;
        std::fs::create_dir_all(dir)
            .map_err(|err| format!("could not create {}: {err}", dir.display()))?;
        // Desktop Entry field codes: a literal `%` in the path must be doubled.
        let entry = format!(
            "[Desktop Entry]\nType=Application\nName={APP_NAME}\nExec={} {HIDDEN_ARG}\nX-GNOME-Autostart-enabled=true\nTerminal=false\n",
            quote_exec(&exe.to_string_lossy()).replace('%', "%%")
        );
        std::fs::write(&path, entry)
            .map_err(|err| format!("could not write {}: {err}", path.display()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_arg_is_stable() {
        // The installer writes this literal into plists/desktop files; changing
        // it without updating `run()` would start the window visibly.
        assert_eq!(HIDDEN_ARG, "--hidden");
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn exec_quoting_survives_spaces() {
        assert_eq!(platform::quote_exec("/opt/MIX agent"), "\"/opt/MIX agent\"");
        assert_eq!(platform::quote_exec("/usr/bin/mix"), "/usr/bin/mix");
    }
}
