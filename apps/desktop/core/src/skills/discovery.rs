//! Discovery of skills that live in the project folder rather than the database.
//!
//! A repository keeps its own procedures next to the code they describe, so
//! the agent should find them without an import step. The directory names are
//! the ones other coding agents already use: `.mix/skills` first (ours), then
//! `.claude/skills` and `.agents/skills`.
//!
//! Only the first directory that exists is read. Merging all three would let a
//! repository accidentally contribute a skill it did not author, and makes the
//! effective catalogue depend on which files happen to be present.

use std::path::{Path, PathBuf};

use mixd_tools::skill::{SkillSource, SkillSummary};

use super::{parse_skill, Skill};

/// Candidate skill roots, most preferred first.
pub const SKILL_ROOTS: [&str; 3] = [".mix/skills", ".claude/skills", ".agents/skills"];

/// The one skill directory that applies to this project, if any.
pub fn skill_root(project_root: &Path) -> Option<PathBuf> {
    SKILL_ROOTS
        .iter()
        .map(|rel| project_root.join(rel))
        .find(|dir| dir.is_dir())
}

/// Every skill under `project_root`'s skill directory.
///
/// A `SKILL.md` that does not parse is skipped with a log line rather than
/// failing the run: one malformed file in a repository must not disable the
/// rest, and the user can still import it through the UI to see the error.
pub fn load_project_skills(project_root: &Path) -> Vec<Skill> {
    let Some(dir) = skill_root(project_root) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut skills = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let file = path.join("SKILL.md");
        let raw = match std::fs::read_to_string(&file) {
            Ok(raw) => raw,
            Err(err) => {
                tracing::warn!(path = %file.display(), %err, "skill file could not be read");
                continue;
            }
        };
        match parse_skill(&raw) {
            Ok(skill) => skills.push(skill),
            Err(err) => {
                tracing::warn!(path = %file.display(), %err, "skill file is not a valid SKILL.md")
            }
        }
    }
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
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
        let path = Path::new(root);
        if !path.is_dir() {
            return Ok(Vec::new());
        }
        Ok(load_project_skills(path)
            .into_iter()
            .map(|skill| SkillSummary {
                name: skill.name,
                description: skill.description,
                body_markdown: skill.body_markdown,
                allowed_tools: skill.allowed_tools,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn a_malformed_skill_is_skipped_not_fatal() {
        let root = temp_root("malformed");
        write(&root.join(".mix/skills/bad"), "no frontmatter, no heading");
        write(
            &root.join(".mix/skills/good"),
            "---\nname: good\ndescription: ok\n---\nbody",
        );
        let names: Vec<String> = load_project_skills(&root)
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(names, vec!["good".to_string()]);
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn without_a_root_the_source_contributes_nothing() {
        let out = ProjectSkillSource.skills(None).await.expect("skills");
        assert!(out.is_empty());
    }
}
