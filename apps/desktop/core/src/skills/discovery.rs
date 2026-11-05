//! Discovery of skills that live in the project folder or the user's home
//! directory rather than the database.
//!
//! A repository keeps its own procedures next to the code they describe, so
//! the agent should find them without an import step. The directory names are
//! the ones other coding agents already use: `.mix/skills` first (ours), then
//! `.claude/skills` and `.agents/skills`.
//!
//! Within one scope only the first existing directory is read. Merging all
//! three would let a repository accidentally contribute a skill it did not
//! author, and makes the effective catalogue depend on which files happen to
//! be present. Across scopes the order is project → user → stored, so a
//! repository can override a personal skill of the same name.
//!
//! Discovery is read-only: packages are offered to the prompt and to the
//! `skill` / `skill_resource` tools as they are, never written to the
//! database, so editing a `SKILL.md` on disk takes effect on the next run.

use std::path::{Path, PathBuf};

use base64::Engine as _;
use mixd_tools::skill::{SkillResourceData, SkillSource, SkillSummary};

use super::{parse_skill, Skill, SkillResource};

/// Candidate skill roots inside a project, most preferred first.
pub const SKILL_ROOTS: [&str; 3] = [".mix/skills", ".claude/skills", ".agents/skills"];

/// Candidate skill roots inside the user's home, most preferred first.
pub const USER_SKILL_ROOTS: [&str; 3] = [".mix/skills", ".claude/skills", ".agents/skills"];

/// Per-file and per-skill caps, matching the Server package limits.
pub const FILE_MAX: usize = 262_144;
pub const FILES_TOTAL_MAX: usize = 2 * 1024 * 1024;
pub const FILES_COUNT_MAX: usize = 100;

/// One `SKILL.md` that could not be used, kept so the UI can show why.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct SkillFailure {
    pub path: String,
    pub reason: String,
}

/// The one project skill directory that applies, if any.
pub fn skill_root(project_root: &Path) -> Option<PathBuf> {
    first_existing(project_root, &SKILL_ROOTS)
}

/// The one user-level skill directory that applies, if any.
pub fn user_skill_root() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    user_skill_root_in(&home)
}

fn user_skill_root_in(home: &Path) -> Option<PathBuf> {
    first_existing(home, &USER_SKILL_ROOTS)
}

fn first_existing(base: &Path, roots: &[&str]) -> Option<PathBuf> {
    roots
        .iter()
        .map(|rel| base.join(rel))
        .find(|dir| dir.is_dir())
}

/// Every skill under `project_root`'s skill directory.
///
/// A `SKILL.md` that does not parse is skipped with a log line rather than
/// failing the run: one malformed file in a repository must not disable the
/// rest, and the user can still import it through the UI to see the error.
/// [`discover_report`] returns the reasons for callers that want to show them.
pub fn load_project_skills(project_root: &Path) -> Vec<Skill> {
    load_project_report(project_root).0
}

pub fn load_project_report(project_root: &Path) -> (Vec<Skill>, Vec<SkillFailure>) {
    match skill_root(project_root) {
        Some(dir) => load_dir(&dir),
        None => (Vec::new(), Vec::new()),
    }
}

/// Every user-level skill, from the first existing home directory.
pub fn load_user_skills() -> Vec<Skill> {
    load_user_report().0
}

pub fn load_user_report() -> (Vec<Skill>, Vec<SkillFailure>) {
    match user_skill_root() {
        Some(dir) => load_dir(&dir),
        None => (Vec::new(), Vec::new()),
    }
}

/// Project then user skills, first name winning.
///
/// Project-first is deliberate: a repository's procedure is more specific than
/// a personal one with the same name, and the resolution must be deterministic
/// rather than dependent on directory iteration order.
pub fn load_all_skills(project_root: Option<&Path>) -> Vec<Skill> {
    let project = project_root.map(load_project_skills).unwrap_or_default();
    merge_skills(project, load_user_skills())
}

fn merge_skills(mut skills: Vec<Skill>, user_skills: Vec<Skill>) -> Vec<Skill> {
    for skill in user_skills {
        if !skills.iter().any(|existing| existing.name == skill.name) {
            skills.push(skill);
        }
    }
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

/// Project and user skills plus the reasons individual files failed.
pub fn discover_report(project_root: Option<&Path>) -> (Vec<Skill>, Vec<SkillFailure>) {
    let (skills, mut failures) = match project_root {
        Some(root) => load_project_report(root),
        None => (Vec::new(), Vec::new()),
    };
    let (user_skills, mut user_failures) = load_user_report();
    failures.append(&mut user_failures);
    (merge_skills(skills, user_skills), failures)
}

fn load_dir(dir: &Path) -> (Vec<Skill>, Vec<SkillFailure>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        tracing::warn!(path = %dir.display(), "skill directory is unreadable");
        return (Vec::new(), Vec::new());
    };
    let mut skills = Vec::new();
    let mut failures = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        // `is_symlink` is checked before `is_dir` so a link out of the skill
        // root cannot turn a repository into an arbitrary-file reader.
        if path.is_symlink() || !path.is_dir() {
            continue;
        }
        let file = path.join("SKILL.md");
        if file.is_symlink() || !file.is_file() {
            continue;
        }
        let raw = match std::fs::read_to_string(&file) {
            Ok(raw) => raw,
            Err(err) => {
                tracing::warn!(path = %file.display(), %err, "skill file could not be read");
                failures.push(SkillFailure {
                    path: file.display().to_string(),
                    reason: err.to_string(),
                });
                continue;
            }
        };
        match parse_skill(&raw) {
            Ok(mut skill) => {
                // The specification requires the name to match the parent
                // directory. Enforcing it here keeps one directory from
                // shadowing a different skill after a copy/paste.
                let directory = path
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_default();
                if skill.name != directory {
                    let reason = format!(
                        "skill name {:?} must match its directory name {:?}",
                        skill.name, directory
                    );
                    tracing::warn!(path = %file.display(), %reason);
                    failures.push(SkillFailure {
                        path: file.display().to_string(),
                        reason,
                    });
                    continue;
                }
                skill.resources = bundle_manifest(&path);
                skills.push(skill);
            }
            Err(err) => {
                tracing::warn!(path = %file.display(), %err, "skill file is not a valid SKILL.md");
                failures.push(SkillFailure {
                    path: file.display().to_string(),
                    reason: err.message,
                });
            }
        }
    }
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    (skills, failures)
}

/// Directory/file names that are never skill resources.
///
/// A repository skill commonly sits next to `.git` or `node_modules`; walking
/// those would either exhaust the bundle cap or ship build artefacts. The
/// specification allows arbitrary files, but these are never intentional.
pub const IGNORED_NAMES: [&str; 10] = [
    ".git",
    ".hg",
    ".svn",
    ".DS_Store",
    "Thumbs.db",
    "__pycache__",
    "__MACOSX",
    "node_modules",
    ".venv",
    "venv",
];

fn is_ignored(path: &Path) -> bool {
    path.file_name().is_some_and(|name| {
        let name = name.to_string_lossy();
        IGNORED_NAMES.iter().any(|ignored| *ignored == name)
    })
}

/// Bundled files next to `SKILL.md`, as metadata only.
///
/// The catalogue and the prompt only need the paths, so discovery must not read
/// (and base64-encode) every byte of every skill on each turn. Contents are
/// fetched one file at a time by [`read_skill_resource`].
fn bundle_manifest(skill_dir: &Path) -> Vec<SkillResource> {
    let mut out = Vec::new();
    let mut total = 0usize;
    collect_manifest(skill_dir, skill_dir, &mut out, &mut total);
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

fn collect_manifest(dir: &Path, root: &Path, out: &mut Vec<SkillResource>, total: &mut usize) {
    if out.len() >= FILES_COUNT_MAX || *total > FILES_TOTAL_MAX {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    // Directory order is filesystem-defined; sorting makes the catalogue and
    // the exported archive deterministic.
    paths.sort();
    for path in paths {
        if out.len() >= FILES_COUNT_MAX || *total > FILES_TOTAL_MAX {
            return;
        }
        if is_ignored(&path) || path.is_symlink() {
            continue;
        }
        if path.is_dir() {
            collect_manifest(&path, root, out, total);
            continue;
        }
        if !path.is_file() {
            continue;
        }
        if path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("skill.md"))
        {
            continue;
        }
        let Ok(metadata) = path.metadata() else {
            continue;
        };
        let size = metadata.len() as usize;
        if size > FILE_MAX {
            tracing::warn!(path = %path.display(), "bundled file exceeds the size limit; skipped");
            continue;
        }
        *total += size;
        if *total > FILES_TOTAL_MAX {
            tracing::warn!(path = %root.display(), "bundled files exceed the total size limit");
            return;
        }
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let relative = relative.to_string_lossy().replace('\\', "/");
        out.push(SkillResource {
            path: relative,
            content: String::new(),
            binary: false,
            size,
        });
    }
}

fn encode_file(path: String, raw: Vec<u8>) -> SkillResource {
    let size = raw.len();
    match String::from_utf8(raw) {
        Ok(content) => SkillResource {
            path,
            content,
            binary: false,
            size,
        },
        Err(err) => SkillResource {
            path,
            content: base64::engine::general_purpose::STANDARD.encode(err.as_bytes()),
            binary: true,
            size,
        },
    }
}

fn summary(skill: &Skill) -> SkillSummary {
    SkillSummary {
        name: skill.name.clone(),
        description: skill.description.clone(),
        body_markdown: skill.body_markdown.clone(),
        license: skill.license.clone(),
        compatibility: skill.compatibility.clone(),
        metadata: skill.metadata.clone(),
        allowed_tools: skill.allowed_tools.clone(),
        resources: skill.resources.iter().map(|r| r.path.clone()).collect(),
    }
}

/// Read one bundled file without scanning or reading the rest of the skill.
///
/// `skill` must be a single directory component and `path` a relative path with
/// no parent traversal. The resolved file must still be under the skill
/// directory, so a symlink planted inside a repository cannot turn this into an
/// arbitrary-file reader. The same caps as an import apply.
pub fn read_skill_resource(root: &Path, skill: &str, path: &str) -> Option<SkillResourceData> {
    let skill = skill.trim();
    if skill.is_empty()
        || skill.contains('/')
        || skill.contains('\\')
        || skill == "."
        || skill == ".."
    {
        return None;
    }
    let dir = root.join(skill);
    if dir.is_symlink() || !dir.is_dir() {
        return None;
    }
    let relative = Path::new(path);
    if relative.is_absolute()
        || relative
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        return None;
    }
    if relative.components().any(|part| {
        IGNORED_NAMES
            .iter()
            .any(|ignored| part.as_os_str() == *ignored)
    }) {
        return None;
    }
    if relative
        .file_name()
        .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("skill.md"))
    {
        return None;
    }
    let file = dir.join(relative);
    if file.is_symlink() || !file.is_file() {
        return None;
    }
    let canonical_dir = dir.canonicalize().ok()?;
    let canonical_file = file.canonicalize().ok()?;
    if !canonical_file.starts_with(&canonical_dir) {
        return None;
    }
    let raw = std::fs::read(&canonical_file).ok()?;
    if raw.len() > FILE_MAX {
        return None;
    }
    let resource = encode_file(path.to_string(), raw);
    Some(SkillResourceData {
        path: resource.path,
        content: resource.content,
        binary: resource.binary,
        size: resource.size,
    })
}

fn resource_from(root: &Path, skill: &str, path: &str) -> Option<SkillResourceData> {
    read_skill_resource(root, skill, path)
}

/// A [`SkillSource`] backed by the project folder.
///
/// Returns nothing without a project root: there is no fallback to the process
/// working directory, which for a Finder-launched app is unrelated to whatever
/// the user meant to work on.
pub struct ProjectSkillSource;

#[async_trait::async_trait]
impl SkillSource for ProjectSkillSource {
    async fn skills(&self, root: Option<&str>) -> anyhow::Result<Vec<SkillSummary>> {
        let Some(root) = root.filter(|r| !r.trim().is_empty()) else {
            return Ok(Vec::new());
        };
        let path = Path::new(root).to_path_buf();
        if !path.is_dir() {
            return Ok(Vec::new());
        }
        // Discovery walks the filesystem. Doing that directly on the Tokio
        // worker blocks every other Run while a large repository is scanned.
        let skills = tokio::task::spawn_blocking(move || load_project_skills(&path)).await?;
        Ok(skills.iter().map(summary).collect())
    }

    async fn resource(
        &self,
        root: Option<&str>,
        skill: &str,
        path: &str,
    ) -> anyhow::Result<Option<SkillResourceData>> {
        let Some(root) = root.filter(|r| !r.trim().is_empty()) else {
            return Ok(None);
        };
        let root = Path::new(root).to_path_buf();
        let skill = skill.to_string();
        let path = path.to_string();
        Ok(tokio::task::spawn_blocking(move || {
            let dir = skill_root(&root)?;
            resource_from(&dir, &skill, &path)
        })
        .await?)
    }
}

/// A [`SkillSource`] backed by the user's home directory.
///
/// Personal skills apply regardless of the open project, so this source
/// ignores `root`. It is a separate source from [`ProjectSkillSource`] so the
/// app can disable it without touching project discovery.
pub struct UserSkillSource;

#[async_trait::async_trait]
impl SkillSource for UserSkillSource {
    async fn skills(&self, _root: Option<&str>) -> anyhow::Result<Vec<SkillSummary>> {
        let skills = tokio::task::spawn_blocking(load_user_skills).await?;
        Ok(skills.iter().map(summary).collect())
    }

    async fn resource(
        &self,
        _root: Option<&str>,
        skill: &str,
        path: &str,
    ) -> anyhow::Result<Option<SkillResourceData>> {
        let skill = skill.to_string();
        let path = path.to_string();
        Ok(tokio::task::spawn_blocking(move || {
            let dir = user_skill_root()?;
            resource_from(&dir, &skill, &path)
        })
        .await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn write(path: &Path, raw: &str) {
        std::fs::create_dir_all(path).expect("mkdir");
        std::fs::write(path.join("SKILL.md"), raw).expect("write");
    }

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mix-skill-discovery-{tag}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    #[test]
    fn the_first_existing_directory_wins() {
        let root = temp_root("chain");
        write(
            &root.join(".mix/skills/alpha"),
            "---\nname: alpha\ndescription: from mix\n---\nbody",
        );
        write(
            &root.join(".claude/skills/beta"),
            "---\nname: beta\ndescription: from claude\n---\nbody",
        );
        assert_eq!(skill_root(&root), Some(root.join(".mix/skills")));
        let names: Vec<String> = load_project_skills(&root)
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(names, vec!["alpha".to_string()]);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn the_chain_falls_back_to_agents() {
        let root = temp_root("fallback");
        write(
            &root.join(".agents/skills/gamma"),
            "---\nname: gamma\ndescription: from agents\n---\nbody",
        );
        assert_eq!(skill_root(&root), Some(root.join(".agents/skills")));
        let names: Vec<String> = load_project_skills(&root)
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(names, vec!["gamma".to_string()]);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn no_skill_directory_means_no_skills() {
        let root = temp_root("none");
        assert_eq!(skill_root(&root), None);
        assert!(load_project_skills(&root).is_empty());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_malformed_skill_is_reported_not_dropped() {
        let root = temp_root("malformed");
        write(
            &root.join(".mix/skills/good"),
            "---\nname: good\ndescription: ok\n---\nbody",
        );
        write(&root.join(".mix/skills/bad"), "no frontmatter, no heading");
        let (skills, failures) = load_project_report(&root);
        assert_eq!(
            skills.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            vec!["good"]
        );
        assert_eq!(failures.len(), 1);
        assert!(failures[0].path.ends_with("SKILL.md"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_name_that_does_not_match_the_directory_is_reported() {
        let root = temp_root("mismatch");
        write(
            &root.join(".mix/skills/alpha"),
            "---\nname: beta\ndescription: copied folder\n---\nbody",
        );
        let (skills, failures) = load_project_report(&root);
        assert!(skills.is_empty());
        assert_eq!(failures.len(), 1);
        assert!(failures[0].reason.contains("must match"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn bundled_files_are_listed_without_reading_their_contents() {
        let root = temp_root("resources");
        let skill_dir = root.join(".mix/skills/packaged");
        write(
            &skill_dir,
            "---\nname: packaged\ndescription: has files\n---\nbody",
        );
        std::fs::create_dir_all(skill_dir.join("references")).expect("mkdir");
        std::fs::create_dir_all(skill_dir.join("assets")).expect("mkdir");
        std::fs::write(skill_dir.join("references/GUIDE.md"), "hello").expect("write");
        std::fs::write(skill_dir.join("assets/logo.bin"), [0xff, 0xfe, 0x00]).expect("write");

        let skills = load_project_skills(&root);
        let resources = &skills[0].resources;
        assert_eq!(resources.len(), 2);
        let binary = resources
            .iter()
            .find(|r| r.path == "assets/logo.bin")
            .expect("binary resource");
        assert_eq!(binary.size, 3);
        // Discovery must not have read the file: only `read_skill_resource`
        // materialises content.
        assert!(binary.content.is_empty());

        let text =
            read_skill_resource(&root.join(".mix/skills"), "packaged", "references/GUIDE.md")
                .expect("text resource");
        assert!(!text.binary);
        assert_eq!(text.content, "hello");
        let binary = read_skill_resource(&root.join(".mix/skills"), "packaged", "assets/logo.bin")
            .expect("binary resource");
        assert!(binary.binary);
        assert_eq!(binary.size, 3);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn symlinks_are_not_followed() {
        let root = temp_root("symlink");
        let skill_dir = root.join(".mix/skills/linked");
        write(
            &skill_dir,
            "---\nname: linked\ndescription: linked\n---\nbody",
        );
        let outside = root.join("outside.txt");
        std::fs::write(&outside, "sensitive").expect("write");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, skill_dir.join("escape.txt")).expect("symlink");

        let skills = load_project_skills(&root);
        assert!(skills[0].resources.is_empty());
        assert!(
            read_skill_resource(&root.join(".mix/skills"), "linked", "escape.txt").is_none(),
            "a symlink must not be readable through skill_resource"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_project_skill_wins_over_a_user_skill_with_the_same_name() {
        let root = temp_root("precedence");
        write(
            &root.join(".mix/skills/shared"),
            "---\nname: shared\ndescription: project\n---\nbody",
        );
        let project = load_project_skills(&root);
        let user = vec![Skill {
            name: "shared".into(),
            description: "user".into(),
            body_markdown: "body".into(),
            ..Skill::default()
        }];
        let skills = merge_skills(project, user);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].description, "project");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn the_user_root_chain_is_the_same_shape() {
        let home = temp_root("user-root");
        std::fs::create_dir_all(home.join(".claude/skills")).expect("mkdir");
        assert_eq!(user_skill_root_in(&home), Some(home.join(".claude/skills")));
        std::fs::remove_dir_all(&home).ok();
    }

    #[tokio::test]
    async fn without_a_root_the_project_source_contributes_nothing() {
        let out = ProjectSkillSource.skills(None).await.expect("skills");
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn the_project_source_reads_a_resource() {
        let root = temp_root("source-resource");
        let skill_dir = root.join(".mix/skills/packaged");
        write(
            &skill_dir,
            "---\nname: packaged\ndescription: has files\n---\nbody",
        );
        std::fs::write(skill_dir.join("helper.py"), "print('hi')").expect("write");

        let root_text = root.display().to_string();
        let source = ProjectSkillSource;
        let skills = source.skills(Some(&root_text)).await.expect("skills");
        assert_eq!(skills[0].resources, vec!["helper.py"]);
        let file = source
            .resource(Some(&root_text), "packaged", "helper.py")
            .await
            .expect("resource")
            .expect("found");
        assert_eq!(file.content, "print('hi')");
        assert!(source
            .resource(Some(&root_text), "packaged", "../secret")
            .await
            .expect("resource")
            .is_none());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn the_shared_fixtures_match_the_expected_catalogue() {
        // The same fixtures are read by `tests/unit/test_skill_fixtures.py`.
        // Keeping one expected.json means a Desktop-only parsing change cannot
        // silently disagree with the Server.
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures/skills");
        let (skills, failures) = load_dir(&root);
        let expected: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(root.join("expected.json")).expect("expected.json"),
        )
        .expect("parse expected.json");

        for skill in &skills {
            let want = expected
                .get(&skill.name)
                .expect("fixture listed in expected.json");
            assert_eq!(
                skill.description,
                want["description"].as_str().expect("description")
            );
            assert_eq!(skill.license, want["license"].as_str().expect("license"));
            assert_eq!(
                skill.compatibility,
                want["compatibility"].as_str().expect("compatibility")
            );
            assert_eq!(
                skill.allowed_tools,
                want["allowed_tools"]
                    .as_array()
                    .expect("allowed_tools")
                    .iter()
                    .map(|value| value.as_str().expect("tool").to_string())
                    .collect::<Vec<_>>()
            );
            let want_metadata: BTreeMap<String, String> = want["metadata"]
                .as_object()
                .expect("metadata")
                .iter()
                .map(|(key, value)| {
                    (
                        key.clone(),
                        value.as_str().expect("metadata value").to_string(),
                    )
                })
                .collect();
            assert_eq!(skill.metadata, want_metadata);
            let want_resources: Vec<String> = want["resources"]
                .as_array()
                .expect("resources")
                .iter()
                .map(|value| value.as_str().expect("resource").to_string())
                .collect();
            assert_eq!(
                skill
                    .resources
                    .iter()
                    .map(|resource| resource.path.clone())
                    .collect::<Vec<_>>(),
                want_resources
            );
        }

        // `legacy` is a plain-Markdown document; Desktop can parse it, but it
        // is not a valid Agent Skills directory because the name does not
        // match the parent folder. The fixture pins that the fallback still
        // exists without letting it pollute the catalogue.
        assert!(failures
            .iter()
            .any(|failure| failure.path.ends_with("legacy/SKILL.md")));
        assert!(skills.iter().all(|skill| skill.name != "legacy-skill"));
    }
}
