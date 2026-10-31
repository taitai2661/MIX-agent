//! Paths the user has explicitly handed to the app.
//!
//! The WebView is denied the `fs` plugin, but the custom commands still take
//! string paths. A path that merely *looks* like it came from a dialog is
//! indistinguishable from one a compromised renderer invented, so every file
//! command asks this store first. Paths enter it from exactly three places:
//!
//! * the native pickers in [`crate::commands::picker`] (the dialog result
//!   itself, not the WebView's claim about it),
//! * project roots loaded from the database at launch, and
//! * an explicit "allow this path?" confirmation the user answered themselves.
//!
//! Entries expire so a path granted today is not still trusted weeks later.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a picker/confirmation keeps a path trusted.
///
/// Long enough to cover a working session (a save dialog followed by several
/// exports), short enough that a one-off grant does not outlive the task.
pub const DEFAULT_GRANT_TTL: Duration = Duration::from_secs(12 * 60 * 60);

#[derive(Debug)]
pub struct PathGrants {
    ttl: Duration,
    /// `None` means the grant has no expiry (project roots loaded at launch).
    entries: Mutex<HashMap<PathBuf, Option<Instant>>>,
}

impl Default for PathGrants {
    fn default() -> Self {
        Self::new(DEFAULT_GRANT_TTL)
    }
}

impl PathGrants {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Normalise a path so the grant and the later command compare the same
    /// thing: resolve symlinks and `..` when the path exists, and fall back to
    /// the canonical parent for a save destination that does not exist yet.
    fn key(path: &Path) -> PathBuf {
        if let Ok(canonical) = path.canonicalize() {
            return canonical;
        }
        match (path.parent(), path.file_name()) {
            (Some(parent), Some(name)) if !parent.as_os_str().is_empty() => {
                match parent.canonicalize() {
                    Ok(canonical) => canonical.join(name),
                    Err(_) => path.to_path_buf(),
                }
            }
            _ => path.to_path_buf(),
        }
    }

    pub fn grant(&self, path: &Path) -> PathBuf {
        let key = Self::key(path);
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        entries.insert(key.clone(), Some(Instant::now()));
        key
    }

    /// Trust a path for the lifetime of the process.
    ///
    /// Used for roots the user chose on an earlier launch and that are still
    /// the project they asked the app to work on. A time limit here would make
    /// a long-running window suddenly prompt for a folder it has been using.
    pub fn grant_permanent(&self, path: &Path) {
        let key = Self::key(path);
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        entries.insert(key, None);
    }

    /// Check a grant without consuming it. Expired entries are pruned on each
    /// lookup so a long-running process does not retain every path ever seen.
    ///
    /// A path inside a granted **folder** is also trusted: picking a project
    /// root is a statement about the whole tree, and the workspace commands
    /// resolve children against that root. A granted *file* is not a prefix of
    /// anything, so this does not widen file grants.
    pub fn is_granted(&self, path: &Path) -> bool {
        let key = Self::key(path);
        let now = Instant::now();
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        entries
            .retain(|_, granted_at| granted_at.is_none_or(|at| now.duration_since(at) < self.ttl));
        entries.contains_key(&key)
            || entries
                .keys()
                .any(|granted| granted.is_dir() && key.starts_with(granted))
    }

    /// Drop every entry. Used when storage is reset/restored so permissions
    /// scoped to the previous data set do not survive.
    pub fn clear(&self) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grants_are_exact_and_expire() {
        let dir = std::env::temp_dir().join(format!("mix-grants-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("note.txt");
        std::fs::write(&file, "hello").unwrap();

        let grants = PathGrants::new(Duration::from_millis(5));
        assert!(!grants.is_granted(&file));
        grants.grant(&file);
        assert!(grants.is_granted(&file));
        assert!(!grants.is_granted(&dir.join("other.txt")));

        std::thread::sleep(Duration::from_millis(10));
        assert!(!grants.is_granted(&file), "expired grants must not pass");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn permanent_grants_survive_the_ttl() {
        let dir = std::env::temp_dir().join(format!("mix-grants-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        let grants = PathGrants::new(Duration::from_millis(5));
        grants.grant_permanent(&dir);
        std::thread::sleep(Duration::from_millis(10));
        assert!(
            grants.is_granted(&dir),
            "a project root must not expire mid-session"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_save_destination_is_compared_by_its_parent() {
        let dir = std::env::temp_dir().join(format!("mix-grants-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("export.md");

        let grants = PathGrants::default();
        grants.grant(&target);
        assert!(grants.is_granted(&target));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn children_of_a_granted_folder_are_trusted() {
        let dir = std::env::temp_dir().join(format!("mix-grants-{}", uuid::Uuid::new_v4()));
        let nested = dir.join("src");
        std::fs::create_dir_all(&nested).unwrap();
        let file = nested.join("main.rs");
        std::fs::write(&file, "fn main() {}").unwrap();

        let grants = PathGrants::default();
        grants.grant(&dir);
        assert!(
            grants.is_granted(&nested),
            "a subdirectory is inside the grant"
        );
        assert!(
            grants.is_granted(&file),
            "a file inside the grant is trusted"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_file_grant_does_not_cover_siblings() {
        let dir = std::env::temp_dir().join(format!("mix-grants-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("picked.txt");
        let sibling = dir.join("sibling.txt");
        std::fs::write(&file, "x").unwrap();
        std::fs::write(&sibling, "y").unwrap();

        let grants = PathGrants::default();
        grants.grant(&file);
        assert!(!grants.is_granted(&sibling));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn relative_and_absolute_spellings_match_once_canonicalised() {
        let dir = std::env::temp_dir().join(format!("mix-grants-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        std::fs::write(&file, "x").unwrap();

        let grants = PathGrants::default();
        grants.grant(&file);
        let roundabout = dir.join(".").join("a.txt");
        assert!(grants.is_granted(&roundabout));
        std::fs::remove_dir_all(&dir).ok();
    }
}
