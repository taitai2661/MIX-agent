//! Workspace-scoped Browser Profile.
//!
//! Each workspace gets its own on-disk user-data directory so cookies,
//! localStorage, and logins persist between runs. The directory lives under
//! the OS app-data directory, never inside the repository.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserProfile {
    pub workspace_id: uuid::Uuid,
    pub display_name: String,
    /// Absolute path to the Chromium user-data directory.
    pub directory: String,
    /// Whether the profile has been materialised on disk.
    pub provisioned: bool,
}

impl BrowserProfile {
    pub fn new(workspace_id: uuid::Uuid, display_name: &str, root: &std::path::Path) -> Self {
        let directory = root.join(workspace_id.to_string());
        Self {
            workspace_id,
            display_name: display_name.to_string(),
            directory: directory.to_string_lossy().to_string(),
            provisioned: false,
        }
    }

    pub fn path(&self) -> std::path::PathBuf {
        std::path::PathBuf::from(&self.directory)
    }

    /// Create the profile directory if it does not exist yet.
    pub async fn ensure(&mut self) -> anyhow::Result<()> {
        if !self.path().is_dir() {
            tokio::fs::create_dir_all(self.path()).await?;
            self.provisioned = true;
        } else {
            self.provisioned = true;
        }
        Ok(())
    }

    /// Wipe cookies and storage without deleting the directory itself.
    pub async fn reset(&self) -> anyhow::Result<()> {
        let path = self.path();
        if path.is_dir() {
            tokio::fs::remove_dir_all(&path).await?;
        }
        Ok(())
    }
}

/// Default root for browser profiles: `<app-data>/browser`.
pub fn default_profile_root(app_data: &std::path::Path) -> std::path::PathBuf {
    app_data.join("browser")
}

/// Resolve a bundled Chromium executable for the current OS.
///
/// The layout is produced by `scripts/fetch-chromium.mjs`. Several shapes are
/// accepted so a hand-dropped build (or a Tauri resource remap) still works.
/// Returns the first path that exists; when none is present the caller is
/// expected to fall back to a system-installed browser.
pub fn find_bundled_chromium(
    app_resource_dir: Option<&std::path::Path>,
) -> Option<std::path::PathBuf> {
    let root = app_resource_dir?;
    let candidates: Vec<std::path::PathBuf> = if cfg!(target_os = "macos") {
        vec![
            root.join("chromium/chrome-mac/Chromium.app/Contents/MacOS/Chromium"),
            root.join("chromium/chrome-mac/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing"),
            root.join("chromium/Chromium.app/Contents/MacOS/Chromium"),
        ]
    } else if cfg!(target_os = "windows") {
        vec![
            root.join("chromium/chrome-win/chrome.exe"),
            root.join("chromium/chrome.exe"),
        ]
    } else {
        vec![
            root.join("chromium/chrome-linux/chrome"),
            root.join("chromium/chrome"),
        ]
    };
    candidates.into_iter().find(|p| p.is_file())
}

/// System Chromium fallback when no bundled build is present.
pub fn find_system_chromium() -> Option<std::path::PathBuf> {
    let names: &[&str] = if cfg!(target_os = "macos") {
        &[
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        ]
    } else if cfg!(target_os = "windows") {
        &[
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        ]
    } else {
        &[
            "/usr/bin/chromium",
            "/usr/bin/chromium-browser",
            "/usr/bin/google-chrome",
            "/snap/bin/chromium",
        ]
    };
    names
        .iter()
        .map(std::path::PathBuf::from)
        .find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_path_is_namespaced_by_workspace() {
        let id = uuid::Uuid::new_v4();
        let root = std::path::Path::new("/tmp/mixd-browser-test");
        let profile = BrowserProfile::new(id, "demo", root);
        assert!(profile.directory.ends_with(&id.to_string()));
        assert_eq!(profile.path(), root.join(id.to_string()));
    }

    #[test]
    fn missing_resource_dir_is_none() {
        assert!(find_bundled_chromium(None).is_none());
    }

    #[test]
    fn unknown_layout_falls_through_to_none() {
        let dir = std::env::temp_dir().join(format!("mixd-bundle-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("chromium/empty")).expect("mkdir");
        assert!(find_bundled_chromium(Some(&dir)).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The candidate list must actually match what `fetch-chromium.mjs` writes,
    /// or the packaged app silently falls back to a system browser.
    #[test]
    fn resolves_the_fetch_script_layout() {
        use std::path::PathBuf;
        let dir: PathBuf =
            std::env::temp_dir().join(format!("mixd-bundle-{}", uuid::Uuid::new_v4()));
        let rel = if cfg!(target_os = "macos") {
            "chromium/chrome-mac/Chromium.app/Contents/MacOS/Chromium"
        } else if cfg!(target_os = "windows") {
            "chromium/chrome-win/chrome.exe"
        } else {
            "chromium/chrome-linux/chrome"
        };
        let expected = dir.join(rel);
        std::fs::create_dir_all(expected.parent().expect("parent")).expect("mkdir");
        std::fs::write(&expected, b"#!/bin/sh\n").expect("write");

        assert_eq!(
            find_bundled_chromium(Some(&dir)).as_deref(),
            Some(expected.as_path()),
            "resolver and fetch script disagree on the layout"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
