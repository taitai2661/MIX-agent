//! Run event envelope shared with the Tauri command surface. Field shapes
//! match the Docker edition's `GET /runs/{key}/events` contract.

use serde::{Deserialize, Serialize};

use crate::runs::state::RunId;

/// Upper bound on rows one replay will read.
///
/// Stream deltas are coalesced at write time, so a real run is far below this;
/// the bound exists so a legacy or pathological run cannot make opening a
/// transcript allocate an unbounded vector. A truncated replay keeps both the
/// head and the tail: the head preserves the tool timeline, and the tail keeps
/// the latest answer and the terminal status. A `replay_truncated` marker
/// records how many rows were omitted.
pub const MAX_REPLAY_EVENTS: i64 = 50_000;

/// One persisted event row in the shape shared by both replay paths.
pub type ReplayRow = (String, i64, String, String, String);

/// Rows returned by [`load_replay_window`] plus the untruncated row count.
#[derive(Debug, Default)]
pub struct ReplayWindow {
    pub rows: Vec<ReplayRow>,
    pub total: i64,
}

impl ReplayWindow {
    /// Number of events the replay window omitted.
    pub fn omitted(&self, limit: i64) -> i64 {
        (self.total - limit).max(0)
    }
}

/// Load at most `limit` events, keeping the oldest and newest halves.
///
/// `COUNT(*)` uses the `(run_id, sequence)` index, so a short run costs one
/// count plus one ordered read. A long run reads `limit / 2` rows from each
/// end instead of loading the whole (potentially huge) log and discarding the
/// part the user most wants to see.
pub async fn load_replay_window(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    run_id: &str,
    limit: i64,
) -> Result<ReplayWindow, sqlx::Error> {
    if limit <= 0 {
        return Ok(ReplayWindow::default());
    }
    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM run_events WHERE run_id = ?")
        .bind(run_id)
        .fetch_one(pool)
        .await?;
    if total <= limit {
        let rows: Vec<ReplayRow> = sqlx::query_as(
            "SELECT run_id, sequence, kind, data, created_at FROM run_events \
             WHERE run_id = ? ORDER BY sequence ASC LIMIT ?",
        )
        .bind(run_id)
        .bind(limit)
        .fetch_all(pool)
        .await?;
        return Ok(ReplayWindow { rows, total });
    }

    // Give the tail at least one slot even for a tiny limit, because the
    // newest status/answer is the part a truncated replay must not lose.
    let tail_limit = (limit / 2).max(1);
    let head_limit = limit - tail_limit;
    let mut rows: Vec<ReplayRow> = if head_limit > 0 {
        sqlx::query_as(
            "SELECT run_id, sequence, kind, data, created_at FROM run_events \
             WHERE run_id = ? ORDER BY sequence ASC LIMIT ?",
        )
        .bind(run_id)
        .bind(head_limit)
        .fetch_all(pool)
        .await?
    } else {
        Vec::new()
    };
    let mut tail: Vec<ReplayRow> = sqlx::query_as(
        "SELECT run_id, sequence, kind, data, created_at FROM run_events \
         WHERE run_id = ? ORDER BY sequence DESC LIMIT ?",
    )
    .bind(run_id)
    .bind(tail_limit)
    .fetch_all(pool)
    .await?;
    tail.reverse();
    rows.extend(tail);
    Ok(ReplayWindow { rows, total })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunEvent {
    pub run_id: RunId,
    pub sequence: i64,
    pub kind: String,
    pub data: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// One row of the Trajectory timeline: a timestamped, human-readable view of
/// a single Run event.
///
/// The UI's `reduceRunEvents` folds events into live state (tool cards, text
/// buffers); the trajectory view instead needs the raw chronology — every
/// event in order with a one-line label — so a long run can be scanned,
/// filtered by kind and searched. This is the shared backend shape for that
/// view; the frontend mirrors it in `ui/src/lib/runEvents.ts`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectoryPoint {
    pub sequence: i64,
    pub kind: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub label: String,
    pub detail: String,
}

fn str_field(data: &serde_json::Value, key: &str) -> Option<String> {
    data.get(key)
        .and_then(|value| value.as_str())
        .map(str::to_string)
}

/// Short `(label, detail)` pair summarising one event for the timeline.
///
/// Unknown kinds fall back to the kind itself with an empty detail rather
/// than being dropped: the trajectory must show every event, including ones
/// written by a newer backend.
pub fn summarize_event(kind: &str, data: &serde_json::Value) -> (String, String) {
    const DETAIL_CHARS: usize = 160;
    let truncate = |text: &str| {
        let clipped: String = text.chars().take(DETAIL_CHARS).collect();
        if text.chars().count() > DETAIL_CHARS {
            format!("{clipped}…")
        } else {
            clipped
        }
    };
    match kind {
        "text" | "reasoning" => (
            kind.to_string(),
            str_field(data, "delta")
                .map(|d| truncate(&d))
                .unwrap_or_default(),
        ),
        "status" => (
            "status".to_string(),
            str_field(data, "status").unwrap_or_default(),
        ),
        "usage" => ("usage".to_string(), "token usage".to_string()),
        "error" => (
            "error".to_string(),
            str_field(data, "error")
                .map(|d| truncate(&d))
                .unwrap_or_default(),
        ),
        "tool_started" => {
            let name = str_field(data, "name").unwrap_or_else(|| "tool".to_string());
            let args = data
                .get("arguments")
                .map(|args| truncate(&args.to_string()))
                .unwrap_or_default();
            (
                format!("tool_started:{name}"),
                str_field(data, "tool_call_id").unwrap_or(args),
            )
        }
        "tool_result" => {
            let name = str_field(data, "name").unwrap_or_else(|| "tool".to_string());
            let outcome = str_field(data, "error")
                .map(|err| format!("error: {}", truncate(&err)))
                .unwrap_or_else(|| "ok".to_string());
            (format!("tool_result:{name}"), outcome)
        }
        "tool_denied" => (
            "tool_denied".to_string(),
            str_field(data, "reason").unwrap_or_default(),
        ),
        "tool_approval_requested" => (
            "tool_approval_requested".to_string(),
            str_field(data, "name").unwrap_or_default(),
        ),
        "tool_approval_resolved" => ("tool_approval_resolved".to_string(), String::new()),
        "budget_extension_requested" => (
            "budget_extension_requested".to_string(),
            str_field(data, "reason")
                .map(|d| truncate(&d))
                .unwrap_or_default(),
        ),
        "budget_extension_resolved" => ("budget_extension_resolved".to_string(), String::new()),
        "checkpoint_saved" => (
            "checkpoint_saved".to_string(),
            str_field(data, "checkpoint_id").unwrap_or_default(),
        ),
        "checkpoint_failed" => (
            "checkpoint_failed".to_string(),
            format!(
                "step {}: {}",
                data.get("step")
                    .and_then(|value| value.as_u64())
                    .map(|step| step.to_string())
                    .unwrap_or_else(|| "?".to_string()),
                str_field(data, "error").unwrap_or_default()
            ),
        ),
        "step_routing" => (
            format!(
                "step_routing:{}",
                str_field(data, "tier").unwrap_or_default()
            ),
            str_field(data, "reason").unwrap_or_default(),
        ),
        "model_switched" => (
            "model_switched".to_string(),
            format!(
                "{} → {}",
                str_field(data, "fromModelId").unwrap_or_default(),
                str_field(data, "toModelId").unwrap_or_default()
            ),
        ),
        "question_requested" => (
            "question_requested".to_string(),
            str_field(data, "question").unwrap_or_default(),
        ),
        "question_resolved" => (
            "question_resolved".to_string(),
            format!(
                "{}: {}",
                str_field(data, "question_id").unwrap_or_default(),
                if data.get("accepted").and_then(|value| value.as_bool()) == Some(true) {
                    "accepted"
                } else {
                    "declined"
                }
            ),
        ),
        "memory_recall_degraded" => (
            "memory_recall_degraded".to_string(),
            str_field(data, "reason").unwrap_or_default(),
        ),
        "stagnation_detected" => (
            "stagnation_detected".to_string(),
            str_field(data, "hint")
                .map(|d| truncate(&d))
                .unwrap_or_default(),
        ),
        "verification_required" => (
            "verification_required".to_string(),
            str_field(data, "reason")
                .map(|d| truncate(&d))
                .unwrap_or_default(),
        ),
        "verification_failed" => (
            "verification_failed".to_string(),
            str_field(data, "reason")
                .map(|d| truncate(&d))
                .unwrap_or_default(),
        ),
        "destructive_tool_called" => (
            "destructive_tool_called".to_string(),
            str_field(data, "tool").unwrap_or_default(),
        ),
        "todo_list" => ("todo_list".to_string(), String::new()),
        "context_summary" => (
            "context_summary".to_string(),
            data.get("messages")
                .and_then(|value| value.as_u64())
                .map(|count| format!("{count} messages"))
                .unwrap_or_default(),
        ),
        "replay_truncated" => (
            "replay_truncated".to_string(),
            data.get("omitted")
                .and_then(|value| value.as_u64())
                .map(|omitted| format!("{omitted} events omitted"))
                .unwrap_or_default(),
        ),
        other => (other.to_string(), String::new()),
    }
}

/// Fold persisted events into trajectory order (by sequence).
pub fn build_trajectory(events: &[RunEvent]) -> Vec<TrajectoryPoint> {
    let mut points: Vec<TrajectoryPoint> = events
        .iter()
        .map(|event| {
            let (label, detail) = summarize_event(&event.kind, &event.data);
            TrajectoryPoint {
                sequence: event.sequence,
                kind: event.kind.clone(),
                created_at: event.created_at,
                label,
                detail,
            }
        })
        .collect();
    points.sort_by_key(|point| point.sequence);
    points
}

/// Convenience trait so the storage layer can also surface Run events
/// without depending on the broadcast channel directly.
#[async_trait::async_trait]
pub trait RunEventSink: Send + Sync {
    async fn emit(&self, event: RunEvent);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn summarize_covers_the_common_kinds() {
        let (label, detail) = summarize_event("text", &json!({"delta": "hello"}));
        assert_eq!(label, "text");
        assert_eq!(detail, "hello");

        let (label, detail) = summarize_event(
            "tool_started",
            &json!({"name": "terminal", "tool_call_id": "t1"}),
        );
        assert_eq!(label, "tool_started:terminal");
        assert_eq!(detail, "t1");

        let (label, detail) = summarize_event("status", &json!({"status": "completed"}));
        assert_eq!((label.as_str(), detail.as_str()), ("status", "completed"));

        // New engine events carry a readable detail instead of becoming an
        // empty timeline row — notably the checkpoint failure's error text.
        let (label, detail) = summarize_event(
            "checkpoint_failed",
            &json!({"step": 4, "error": "no parent run"}),
        );
        assert_eq!(label, "checkpoint_failed");
        assert!(detail.contains("no parent run"), "{detail}");
        let (label, detail) = summarize_event(
            "step_routing",
            &json!({"tier": "cheap", "reason": "予算圧力 70% のため strong → cheap にダウングレード"}),
        );
        assert_eq!(label, "step_routing:cheap");
        assert!(detail.contains("ダウングレード"));
        let (label, detail) =
            summarize_event("question_requested", &json!({"question": "Pick one"}));
        assert_eq!(
            (label.as_str(), detail.as_str()),
            ("question_requested", "Pick one")
        );
        let (label, detail) = summarize_event(
            "question_resolved",
            &json!({"question_id": "q1", "accepted": true}),
        );
        assert_eq!(
            (label.as_str(), detail.as_str()),
            ("question_resolved", "q1: accepted")
        );

        // Unknown kinds are kept visible, never dropped.
        let (label, detail) = summarize_event("future_kind", &json!({}));
        assert_eq!((label.as_str(), detail.as_str()), ("future_kind", ""));
    }

    #[test]
    fn summarize_truncates_long_deltas() {
        let long = "あ".repeat(200);
        let (_, detail) = summarize_event("text", &json!({"delta": long}));
        assert!(detail.ends_with('…'));
        assert_eq!(detail.chars().count(), 161);
    }

    #[test]
    fn build_trajectory_sorts_by_sequence() {
        let run_id = uuid::Uuid::new_v4();
        let now = chrono::Utc::now();
        let event = |sequence: i64, kind: &str| RunEvent {
            run_id,
            sequence,
            kind: kind.to_string(),
            data: json!({}),
            created_at: now,
        };
        let points = build_trajectory(&[
            event(3, "status"),
            event(1, "text"),
            event(2, "tool_started"),
        ]);
        let sequences: Vec<i64> = points.iter().map(|point| point.sequence).collect();
        assert_eq!(sequences, vec![1, 2, 3]);
    }
}
