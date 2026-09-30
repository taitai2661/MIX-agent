//! Codex-compatible `AGENTS.md` discovery.
//!
//! Mirrors the rules documented at
//! <https://developers.openai.com/codex/agent-configuration/agents-md>:
//!
//! 1. **Global scope** — read the first non-empty file from
//!    `<codex_home>/AGENTS.override.md` or `<codex_home>/AGENTS.md`.
//! 2. **Project scope** — starting at the project root (typically the git
//!    root) and walking down to `cwd_in_project`, in each directory take the
//!    first non-empty file from `AGENTS.override.md`, then `AGENTS.md`, then
//!    every entry in `fallback_filenames`.
//! 3. Concatenate root → leaf with blank lines between files.
//! 4. Stop once the combined size reaches `max_bytes`. A run is allowed to
//!    proceed with a truncated block; the truncation is recorded so the
//!    caller can surface a warning.

use std::path::{Path, PathBuf};

/// Default byte cap, matching Codex's `project_doc_max_bytes = 32 KiB`.
pub const DEFAULT_MAX_BYTES: u64 = 32 * 1024;

/// Filenames consulted first at every directory level.
const PRIMARY_FILENAMES: [&str; 2] = ["AGENTS.override.md", "AGENTS.md"];

/// The discovered instruction chain for a single run.
#[derive(Debug, Clone, Default)]
pub struct ProjectInstructions {
    /// Either the global `AGENTS.override.md` or `AGENTS.md`, if either
    /// exists at `<codex_home>` and is non-empty. Stored separately so the
    /// caller can place it in a distinct section of the system prompt.
    pub global_block: Option<String>,
    /// The root → leaf chain (excluding the global block), with blank
    /// lines between files.
    pub project_chain: String,
    /// Combined byte size of `global_block` + `project_chain` (utf-8 length).
    pub total_bytes: usize,
    /// True when the chain hit the byte cap before all files were read.
    pub truncated: bool,
}

/// Walk the project tree and concatenate every `AGENTS.md` (or fallback)
/// file into a single block. `cwd_in_project` is typically the agent's
/// working directory inside the project; if `None` the project root is the
/// only level consulted. `codex_home: None` skips the global block (tests)
/// but still reads the project chain.
pub fn load_project_instructions(
    codex_home: Option<&Path>,
    project_root: &Path,
    cwd_in_project: Option<&Path>,
    fallback_filenames: &[&str],
    max_bytes: u64,
) -> ProjectInstructions {
    let mut out = ProjectInstructions::default();
    out.global_block = codex_home.and_then(|home| {
        let block = read_first_existing(home, &PRIMARY_FILENAMES, &[] as &[&str]);
        if let Some((_, content)) = &block {
            out.total_bytes += content.len();
        }
        block.map(|(_, content)| content)
    });

    let levels = chain_levels(project_root, cwd_in_project);
    let mut parts: Vec<String> = Vec::new();
    for dir in levels {
        let Some((name, content)) = read_first_existing(&dir, &PRIMARY_FILENAMES, fallback_filenames)
        else {
            continue;
        };
        if content.trim().is_empty() {
            continue;
        }
        parts.push(format!("# {}/{}\n\n{}", dir.display(), name, content));
    }
    let joined = parts.join("\n\n");
    let remaining = max_bytes.saturating_sub(out.total_bytes as u64);
    let (taken, truncated) = truncate_to(&joined, remaining);
    out.project_chain = taken;
    out.total_bytes += out.project_chain.len();
    out.truncated = truncated || out.truncated;
    out
}

/// The directory the chain starts from. Codex walks "from the project root
/// (typically the Git root) down to your current working directory", so a
/// `.git` marker above `start` widens the chain; otherwise `start` itself is
/// the root. `.git` may be a directory (normal) or a file (worktrees and
/// submodules).
pub fn project_root_marker(start: &Path) -> std::path::PathBuf {
    let start = start.canonicalize().unwrap_or_else(|_| start.to_path_buf());
    let mut cursor = start.clone();
    loop {
        if cursor.join(".git").exists() {
            return cursor;
        }
        match cursor.parent() {
            Some(parent) if parent != cursor => cursor = parent.to_path_buf(),
            _ => return start,
        }
    }
}

/// Build the directory chain from `project_root` down to `cwd_in_project`
/// (inclusive). If `cwd_in_project` is not actually inside `project_root`,
/// only the root is returned. The root is always included.
///
/// Both sides are canonicalized first: [`project_root_marker`] returns a
/// canonical path, while callers usually hold a raw one (`/var/...` on macOS
/// is a symlink to `/private/var/...`), and `strip_prefix` would otherwise
/// silently drop the whole chain.
fn chain_levels(project_root: &Path, cwd_in_project: Option<&Path>) -> Vec<PathBuf> {
    let root = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    let mut levels = vec![root.clone()];
    let Some(cwd) = cwd_in_project else {
        return levels;
    };
    let cwd = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    let Ok(rel) = cwd.strip_prefix(&root) else {
        return levels;
    };
    let mut current = root;
    for component in rel.components() {
        current.push(component.as_os_str());
        levels.push(current.clone());
    }
    levels
}

/// Read the first existing, non-empty file among the candidates and the
/// fallback list. Returns the file's name and contents; `None` if nothing
/// matches.
fn read_first_existing(dir: &Path, primary: &[&str], fallback: &[&str]) -> Option<(String, String)> {
    for name in primary.iter().chain(fallback.iter()) {
        let path = dir.join(name);
        match std::fs::read_to_string(&path) {
            Ok(content) if !content.trim().is_empty() => {
                return Some((name.to_string(), content))
            }
            Ok(_) => continue,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => {
                tracing::warn!(path = %path.display(), %err, "instruction file could not be read");
                continue;
            }
        }
    }
    None
}

/// Truncate `s` to at most `max_bytes` bytes on a char boundary. Returns the
/// possibly-truncated string and a flag indicating whether truncation
/// occurred. A `max_bytes` of zero yields an empty string with
/// `truncated = true` whenever the input was non-empty.
fn truncate_to(s: &str, max_bytes: u64) -> (String, bool) {
    if s.len() as u64 <= max_bytes {
        return (s.to_string(), false);
    }
    let mut end = max_bytes as usize;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    (s[..end].to_string(), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, body: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(name), body).unwrap();
    }

    fn temp_root(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("mixd-instructions-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn picks_override_over_plain_agents_md() {
        let root = temp_root("override");
        write(&root, "AGENTS.override.md", "override body");
        write(&root, "AGENTS.md", "plain body");
        let home = root.join("home");
        std::fs::create_dir_all(&home).unwrap();

        let out = load_project_instructions(Some(&home), &root, None, &[], 1024);
        assert_eq!(out.global_block, None);
        assert!(out.project_chain.contains("override body"));
        assert!(!out.project_chain.contains("plain body"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn global_block_is_first_non_empty_file() {
        let root = temp_root("global");
        let home = root.join("home");
        std::fs::create_dir_all(&home).unwrap();
        write(&home, "AGENTS.override.md", "");
        write(&home, "AGENTS.md", "global body");

        let out = load_project_instructions(Some(&home), &root, None, &[], 1024);
        assert_eq!(out.global_block.as_deref(), Some("global body"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn chain_merges_root_to_leaf() {
        let root = temp_root("chain");
        write(&root, "AGENTS.md", "root-level rules");
        write(&root.join("services/payments"), "AGENTS.md", "payments rules");

        let cwd = root.join("services/payments");
        let out =
            load_project_instructions(Some(&root.join("home")), &root, Some(&cwd), &[], 4096);
        let root_pos = out.project_chain.find("root-level rules").unwrap();
        let leaf_pos = out.project_chain.find("payments rules").unwrap();
        assert!(root_pos < leaf_pos, "root must precede leaf");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn cwd_outside_root_is_ignored() {
        let root = temp_root("outside");
        write(&root, "AGENTS.md", "root rules");
        let cwd = std::env::temp_dir();
        let out =
            load_project_instructions(Some(&root.join("home")), &root, Some(&cwd), &[], 4096);
        assert_eq!(out.project_chain.matches("root rules").count(), 1);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn fallback_filenames_are_consulted_after_primary() {
        let root = temp_root("fallback");
        write(&root, "TEAM_GUIDE.md", "team rules");
        let out = load_project_instructions(
            Some(&root.join("home")),
            &root,
            None,
            &["TEAM_GUIDE.md"],
            4096,
        );
        assert!(out.project_chain.contains("team rules"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn byte_cap_truncates_and_marks_truncated() {
        let root = temp_root("cap");
        let big = "x".repeat(2048);
        write(&root, "AGENTS.md", &big);
        let out = load_project_instructions(Some(&root.join("home")), &root, None, &[], 100);
        assert!(out.truncated);
        assert!(out.project_chain.len() <= 100);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn empty_files_are_skipped() {
        let root = temp_root("empty");
        write(&root, "AGENTS.md", "   \n  \n");
        let out = load_project_instructions(Some(&root.join("home")), &root, None, &[], 4096);
        assert_eq!(out.project_chain, "");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn truncate_to_respects_char_boundaries() {
        let s = "こんにちはworld";
        let (taken, trunc) = truncate_to(s, 5);
        // 5 bytes sits inside the first character; we must land on a boundary.
        assert!(trunc);
        assert!(s.is_char_boundary(taken.len()));
    }

    #[test]
    fn no_global_home_still_reads_the_project_chain() {
        let root = temp_root("noglobal");
        write(&root, "AGENTS.md", "project rules");
        let out = load_project_instructions(None, &root, None, &[], 4096);
        assert_eq!(out.global_block, None);
        assert!(out.project_chain.contains("project rules"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn project_root_marker_finds_the_nearest_git_root() {
        let root = temp_root("marker");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        let nested = root.join("services/payments");
        std::fs::create_dir_all(&nested).unwrap();

        assert_eq!(project_root_marker(&nested), root.canonicalize().unwrap());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn project_root_marker_treats_a_git_file_as_a_marker() {
        // Worktrees and submodules keep `.git` as a file, not a directory.
        let root = temp_root("nogit");
        let nested = root.join("leaf");
        std::fs::create_dir_all(&nested).unwrap();
        // No marker anywhere above: the start directory is its own root.
        assert_eq!(
            project_root_marker(&nested),
            nested.canonicalize().unwrap()
        );

        std::fs::write(root.join(".git"), "gitdir: /elsewhere").unwrap();
        assert_eq!(project_root_marker(&nested), root.canonicalize().unwrap());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn marker_widened_chain_walks_git_root_to_project_folder() {
        let root = temp_root("widen");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        write(&root, "AGENTS.md", "repo-wide rules");
        let nested = root.join("services/payments");
        write(&nested, "AGENTS.md", "payments rules");

        let chain_root = project_root_marker(&nested);
        let out = load_project_instructions(None, &chain_root, Some(&nested), &[], 4096);
        let root_pos = out.project_chain.find("repo-wide rules").unwrap();
        let leaf_pos = out.project_chain.find("payments rules").unwrap();
        assert!(root_pos < leaf_pos, "repo root must precede leaf");
        std::fs::remove_dir_all(&root).ok();
    }
}
