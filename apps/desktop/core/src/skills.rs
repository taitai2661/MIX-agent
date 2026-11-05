//! Skills: the Agent Skills (`SKILL.md`) catalogue with paste / zip /
//! directory import and export.
//!
//! A skill is a directory containing `SKILL.md` (YAML frontmatter plus a
//! Markdown body) and, optionally, bundled files under `scripts/`,
//! `references/` or `assets/`. Parsing is deliberately complete enough for
//! real-world skills: block scalars, quoted values containing `:`, and nested
//! `metadata` maps all appear in published skills and a line parser silently
//! corrupts them.
//!
//! Storage keeps the whole package (metadata plus bundled files) so an
//! imported skill can be exported back without loss. Bundled scripts are never
//! executed by this module; they are data the agent may read or the user may
//! run through the normal terminal approval path.

pub mod discovery;

use std::collections::BTreeMap;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_yaml_ng::{Mapping, Value};
use uuid::Uuid;

/// A `SKILL.md` document. `name` and `description` are required by the
/// frontmatter; `body_markdown` is everything after the frontmatter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub body_markdown: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub compatibility: String,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub resources: Vec<SkillResource>,
    pub updated_at: chrono::DateTime<Utc>,
}

impl Default for Skill {
    fn default() -> Self {
        Self {
            name: String::new(),
            description: String::new(),
            body_markdown: String::new(),
            license: String::new(),
            compatibility: String::new(),
            metadata: BTreeMap::new(),
            allowed_tools: Vec::new(),
            resources: Vec::new(),
            updated_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillResource {
    pub path: String,
    /// Text content, or base64 when `binary` is true.
    pub content: String,
    /// Whether `content` is base64-encoded bytes rather than UTF-8 text.
    #[serde(default)]
    pub binary: bool,
    /// Original size in bytes, used by the UI and by the resource tool.
    #[serde(default)]
    pub size: usize,
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
/// Accepts the Agent Skills YAML frontmatter format, or a plain Markdown body
/// where the first `# Heading` is the name and the first paragraph is the
/// description. The plain-Markdown fallback exists only for skills saved by
/// older builds; new imports go through the frontmatter path.
///
/// Parsing stays permissive and validation happens at [`SkillService::save`]:
/// stored skills predate the naming rule, and a read path that rejected them
/// would make an existing catalogue disappear.
pub fn parse_skill(raw: &str) -> Result<Skill, SkillParseError> {
    let trimmed = raw.trim_start_matches('\u{feff}').trim_start();
    if trimmed.starts_with("---") {
        let (front, body) = split_frontmatter(trimmed).ok_or(SkillParseError {
            message: "SKILL.md frontmatter is not closed with a `---` line".into(),
        })?;
        let mut skill = parse_frontmatter(front)?;
        let body = body.trim_start().to_string();
        if body.chars().count() > CONTENT_MAX {
            return Err(SkillParseError {
                message: format!("SKILL.md body must be at most {CONTENT_MAX} characters"),
            });
        }
        skill.body_markdown = body;
        return Ok(skill);
    }

    // Plain Markdown fallback (legacy rows and hand-pasted notes).
    let mut lines = trimmed.lines();
    let heading = lines
        .find(|l| l.trim_start().starts_with('#'))
        .map(|l| l.trim_start().trim_start_matches('#').trim().to_string())
        .ok_or(SkillParseError {
            message: "SKILL.md must start with YAML frontmatter delimited by `---`".into(),
        })?;
    // A heading is prose ("My Skill"), never a valid name on its own, so slug
    // it instead of rejecting the document the user pasted.
    let name = slug_skill_name(&heading);
    if name.is_empty() {
        return Err(SkillParseError {
            message: format!(
                "`# {heading}` does not contain any letter or digit, so it cannot be a skill name"
            ),
        });
    }
    let body = trimmed.to_string();
    let description = body
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .map(|l| l.trim())
        .find(|l| !l.is_empty())
        .unwrap_or_default()
        .to_string();
    Ok(Skill {
        name,
        description,
        body_markdown: body,
        ..Skill::default()
    })
}

/// Split frontmatter from body. The opening `---` and the closing `---` must
/// each occupy a line; blank lines and trailing spaces are tolerated.
fn split_frontmatter(text: &str) -> Option<(&str, &str)> {
    let rest = text.strip_prefix("---").and_then(|rest| {
        rest.strip_prefix("\r\n")
            .or_else(|| rest.strip_prefix('\n'))
    })?;
    let mut offset = 0usize;
    for line in rest.split_inclusive('\n') {
        let content = line.trim_end_matches(['\r', '\n']);
        if content.trim_end() == "---" {
            return Some((&rest[..offset], &rest[offset + line.len()..]));
        }
        offset += line.len();
    }
    None
}

fn parse_frontmatter(text: &str) -> Result<Skill, SkillParseError> {
    let value: Value = serde_yaml_ng::from_str(text).map_err(|err| SkillParseError {
        message: format!("invalid YAML frontmatter: {err}"),
    })?;
    let map = value.as_mapping().ok_or(SkillParseError {
        message: "frontmatter must be a YAML mapping".into(),
    })?;

    let name = required_scalar(map, "name")?;
    let description = required_scalar(map, "description")?;
    let license = optional_scalar(map, "license")?.unwrap_or_default();
    let compatibility = optional_scalar(map, "compatibility")?.unwrap_or_default();
    let metadata = parse_metadata(map)?;
    let allowed_tools = parse_allowed_tools(map)?;

    let mut errors = validate_meta(&name, &description, &compatibility);
    if !metadata.is_empty() {
        errors.extend(validate_metadata(&metadata));
    }
    if !errors.is_empty() {
        return Err(SkillParseError {
            message: errors.join("; "),
        });
    }

    Ok(Skill {
        name,
        description,
        license,
        compatibility,
        metadata,
        allowed_tools,
        ..Skill::default()
    })
}

fn field<'a>(map: &'a Mapping, key: &str) -> Option<&'a Value> {
    map.iter().find_map(|(k, v)| match k {
        Value::String(s) if s == key => Some(v),
        _ => None,
    })
}

fn scalar_string(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn required_scalar(map: &Mapping, key: &str) -> Result<String, SkillParseError> {
    match field(map, key) {
        None | Some(Value::Null) => Err(SkillParseError {
            message: format!("{key} is required"),
        }),
        Some(value) => scalar_string(value).ok_or_else(|| SkillParseError {
            message: format!("{key} must be a string"),
        }),
    }
}

fn optional_scalar(map: &Mapping, key: &str) -> Result<Option<String>, SkillParseError> {
    match field(map, key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => scalar_string(value)
            .map(Some)
            .ok_or_else(|| SkillParseError {
                message: format!("{key} must be a string"),
            }),
    }
}

fn parse_metadata(map: &Mapping) -> Result<BTreeMap<String, String>, SkillParseError> {
    let Some(value) = field(map, "metadata") else {
        return Ok(BTreeMap::new());
    };
    if value.is_null() {
        return Ok(BTreeMap::new());
    }
    let Some(entries) = value.as_mapping() else {
        return Err(SkillParseError {
            message: "metadata must be a mapping".into(),
        });
    };
    let mut out = BTreeMap::new();
    for (key, value) in entries {
        let Some(key) = scalar_string(key) else {
            return Err(SkillParseError {
                message: "metadata keys must be strings".into(),
            });
        };
        let Some(value) = scalar_string(value) else {
            return Err(SkillParseError {
                message: format!("metadata value for {key:?} must be a string"),
            });
        };
        out.insert(key, value);
    }
    Ok(out)
}

/// `allowed-tools` is a space-separated string in the specification. Older
/// MIX builds wrote a YAML list or a comma-separated string, so both are
/// accepted on read; [`render_skill`] always writes the spec form.
fn parse_allowed_tools(map: &Mapping) -> Result<Vec<String>, SkillParseError> {
    let Some(value) = field(map, "allowed-tools").or_else(|| field(map, "allowed_tools")) else {
        return Ok(Vec::new());
    };
    match value {
        Value::Null => Ok(Vec::new()),
        Value::String(text) => Ok(text
            .split(|c: char| c.is_whitespace() || c == ',')
            .map(str::trim)
            .filter(|tool| !tool.is_empty())
            .map(str::to_string)
            .collect()),
        Value::Sequence(items) => {
            let mut out = Vec::new();
            for item in items {
                let tool = scalar_string(item).ok_or_else(|| SkillParseError {
                    message: "allowed-tools entries must be strings".into(),
                })?;
                if !tool.trim().is_empty() {
                    out.push(tool);
                }
            }
            Ok(out)
        }
        _ => Err(SkillParseError {
            message: "allowed-tools must be a space-separated string".into(),
        }),
    }
}

/// Longest name the catalogue accepts, matching the Agent Skills rule.
pub const SKILL_NAME_MAX: usize = 64;
pub const DESCRIPTION_MAX: usize = 1024;
pub const COMPATIBILITY_MAX: usize = 500;
pub const CONTENT_MAX: usize = 50_000;

pub use discovery::{FILES_COUNT_MAX, FILES_TOTAL_MAX, FILE_MAX};

/// Lowercase, map every run of disallowed characters to a single `-`, and trim
/// the ends: `My Skill` → `my-skill`, `a--b` → `a-b`.
pub fn slug_skill_name(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut pending_dash = false;
    for ch in raw.chars() {
        let ok = ch.is_ascii_lowercase() || ch.is_ascii_digit();
        if ok {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(ch);
        } else if ch.is_ascii_uppercase() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(ch.to_ascii_lowercase());
        } else if !out.is_empty() {
            pending_dash = true;
        }
    }
    out
}

/// A name the `skill` tool and the catalogue can rely on:
/// `^[a-z0-9]+(-[a-z0-9]+)*$`, 1–64 characters.
pub fn validate_skill_name(name: &str) -> Result<(), SkillParseError> {
    if name.is_empty() {
        return Err(SkillParseError {
            message: "skill name must not be empty".into(),
        });
    }
    if name.chars().count() > SKILL_NAME_MAX {
        return Err(SkillParseError {
            message: format!("skill name must be at most {SKILL_NAME_MAX} characters"),
        });
    }
    let mut alphanumeric_run = false;
    for ch in name.chars() {
        match ch {
            'a'..='z' | '0'..='9' => alphanumeric_run = true,
            '-' if alphanumeric_run => alphanumeric_run = false,
            _ => {
                return Err(SkillParseError {
                    message: format!(
                        "invalid skill name {name:?}: expected lowercase letters, digits and \
                         single hyphens, e.g. `my-skill`"
                    ),
                })
            }
        }
    }
    if !alphanumeric_run {
        return Err(SkillParseError {
            message: format!("invalid skill name {name:?}: must not end with a hyphen"),
        });
    }
    Ok(())
}

fn validate_meta(name: &str, description: &str, compatibility: &str) -> Vec<String> {
    let mut errors = Vec::new();
    if let Err(err) = validate_skill_name(name.trim()) {
        errors.push(err.message);
    }
    let description = description.trim();
    if description.is_empty() {
        errors.push("description is required".into());
    } else if description.chars().count() > DESCRIPTION_MAX {
        errors.push(format!(
            "description must be at most {DESCRIPTION_MAX} characters"
        ));
    }
    if compatibility.chars().count() > COMPATIBILITY_MAX {
        errors.push(format!(
            "compatibility must be at most {COMPATIBILITY_MAX} characters"
        ));
    }
    errors
}

fn validate_metadata(metadata: &BTreeMap<String, String>) -> Vec<String> {
    let mut errors = Vec::new();
    for (key, value) in metadata {
        if key.trim().is_empty() {
            errors.push("metadata keys must not be empty".into());
        }
        if value.chars().count() > DESCRIPTION_MAX {
            errors.push(format!(
                "metadata value for {key:?} must be at most {DESCRIPTION_MAX} characters"
            ));
        }
    }
    errors
}

/// Everything a stored skill has to satisfy. Called on write only.
pub fn validate_skill(skill: &Skill) -> Result<(), SkillParseError> {
    let mut errors = validate_meta(&skill.name, &skill.description, &skill.compatibility);
    errors.extend(validate_metadata(&skill.metadata));
    if skill.body_markdown.chars().count() > CONTENT_MAX {
        errors.push(format!(
            "SKILL.md body must be at most {CONTENT_MAX} characters"
        ));
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(SkillParseError {
            message: errors.join("; "),
        })
    }
}

/// Render a `Skill` back into `SKILL.md` form.
///
/// Serialised through a real YAML writer so a description containing `:` or
/// `#` cannot produce a document that parses differently on the next read.
pub fn render_skill(skill: &Skill) -> String {
    let mut meta = Mapping::new();
    meta.insert(
        Value::String("name".into()),
        Value::String(skill.name.clone()),
    );
    meta.insert(
        Value::String("description".into()),
        Value::String(skill.description.clone()),
    );
    if !skill.license.trim().is_empty() {
        meta.insert(
            Value::String("license".into()),
            Value::String(skill.license.clone()),
        );
    }
    if !skill.compatibility.trim().is_empty() {
        meta.insert(
            Value::String("compatibility".into()),
            Value::String(skill.compatibility.clone()),
        );
    }
    if !skill.metadata.is_empty() {
        let mut entries = Mapping::new();
        for (key, value) in &skill.metadata {
            entries.insert(Value::String(key.clone()), Value::String(value.clone()));
        }
        meta.insert(Value::String("metadata".into()), Value::Mapping(entries));
    }
    if !skill.allowed_tools.is_empty() {
        meta.insert(
            Value::String("allowed-tools".into()),
            Value::String(skill.allowed_tools.join(" ")),
        );
    }
    let dumped = serde_yaml_ng::to_string(&Value::Mapping(meta)).unwrap_or_default();
    let body = skill.body_markdown.trim();
    format!("---\n{dumped}---\n\n{body}\n")
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
    ///
    /// The one strict gate: `import_markdown` is the only caller, so paste,
    /// zip and directory imports are all validated here while reads stay
    /// permissive about names written before the rule existed.
    pub async fn save(storage: &mixd_storage::Storage, skill: Skill) -> anyhow::Result<Skill> {
        validate_skill(&skill)?;
        let now = Utc::now();
        let data = serde_json::json!({
            "name": skill.name,
            "description": skill.description,
            "bodyMarkdown": skill.body_markdown,
            "license": skill.license,
            "compatibility": skill.compatibility,
            "metadata": skill.metadata,
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
            license: v
                .get("license")
                .and_then(|d| d.as_str())
                .unwrap_or_default()
                .to_string(),
            compatibility: v
                .get("compatibility")
                .and_then(|d| d.as_str())
                .unwrap_or_default()
                .to_string(),
            metadata: v
                .get("metadata")
                .and_then(|d| d.as_object())
                .map(|map| {
                    map.iter()
                        .filter_map(|(key, value)| {
                            value.as_str().map(|value| (key.clone(), value.to_string()))
                        })
                        .collect()
                })
                .unwrap_or_default(),
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
                            let content = item
                                .get("content")
                                .and_then(|c| c.as_str())
                                .unwrap_or_default()
                                .to_string();
                            Some(SkillResource {
                                path: item.get("path")?.as_str()?.to_string(),
                                size: item
                                    .get("size")
                                    .and_then(|s| s.as_u64())
                                    .map(|s| s as usize)
                                    .unwrap_or_else(|| content.len()),
                                binary: item
                                    .get("binary")
                                    .and_then(|b| b.as_bool())
                                    .unwrap_or(false),
                                content,
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
            if !skill.resources.is_empty() {
                out.push_str(
                    "\n\nBundled files (read with the `skill_resource` tool before acting):\n",
                );
                for resource in &skill.resources {
                    out.push_str(&format!("- {}\n", resource.path));
                }
            }
            out.push_str("\n</skill>");
            out
        }
        None => format!("no skill named {name:?}; call the `skill` tool with no name to list them"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"---
name: expense-report
description: File and validate expense reports. Use when asked about reimbursements.
license: Apache-2.0
compatibility: Requires python3
metadata:
  author: finance
  version: "2.1"
allowed-tools: Read Write
---

# Expense report

Follow the company policy.
"#;

    #[test]
    fn parses_the_full_frontmatter() {
        let skill = parse_skill(FULL).expect("parse");
        assert_eq!(skill.name, "expense-report");
        assert_eq!(skill.license, "Apache-2.0");
        assert_eq!(skill.compatibility, "Requires python3");
        assert_eq!(
            skill.metadata.get("author").map(String::as_str),
            Some("finance")
        );
        assert_eq!(
            skill.metadata.get("version").map(String::as_str),
            Some("2.1")
        );
        assert_eq!(skill.allowed_tools, vec!["Read", "Write"]);
        assert!(skill.body_markdown.starts_with("# Expense report"));
    }

    #[test]
    fn parses_block_scalars_and_quoted_colons() {
        let raw = r#"---
name: release-notes
description: >-
  Draft release notes from a git log.
  Use when preparing a release.
allowed-tools: Bash(git:*) Read
---

Body.
"#;
        let skill = parse_skill(raw).expect("parse");
        assert_eq!(
            skill.description,
            "Draft release notes from a git log. Use when preparing a release."
        );
        assert_eq!(skill.allowed_tools, vec!["Bash(git:*)", "Read"]);
    }

    #[test]
    fn parses_yaml_list_allowed_tools_for_old_packages() {
        let raw = "---\nname: a\ndescription: b\nallowed-tools: [files, terminal]\n---\n\nbody";
        let skill = parse_skill(raw).expect("parse");
        assert_eq!(skill.allowed_tools, vec!["files", "terminal"]);
    }

    #[test]
    fn parses_plain_markdown_as_a_legacy_row() {
        let skill = parse_skill("# My Skill\n\nDoes a thing.\n\nMore.").expect("parse");
        assert_eq!(skill.name, "my-skill");
        assert_eq!(skill.description, "Does a thing.");
    }

    #[test]
    fn an_unterminated_frontmatter_is_an_error_not_a_fallback() {
        let raw = "---\nname: broken\ndescription: no closing delimiter";
        assert!(parse_skill(raw).is_err());
    }

    #[test]
    fn missing_name_is_an_error() {
        let raw = "---\ndescription: no name here\n---\nbody";
        assert!(parse_skill(raw).is_err());
    }

    #[test]
    fn a_description_is_required_on_write() {
        let mut skill = skill_named("my-skill", "   ");
        assert!(validate_skill(&skill).is_err());
        skill.description = "does a thing".into();
        assert!(validate_skill(&skill).is_ok());
    }

    #[test]
    fn optional_fields_round_trip() {
        let skill = parse_skill(FULL).expect("parse");
        let rendered = render_skill(&skill);
        let again = parse_skill(&rendered).expect("reparse");
        assert_eq!(again.name, skill.name);
        assert_eq!(again.description, skill.description);
        assert_eq!(again.license, skill.license);
        assert_eq!(again.compatibility, skill.compatibility);
        assert_eq!(again.metadata, skill.metadata);
        assert_eq!(again.allowed_tools, skill.allowed_tools);
        assert_eq!(again.body_markdown, skill.body_markdown);
    }

    #[test]
    fn rendering_uses_the_spec_allowed_tools_form() {
        let mut skill = skill_named("a", "b");
        skill.allowed_tools = vec!["Read".into(), "Bash(git:*)".into()];
        let rendered = render_skill(&skill);
        assert!(
            rendered.contains("allowed-tools: Read Bash(git:*)"),
            "{rendered}"
        );
        assert!(
            !rendered.contains("allowed-tools: ["),
            "the spec form is a string: {rendered}"
        );
    }

    #[test]
    fn a_description_with_yaml_metacharacters_survives_a_round_trip() {
        let mut skill = skill_named("a", "Use when: the user says #1");
        skill.body_markdown = "Body".into();
        let rendered = render_skill(&skill);
        let again = parse_skill(&rendered).expect("reparse");
        assert_eq!(again.description, skill.description);
    }

    #[test]
    fn a_stored_skill_with_an_old_style_name_still_reads() {
        // Read paths never validate: rows written before the rule must not
        // vanish from the catalogue.
        let raw = r#"{"name":"My Skill","description":"legacy","bodyMarkdown":"x","allowedTools":[],"resources":[],"updatedAt":"2026-01-01T00:00:00Z"}"#;
        let value: serde_json::Value = serde_json::from_str(raw).unwrap();
        let skill = SkillService::from_value(&value).expect("legacy row loads");
        assert_eq!(skill.name, "My Skill");
        assert!(skill.metadata.is_empty());
        assert!(skill.license.is_empty());
    }

    #[test]
    fn a_legacy_resource_without_binary_flag_reads_as_text() {
        let raw = r#"{"name":"a","description":"b","bodyMarkdown":"x","allowedTools":[],"resources":[{"path":"references/A.md","content":"hello"}],"updatedAt":"2026-01-01T00:00:00Z"}"#;
        let value: serde_json::Value = serde_json::from_str(raw).unwrap();
        let skill = SkillService::from_value(&value).expect("legacy resource loads");
        assert_eq!(skill.resources.len(), 1);
        assert!(!skill.resources[0].binary);
        assert_eq!(skill.resources[0].size, 5);
    }

    #[test]
    fn names_follow_the_agent_skills_rule() {
        for ok in ["a", "my-skill", "skill2", &"x".repeat(64)] {
            assert!(validate_skill_name(ok).is_ok(), "{ok:?} should be valid");
        }
        for bad in [
            "",
            "My Skill",
            "my--skill",
            "-my-skill",
            "my-skill-",
            "skill_1",
            "スキル",
            &"x".repeat(65),
        ] {
            assert!(
                validate_skill_name(bad).is_err(),
                "{bad:?} should be rejected"
            );
        }
    }

    fn skill_named(name: &str, description: &str) -> Skill {
        Skill {
            name: name.to_string(),
            description: description.to_string(),
            body_markdown: "body".into(),
            ..Skill::default()
        }
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
            ..Skill::default()
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
        let skills = vec![skill_named("a", "b")];
        assert!(!catalogue_prompt(&skills).contains("allowed-tools"));
    }

    #[test]
    fn a_skill_body_is_rendered_on_request() {
        let mut skill = skill_named("expense-report", "Build expense reports");
        skill.body_markdown = "1. Collect receipts".into();
        let detail = skill_detail_prompt(&[skill], "expense-report");
        assert!(detail.contains("Collect receipts"));
    }

    #[test]
    fn a_skill_detail_lists_bundled_files_but_does_not_inline_them() {
        let mut skill = skill_named("pdf", "PDF work");
        skill.resources = vec![SkillResource {
            path: "references/FORMS.md".into(),
            content: "secret reference body".into(),
            binary: false,
            size: 21,
        }];
        let detail = skill_detail_prompt(&[skill], "pdf");
        assert!(detail.contains("references/FORMS.md"));
        assert!(
            !detail.contains("secret reference body"),
            "resources must stay on demand: {detail}"
        );
    }

    #[test]
    fn requesting_an_unknown_skill_says_so() {
        let skills = vec![skill_named("a", "b")];
        let detail = skill_detail_prompt(&skills, "nope");
        assert!(detail.contains("nope"));
        assert!(!detail.contains("body"), "must not leak an unrelated skill");
    }
}
