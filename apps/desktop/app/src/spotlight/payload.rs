//! Pure payload builders for the macOS Spotlight index.
//!
//! Everything in this module is platform-neutral: it reads SQLite rows, turns
//! them into [`IndexItem`] values, and never touches Core Spotlight. That keeps
//! the interesting logic testable on Linux/Windows CI while [`super::index`]
//! owns the platform-specific write path.

use std::fmt::Write as _;

use chrono::{DateTime, Utc};
use mixd_storage::records::{KnowledgeData, ProjectData};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Pool, Sqlite};

pub const DOMAIN_CONVERSATIONS: &str = "conversations";
pub const DOMAIN_PROJECTS: &str = "projects";
pub const DOMAIN_KNOWLEDGE: &str = "knowledge";

/// How much body text is sent to the OS index. These caps keep a very long
/// thread or knowledge document from turning one update into a multi-megabyte
/// write; Core Spotlight ranks on the field, not on completeness.
pub const CONVERSATION_BODY_CHARS: usize = 8_000;
pub const KNOWLEDGE_BODY_CHARS: usize = 64_000;
pub const PROJECT_BODY_CHARS: usize = 20_000;
pub const SUMMARY_CHARS: usize = 200;
pub const TITLE_FALLBACK_CHARS: usize = 60;

/// `(id, title, mode, project_id, created_at, updated_at, latest preview)`.
type ConversationIndexRow = (
    String,
    String,
    String,
    Option<String>,
    String,
    String,
    String,
);

/// The three things MIX-agent publishes to Spotlight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpotlightKind {
    Conversation,
    Project,
    Knowledge,
}

impl SpotlightKind {
    pub fn domain(self) -> &'static str {
        match self {
            Self::Conversation => DOMAIN_CONVERSATIONS,
            Self::Project => DOMAIN_PROJECTS,
            Self::Knowledge => DOMAIN_KNOWLEDGE,
        }
    }

    pub fn prefix(self) -> &'static str {
        match self {
            Self::Conversation => "conversation",
            Self::Project => "project",
            Self::Knowledge => "knowledge",
        }
    }

    pub fn item_id(self, source_id: &str) -> String {
        format!("{}:{source_id}", self.prefix())
    }

    /// Inverse of [`Self::item_id`]. Used by the NSUserActivity callback; an
    /// unknown prefix is ignored rather than treated as a conversation.
    pub fn parse(item_id: &str) -> Option<(Self, String)> {
        let (prefix, id) = item_id.split_once(':')?;
        if id.is_empty() {
            return None;
        }
        let kind = match prefix {
            "conversation" => Self::Conversation,
            "project" => Self::Project,
            "knowledge" => Self::Knowledge,
            _ => return None,
        };
        Some((kind, id.to_string()))
    }
}

/// One row the indexer wants Core Spotlight to hold.
#[derive(Debug, Clone, PartialEq)]
pub struct IndexItem {
    pub item_id: String,
    pub domain: &'static str,
    pub source_id: String,
    pub title: String,
    pub summary: String,
    pub keywords: Vec<String>,
    /// Present only when the user opted into body indexing.
    pub body: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

impl IndexItem {
    /// Stable fingerprint used to skip unchanged items during a reconcile.
    /// Includes the body, so toggling body indexing invalidates every row.
    pub fn content_hash(&self) -> String {
        let mut hasher = Sha256::new();
        for field in [
            self.item_id.as_str(),
            self.domain,
            self.source_id.as_str(),
            self.title.as_str(),
            self.summary.as_str(),
        ] {
            hasher.update(field.as_bytes());
            hasher.update([0u8]);
        }
        for keyword in &self.keywords {
            hasher.update(keyword.as_bytes());
            hasher.update([0u8]);
        }
        if let Some(body) = &self.body {
            hasher.update(body.as_bytes());
        }
        hasher.update([0u8]);
        for date in [self.created_at, self.updated_at].into_iter().flatten() {
            hasher.update(date.to_rfc3339().as_bytes());
            hasher.update([0u8]);
        }
        let digest = hasher.finalize();
        let mut out = String::with_capacity(digest.len() * 2);
        for byte in digest {
            let _ = write!(out, "{byte:02x}");
        }
        out
    }
}

/// Collapse whitespace and cap the result. Used for titles and previews, where
/// Spotlight expects one readable line.
pub fn truncate_flat(text: &str, max_chars: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_raw(&flat, max_chars)
}

/// Cap a multi-line string without collapsing its line breaks (body text).
pub fn truncate_raw(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Both UI languages are searchable: the app ships a Japanese and an English
/// interface, and the index has no per-item locale.
fn mode_keywords(mode: &str) -> &'static [&'static str] {
    match mode {
        "agent" => &["エージェント", "agent"],
        "thinking" => &["思考", "thinking"],
        _ => &["チャット", "chat"],
    }
}

fn parse_time(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

fn last_path_component(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| path.to_string())
}

/// Build the conversation item. `body_enabled` is the user's opt-in; without
/// it only metadata and the latest assistant preview leave the database.
pub async fn build_conversation(
    pool: &Pool<Sqlite>,
    id: &str,
    body_enabled: bool,
) -> anyhow::Result<Option<IndexItem>> {
    let row: Option<ConversationIndexRow> = sqlx::query_as(
        "SELECT c.id, \
                    COALESCE(json_extract(c.data, '$.title'), ''), \
                    COALESCE(json_extract(c.data, '$.mode'), 'chat'), \
                    json_extract(c.data, '$.projectId'), \
                    c.created_at, \
                    COALESCE(json_extract(c.data, '$.updatedAt'), c.created_at), \
                    COALESCE((SELECT json_extract(m.data, '$.content') \
                              FROM messages m \
                              WHERE m.conversation_id = c.id \
                                AND json_extract(m.data, '$.role') = 'assistant' \
                              ORDER BY m.created_at DESC, m.rowid DESC LIMIT 1), '') \
             FROM conversations c WHERE c.id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    let Some((id, title, mode, project_id, created_at, updated_at, preview)) = row else {
        return Ok(None);
    };

    // The default title is assigned before the first message exists. Once the
    // user has sent something, the first user turn is a much better search
    // key than "新しい会話".
    let first_user: Option<(String,)> = sqlx::query_as(
        "SELECT COALESCE(json_extract(data, '$.content'), '') FROM messages \
         WHERE conversation_id = ? AND json_extract(data, '$.role') = 'user' \
         ORDER BY created_at, rowid LIMIT 1",
    )
    .bind(&id)
    .fetch_optional(pool)
    .await?;
    let first_user = first_user.map(|(text,)| truncate_flat(&text, TITLE_FALLBACK_CHARS));
    let title = if title.trim().is_empty() || title == "新しい会話" {
        first_user
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| "新しい会話".to_string())
    } else {
        title
    };

    let project_name: Option<String> = match project_id.as_deref().filter(|p| !p.is_empty()) {
        Some(project_id) => sqlx::query_scalar(
            "SELECT COALESCE(json_extract(data, '$.name'), '') \
                 FROM projects WHERE id = ? LIMIT 1",
        )
        .bind(project_id)
        .fetch_optional(pool)
        .await?
        .filter(|name: &String| !name.is_empty()),
        None => None,
    };

    let body = if body_enabled {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT COALESCE(json_extract(data, '$.role'), ''), \
                    COALESCE(json_extract(data, '$.content'), '') \
             FROM messages \
             WHERE conversation_id = ? \
               AND json_extract(data, '$.role') IN ('user', 'assistant') \
             ORDER BY created_at, rowid",
        )
        .bind(&id)
        .fetch_all(pool)
        .await?;
        let mut joined = String::new();
        for (role, content) in rows {
            if content.trim().is_empty() {
                continue;
            }
            if !joined.is_empty() {
                joined.push('\n');
            }
            joined.push_str(role.as_str());
            joined.push_str(": ");
            joined.push_str(&content);
            // Stop reading once the cap is reached; the remainder cannot be
            // indexed anyway.
            if joined.chars().count() >= CONVERSATION_BODY_CHARS {
                break;
            }
        }
        Some(truncate_raw(
            &mixd_core::memory::redact_sensitive(&joined),
            CONVERSATION_BODY_CHARS,
        ))
    } else {
        None
    };

    let summary = {
        let preview = truncate_flat(&preview, SUMMARY_CHARS);
        if preview.is_empty() {
            title.clone()
        } else {
            preview
        }
    };
    let mut keywords = vec!["MIX-agent".to_string()];
    keywords.extend(mode_keywords(&mode).iter().map(|value| value.to_string()));
    if let Some(name) = &project_name {
        keywords.push(name.clone());
    }

    Ok(Some(IndexItem {
        item_id: SpotlightKind::Conversation.item_id(&id),
        domain: DOMAIN_CONVERSATIONS,
        source_id: id,
        title: truncate_flat(&title, 200),
        summary,
        keywords,
        body,
        created_at: parse_time(&created_at),
        updated_at: parse_time(&updated_at),
    }))
}

/// Build the project item. Body indexing adds the user's instructions and
/// source texts; without it only the name and folder name are visible.
pub async fn build_project(
    pool: &Pool<Sqlite>,
    id: &str,
    body_enabled: bool,
) -> anyhow::Result<Option<IndexItem>> {
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT data, created_at FROM projects WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?;
    let Some((data, _created_at)) = row else {
        return Ok(None);
    };
    let project: ProjectData = match serde_json::from_str(&data) {
        Ok(project) => project,
        Err(err) => {
            tracing::warn!(project = %id, %err, "project row is not valid ProjectData");
            return Ok(None);
        }
    };
    let folder = last_path_component(&project.root_path);
    let body = if body_enabled {
        let mut body = String::new();
        body.push_str(&project.root_path);
        if !project.instructions.trim().is_empty() {
            body.push_str("\n\n");
            body.push_str(&project.instructions);
        }
        for source in &project.sources {
            body.push('\n');
            body.push_str(source);
        }
        Some(truncate_raw(
            &mixd_core::memory::redact_sensitive(&body),
            PROJECT_BODY_CHARS,
        ))
    } else {
        None
    };

    Ok(Some(IndexItem {
        item_id: SpotlightKind::Project.item_id(id),
        domain: DOMAIN_PROJECTS,
        source_id: id.to_string(),
        title: truncate_flat(&project.name, 200),
        summary: truncate_flat(&format!("{folder} · プロジェクト"), SUMMARY_CHARS),
        keywords: vec!["MIX-agent".to_string(), "プロジェクト".to_string(), folder],
        body,
        created_at: Some(project.created_at),
        updated_at: Some(project.updated_at),
    }))
}

/// Build one knowledge document. Rows are chunks, so they are grouped by
/// `docId` before the item is produced.
pub async fn build_knowledge(
    pool: &Pool<Sqlite>,
    doc_id: &str,
    body_enabled: bool,
) -> anyhow::Result<Option<IndexItem>> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT data, created_at FROM knowledge \
         WHERE json_extract(data, '$.docId') = ? \
         ORDER BY created_at, rowid",
    )
    .bind(doc_id)
    .fetch_all(pool)
    .await?;
    if rows.is_empty() {
        return Ok(None);
    }

    let mut filename = String::new();
    let mut first_text = String::new();
    let mut joined = String::new();
    let mut created_at = None;
    for (data, created) in rows {
        let chunk: KnowledgeData = match serde_json::from_str(&data) {
            Ok(chunk) => chunk,
            Err(err) => {
                tracing::warn!(doc = %doc_id, %err, "knowledge row is not valid KnowledgeData");
                continue;
            }
        };
        if filename.is_empty() {
            filename = chunk.filename.clone();
        }
        if first_text.is_empty() {
            first_text = chunk.text.clone();
        }
        if created_at.is_none() {
            created_at = parse_time(&created);
        }
        if body_enabled && joined.chars().count() < KNOWLEDGE_BODY_CHARS {
            if !joined.is_empty() {
                joined.push('\n');
            }
            joined.push_str(&chunk.text);
        }
    }
    if filename.is_empty() {
        return Ok(None);
    }

    let summary = if first_text.trim().is_empty() {
        filename.clone()
    } else {
        truncate_flat(&first_text, SUMMARY_CHARS)
    };
    let body = if body_enabled {
        Some(truncate_raw(
            &mixd_core::memory::redact_sensitive(&joined),
            KNOWLEDGE_BODY_CHARS,
        ))
    } else {
        None
    };

    Ok(Some(IndexItem {
        item_id: SpotlightKind::Knowledge.item_id(doc_id),
        domain: DOMAIN_KNOWLEDGE,
        source_id: doc_id.to_string(),
        title: truncate_flat(&filename, 200),
        summary,
        keywords: vec![
            "MIX-agent".to_string(),
            "ナレッジ".to_string(),
            "knowledge".to_string(),
        ],
        body,
        created_at,
        updated_at: created_at,
    }))
}

pub async fn build_item(
    pool: &Pool<Sqlite>,
    kind: SpotlightKind,
    source_id: &str,
    body_enabled: bool,
) -> anyhow::Result<Option<IndexItem>> {
    match kind {
        SpotlightKind::Conversation => build_conversation(pool, source_id, body_enabled).await,
        SpotlightKind::Project => build_project(pool, source_id, body_enabled).await,
        SpotlightKind::Knowledge => build_knowledge(pool, source_id, body_enabled).await,
    }
}

/// Every source id that currently exists for one domain. Used by reconcile to
/// find rows in `spotlight_items` whose source has disappeared.
pub async fn source_ids(pool: &Pool<Sqlite>, kind: SpotlightKind) -> anyhow::Result<Vec<String>> {
    let rows: Vec<(String,)> = match kind {
        SpotlightKind::Conversation => {
            sqlx::query_as("SELECT id FROM conversations")
                .fetch_all(pool)
                .await?
        }
        SpotlightKind::Project => {
            sqlx::query_as("SELECT id FROM projects")
                .fetch_all(pool)
                .await?
        }
        SpotlightKind::Knowledge => {
            sqlx::query_as(
                "SELECT DISTINCT json_extract(data, '$.docId') FROM knowledge \
             WHERE json_extract(data, '$.docId') IS NOT NULL",
            )
            .fetch_all(pool)
            .await?
        }
    };
    Ok(rows
        .into_iter()
        .map(|(id,)| id)
        .filter(|id| !id.is_empty())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifier_round_trips() {
        for kind in [
            SpotlightKind::Conversation,
            SpotlightKind::Project,
            SpotlightKind::Knowledge,
        ] {
            let item = kind.item_id("abc-123");
            assert_eq!(
                SpotlightKind::parse(&item),
                Some((kind, "abc-123".to_string()))
            );
        }
    }

    #[test]
    fn unknown_identifiers_are_ignored() {
        assert_eq!(SpotlightKind::parse("conversation:"), None);
        assert_eq!(SpotlightKind::parse("other:abc"), None);
        assert_eq!(SpotlightKind::parse("nonsense"), None);
    }

    #[test]
    fn flat_truncation_collapses_whitespace_and_never_splits_chars() {
        assert_eq!(truncate_flat("a\n\n b\tc", 100), "a b c");
        let out = truncate_flat(&"日本語".repeat(20), 5);
        assert_eq!(out.chars().count(), 5);
        assert!(out.ends_with('…'));
        assert!(!out.contains('\u{FFFD}'));
    }

    #[test]
    fn raw_truncation_keeps_line_breaks() {
        let out = truncate_raw("one\ntwo\nthree", 100);
        assert_eq!(out, "one\ntwo\nthree");
    }

    #[test]
    fn content_hash_is_stable_and_includes_body() {
        let item = IndexItem {
            item_id: "conversation:a".into(),
            domain: DOMAIN_CONVERSATIONS,
            source_id: "a".into(),
            title: "Title".into(),
            summary: "Summary".into(),
            keywords: vec!["MIX-agent".into()],
            body: None,
            created_at: None,
            updated_at: None,
        };
        assert_eq!(item.content_hash(), item.clone().content_hash());
        let mut with_body = item.clone();
        with_body.body = Some("secret".into());
        assert_ne!(item.content_hash(), with_body.content_hash());
    }

    #[test]
    fn path_component_handles_trailing_slashes() {
        assert_eq!(last_path_component("/Users/me/project/"), "project");
        assert_eq!(last_path_component("/"), "/");
    }
}
