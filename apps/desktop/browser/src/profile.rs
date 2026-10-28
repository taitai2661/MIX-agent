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

/// Resolve a Chromium executable inside an extracted install root.
///
/// `root` is the directory that contains `chrome-mac/`, `chrome-win/` or
/// `chrome-linux/` — either the old bundle's `resources/chromium` or the
/// managed `<app-data>/chromium`. Several shapes are accepted so a hand-dropped
/// build still works.
pub fn find_chromium_in_root(root: &std::path::Path) -> Option<std::path::PathBuf> {
    let candidates: Vec<std::path::PathBuf> = if cfg!(target_os = "macos") {
        vec![
            root.join("chrome-mac/Chromium.app/Contents/MacOS/Chromium"),
            root.join(
                "chrome-mac/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
            ),
            root.join("Chromium.app/Contents/MacOS/Chromium"),
            root.join("Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing"),
        ]
    } else if cfg!(target_os = "windows") {
        vec![root.join("chrome-win/chrome.exe"), root.join("chrome.exe")]
    } else {
        vec![root.join("chrome-linux/chrome"), root.join("chrome")]
    };
    candidates.into_iter().find(|p| p.is_file())
}

/// Resolve the legacy bundled Chromium for the current OS.
///
/// The install model no longer ships this, but the lookup stays for one
/// migration release and for developers who still have `resources/chromium`.
pub fn find_bundled_chromium(
    app_resource_dir: Option<&std::path::Path>,
) -> Option<std::path::PathBuf> {
    let root = app_resource_dir?;
    find_chromium_in_root(&root.join("chromium"))
}

/// Resolve a managed Chromium executable in the user app-data directory.
pub fn find_downloaded_chromium(app_data_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    find_chromium_in_root(&app_data_dir.join("chromium"))
}

/// Developer override for testing a pre-seeded install without touching the
/// real app-data directory. `MIX_CHROMIUM_HOME` points at the directory that
/// contains `chrome-mac/`, `chrome-win/` or `chrome-linux/`.
pub fn find_override_chromium() -> Option<std::path::PathBuf> {
    std::env::var_os("MIX_CHROMIUM_HOME")
        .map(std::path::PathBuf::from)
        .and_then(|root| find_chromium_in_root(&root))
}

/// Pick the resource root to hand to [`find_bundled_chromium`].
///
/// In a packaged app Tauri's `resource_dir()` is authoritative and must win.
/// Under `cargo build` / `tauri dev` it resolves to `target/debug`, where
/// `tauri-build` has mirrored `resources/chromium` — but that mirror is not
/// faithful: `tauri-utils` tests directories with `is_dir()`, which follows
/// symlinks, so the four symlinked directories inside the macOS framework
/// (`Versions/Current`, `Helpers`, `Libraries`, `Resources`) are skipped, while
/// the one symlinked framework binary is dereferenced into a real ~247 MB file.
/// The result is a ~596 MB tree that is both larger than the source and missing
/// the layout Chromium needs.
///
/// In debug builds the pristine payload under `app/resources` is preferred
/// whenever it is present. `manifest_resources` is passed in rather than
/// derived here so this crate stays free of compile-time path assumptions, and
/// so the release path stays a plain `resource_dir()` lookup.
pub fn resolve_resource_dir(
    tauri_resource_dir: Option<&std::path::Path>,
    manifest_resources: Option<&std::path::Path>,
) -> Option<std::path::PathBuf> {
    if cfg!(debug_assertions) {
        if let Some(dir) = manifest_resources {
            if dir.is_dir() {
                return Some(dir.to_path_buf());
            }
        }
    }
    tauri_resource_dir.map(std::path::Path::to_path_buf)
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

    /// The candidate list must actually match Chrome for Testing's archive
    /// layout, or the managed install silently falls back to a system browser.
    #[test]
    fn resolves_the_chrome_for_testing_layout() {
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

    fn scratch_dir(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("mixd-res-{}", tag))
    }

    /// A debug build must ignore the `target/debug` mirror that `tauri-build`
    /// produces, because that mirror loses the macOS framework symlinks.
    #[test]
    #[cfg(debug_assertions)]
    fn debug_prefers_manifest_resources_over_tauri_mirror() {
        let root = scratch_dir("prefer");
        let mirror = root.join("target-mirror");
        let manifest = root.join("manifest");
        std::fs::create_dir_all(&mirror).expect("mkdir mirror");
        std::fs::create_dir_all(&manifest).expect("mkdir manifest");

        assert_eq!(
            resolve_resource_dir(Some(&mirror), Some(&manifest)),
            Some(manifest),
            "debug builds must prefer the pristine app/resources tree"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A clean checkout has no `app/resources/chromium`; the Tauri lookup is
    /// then the only source and must still be honoured.
    #[test]
    #[cfg(debug_assertions)]
    fn missing_manifest_resources_falls_back_to_tauri_dir() {
        let root = scratch_dir("fallback");
        let mirror = root.join("target-mirror");
        std::fs::create_dir_all(&mirror).expect("mkdir mirror");
        let absent = root.join("does-not-exist");

        assert_eq!(
            resolve_resource_dir(Some(&mirror), Some(&absent)),
            Some(mirror),
            "a skipped download must not disable the Tauri resource dir"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Neither root available: the operator must report "no bundled browser"
    /// so it can fall back to the system install.
    #[test]
    fn no_roots_yields_none() {
        assert_eq!(resolve_resource_dir(None, None), None);
    }

    /// Release builds must never prefer the source tree — the packaged app only
    /// ships what Tauri placed next to the executable.
    #[test]
    #[cfg(not(debug_assertions))]
    fn release_always_uses_tauri_resource_dir() {
        let root = scratch_dir("release");
        let manifest = root.join("manifest");
        std::fs::create_dir_all(&manifest).expect("mkdir manifest");
        assert_eq!(
            resolve_resource_dir(None, Some(&manifest)),
            None,
            "release must not read the source tree"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
