//! Skills: `SKILL.md` catalogue with paste / zip / directory import and export.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A `SKILL.md` document. `name` and `description` are required by the
/// frontmatter; `body_markdown` is everything after the frontmatter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub body_markdown: String,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub resources: Vec<SkillResource>,
    pub updated_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillResource {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillParseError {
    pub message: String,
}

impl std::fmt::Display for SkillParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for SkillParseError {}

/// Parse a `SKILL.md` document.
///
/// Accepts YAML frontmatter delimited by `---`, or a plain Markdown body where
/// the first `# Heading` is the name and the first paragraph is the
/// description.
pub fn parse_skill(raw: &str) -> Result<Skill, SkillParseError> {
    let trimmed = raw.trim_start();
    if let Some(rest) = trimmed.strip_prefix("---") {
        let after = rest.trim_start_matches('\n');
        if let Some(end) = after.find("\n---") {
            let front = &after[..end];
            let body = after[end + 4..]
                .trim_start_matches(['\n', '-'])
                .trim_start();
            let (name, description, allowed) = parse_frontmatter(front);
            let name = name.ok_or(SkillParseError {
                message: "SKILL.md frontmatter is missing `name`".into(),
            })?;
            return Ok(Skill {
                name,
                description: description.unwrap_or_default(),
                body_markdown: body.to_string(),
                allowed_tools: allowed,
                resources: Vec::new(),
                updated_at: Utc::now(),
            });
        }
    }
    // Plain Markdown fallback.
    let mut lines = trimmed.lines();
    let heading = lines
        .find(|l| l.trim_start().starts_with('#'))
        .map(|l| l.trim_start().trim_start_matches('#').trim().to_string())
        .ok_or(SkillParseError {
            message: "SKILL.md must start with a `# name` heading".into(),
        })?;
    let body = trimmed.to_string();
    let description = body
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .map(|l| l.trim())
        .find(|l| !l.is_empty())
        .unwrap_or_default()
        .to_string();
    Ok(Skill {
        name: heading,
        description,
        body_markdown: body,
        allowed_tools: Vec::new(),
        resources: Vec::new(),
        updated_at: Utc::now(),
    })
}

/// Minimal YAML frontmatter reader: `key: value` plus `key: [a, b]` lists.
fn parse_frontmatter(text: &str) -> (Option<String>, Option<String>, Vec<String>) {
    let mut name = None;
    let mut description = None;
    let mut allowed = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        match key {
            "name" => name = Some(unquote(value)),
            "description" => description = Some(unquote(value)),
            "allowed-tools" | "allowed_tools" => {
                allowed = value
                    .trim_start_matches('[')
                    .trim_end_matches(']')
                    .split(',')
                    .map(|s| unquote(s.trim()))
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            _ => {}
        }
    }
    (name, description, allowed)
}

fn unquote(value: &str) -> String {
    value
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_string()
}

/// Render a `Skill` back into `SKILL.md` form.
pub fn render_skill(skill: &Skill) -> String {
    let mut out = String::from("---\n");
    out.push_str(&format!("name: {}\n", skill.name));
    out.push_str(&format!("description: {}\n", skill.description));
    if !skill.allowed_tools.is_empty() {
        out.push_str(&format!(
            "allowed-tools: [{}]\n",
            skill.allowed_tools.join(", ")
        ));
    }
    out.push_str("---\n\n");
    out.push_str(skill.body_markdown.trim());
    out.push('\n');
    out
}

pub struct SkillService;

impl SkillService {
    /// Skills whose name or description mentions `query`, best match first.
    ///
    /// Token overlap rather than embeddings: the catalogue is small and user
    /// authored, so an explainable ranking beats an opaque one.
    pub async fn search(
        storage: &mixd_storage::Storage,
        query: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<Skill>> {
        let all = Self::list(storage).await?;
        let terms: Vec<String> = query
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| t.len() > 2)
            .map(|t| t.to_lowercase())
            .collect();
        if terms.is_empty() {
            return Ok(all.into_iter().take(limit).collect());
        }
        let mut scored: Vec<(usize, Skill)> = all
            .into_iter()
            .filter_map(|skill| {
                let haystack = format!("{} {}", skill.name, skill.description).to_lowercase();
                let hits = terms.iter().filter(|t| haystack.contains(*t)).count();
                (hits > 0).then_some((hits, skill))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.name.cmp(&b.1.name)));
        Ok(scored.into_iter().take(limit).map(|(_, s)| s).collect())
    }

    /// Save (or replace) a skill by name.
    pub async fn save(storage: &mixd_storage::Storage, skill: Skill) -> anyhow::Result<Skill> {
        let now = Utc::now();
        let data = serde_json::json!({
            "name": skill.name,
            "description": skill.description,
            "bodyMarkdown": skill.body_markdown,
            "allowedTools": skill.allowed_tools,
            "resources": skill.resources,
            "source": "user",
            "updatedAt": now.to_rfc3339(),
        });
        let existing: Option<String> =
            sqlx::query_scalar("SELECT id FROM skills WHERE json_extract(data, '$.name') = ?")
                .bind(&skill.name)
                .fetch_optional(storage.pool())
                .await?;
        let id = existing
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        if existing.is_some() {
            sqlx::query("UPDATE skills SET data = ? WHERE id = ?")
                .bind(data.to_string())
                .bind(&id)
                .execute(storage.pool())
                .await?;
        } else {
            sqlx::query("INSERT INTO skills (id, owner_id, data) VALUES (?, ?, ?)")
                .bind(&id)
                .bind(mixd_storage::records::SINGLE_USER_ID)
                .bind(data.to_string())
                .execute(storage.pool())
                .await?;
        }
        let mut out = skill;
        out.updated_at = now;
        Ok(out)
    }

    pub async fn import_markdown(
        storage: &mixd_storage::Storage,
        raw: &str,
        resources: Vec<SkillResource>,
    ) -> anyhow::Result<Skill> {
        let mut skill = parse_skill(raw)?;
        skill.resources = resources;
        Self::save(storage, skill).await
    }

    /// Every stored skill.
    ///
    /// This had no non-CRUD caller: the skills table was written and displayed
    /// but never consulted at run time, so a `SKILL.md` an author spent time on
    /// had no effect on what the model could do. [`catalogue_prompt`] is what
    /// turns this into a capability.
    pub async fn list(storage: &mixd_storage::Storage) -> anyhow::Result<Vec<Skill>> {
        let rows = sqlx::query("SELECT data FROM skills ORDER BY json_extract(data, '$.name')")
            .fetch_all(storage.pool())
            .await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| Self::from_data(&row))
            .collect())
    }

    pub async fn get(storage: &mixd_storage::Storage, name: &str) -> anyhow::Result<Option<Skill>> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT data FROM skills WHERE json_extract(data, '$.name') = ?")
                .bind(name)
                .fetch_optional(storage.pool())
                .await?;
        let row = match row {
            Some(r) => r,
            None => return Ok(None),
        };
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap_or_default();
        Ok(Self::from_value(&parsed))
    }

    pub async fn delete(storage: &mixd_storage::Storage, name: &str) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM skills WHERE json_extract(data, '$.name') = ?")
            .bind(name)
            .execute(storage.pool())
            .await?;
        Ok(())
    }

    pub async fn export(storage: &mixd_storage::Storage, name: &str) -> anyhow::Result<String> {
        let skill = Self::get(storage, name)
            .await?
            .ok_or_else(|| anyhow::anyhow!("skill not found: {}", name))?;
        Ok(render_skill(&skill))
    }

    fn from_data(row: &sqlx::sqlite::SqliteRow) -> Option<Skill> {
        use sqlx::Row;
        let raw: String = row.try_get("data").ok()?;
        let parsed: serde_json::Value = serde_json::from_str(&raw).ok()?;
        Self::from_value(&parsed)
    }

    fn from_value(v: &serde_json::Value) -> Option<Skill> {
        Some(Skill {
            name: v.get("name")?.as_str()?.to_string(),
            description: v
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or_default()
                .to_string(),
            body_markdown: v
                .get("bodyMarkdown")
                .and_then(|d| d.as_str())
                .unwrap_or_default()
                .to_string(),
            allowed_tools: v
                .get("allowedTools")
                .and_then(|d| d.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default(),
            resources: v
                .get("resources")
                .and_then(|d| d.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|item| {
                            Some(SkillResource {
                                path: item.get("path")?.as_str()?.to_string(),
                                content: item
                                    .get("content")
                                    .and_then(|c| c.as_str())
                                    .unwrap_or_default()
                                    .to_string(),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default(),
            updated_at: v
                .get("updatedAt")
                .and_then(|d| d.as_str())
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|t| t.with_timezone(&Utc))
                .unwrap_or_else(Utc::now),
        })
    }
}

/// The skill catalogue as a system-prompt block.
///
/// Name, description and permitted tools only: the procedure itself is fetched
/// on demand through [`skill_detail_prompt`], so a library of twenty skills
/// costs a few hundred tokens rather than twenty documents.
pub fn catalogue_prompt(skills: &[Skill]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "<skills>\nThe following skills are available. Each has a procedure you have not \
         been shown yet. To use one, call the `skill` tool with its name to read the \
         full instructions before acting.\n",
    );
    for skill in skills {
        out.push_str(&format!("- {}: {}", skill.name, skill.description));
        if !skill.allowed_tools.is_empty() {
            out.push_str(&format!(
                " [allowed-tools: {}]",
                skill.allowed_tools.join(", ")
            ));
        }
        out.push('\n');
    }
    out.push_str("</skills>");
    out
}

/// The full body of one skill, for the `skill` tool to return.
pub fn skill_detail_prompt(skills: &[Skill], name: &str) -> String {
    match skills
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case(name.trim()))
    {
        Some(skill) => {
            let mut out = format!("<skill name=\"{}\">\n", skill.name);
            if !skill.allowed_tools.is_empty() {
                out.push_str(&format!(
                    "allowed-tools: {}\n",
                    skill.allowed_tools.join(", ")
                ));
            }
            out.push_str(skill.body_markdown.trim());
            out.push_str("\n</skill>");
            out
        }
        None => format!("no skill named {name:?}; call the `skill` tool with no name to list them"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frontmatter() {
        let raw = "---\nname: expense-report\ndescription: Build expense reports\nallowed-tools: [files, terminal]\n---\n\n# Expense report\n\nSteps…";
        let skill = parse_skill(raw).expect("parse");
        assert_eq!(skill.name, "expense-report");
        assert_eq!(skill.description, "Build expense reports");
        assert_eq!(skill.allowed_tools, vec!["files", "terminal"]);
        assert!(skill.body_markdown.contains("Steps"));
    }

    #[test]
    fn parses_plain_markdown() {
        let skill = parse_skill("# My Skill\n\nDoes a thing.\n\nMore.").expect("parse");
        assert_eq!(skill.name, "My Skill");
        assert_eq!(skill.description, "Does a thing.");
    }

    #[test]
    fn missing_name_is_an_error() {
        let raw = "---\ndescription: no name here\n---\nbody";
        assert!(parse_skill(raw).is_err());
    }

    #[test]
    fn round_trips() {
        let raw = "---\nname: a\ndescription: b\n---\n\nbody text";
        let skill = parse_skill(raw).expect("parse");
        let rendered = render_skill(&skill);
        let again = parse_skill(&rendered).expect("reparse");
        assert_eq!(again.name, "a");
        assert_eq!(again.description, "b");
    }

    #[test]
    fn the_catalogue_lists_name_and_description_only() {
        // Progressive disclosure: the body is only worth tokens once the model
        // has decided it wants the skill.
        let skills = vec![Skill {
            name: "expense-report".into(),
            description: "Build expense reports from receipts".into(),
            body_markdown: "A very long procedure that should not be inlined.".into(),
            allowed_tools: vec!["files".into()],
            resources: Vec::new(),
            updated_at: Utc::now(),
        }];
        let prompt = catalogue_prompt(&skills);
        assert!(prompt.contains("expense-report"));
        assert!(prompt.contains("Build expense reports"));
        assert!(
            !prompt.contains("very long procedure"),
            "the body must not be inlined: {prompt}"
        );
        assert!(prompt.contains("files"), "allowed-tools must be shown");
    }

    #[test]
    fn an_empty_catalogue_renders_nothing() {
        assert_eq!(catalogue_prompt(&[]), "");
    }

    #[test]
    fn a_catalogue_without_allowed_tools_omits_the_clause() {
        let skills = vec![Skill {
            name: "a".into(),
            description: "b".into(),
            body_markdown: String::new(),
            allowed_tools: Vec::new(),
            resources: Vec::new(),
            updated_at: Utc::now(),
        }];
        assert!(!catalogue_prompt(&skills).contains("allowed-tools"));
    }

    #[test]
    fn a_skill_body_is_rendered_on_request() {
        let skills = vec![Skill {
            name: "expense-report".into(),
            description: "Build expense reports".into(),
            body_markdown: "1. Collect receipts".into(),
            allowed_tools: Vec::new(),
            resources: Vec::new(),
            updated_at: Utc::now(),
        }];
        let detail = skill_detail_prompt(&skills, "expense-report");
        assert!(detail.contains("Collect receipts"));
    }

    #[test]
    fn requesting_an_unknown_skill_says_so() {
        let skills = vec![Skill {
            name: "a".into(),
            description: "b".into(),
            body_markdown: "body".into(),
            allowed_tools: Vec::new(),
            resources: Vec::new(),
            updated_at: Utc::now(),
        }];
        let detail = skill_detail_prompt(&skills, "nope");
        assert!(detail.contains("nope"));
        assert!(!detail.contains("body"), "must not leak an unrelated skill");
    }
}
