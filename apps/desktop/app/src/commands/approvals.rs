use crate::AppState;
use mixd_tools::approval::ApprovalOutcome;
use serde::{Deserialize, Serialize};
use tauri::State;

#[derive(Deserialize)]
pub struct ApprovalDecision {
    /// The approval id shown in the Approvals pane.
    pub id: String,
    pub grant: bool,
    /// "Always Allow": persist a permission rule so later calls skip the prompt.
    #[serde(default)]
    pub remember: bool,
    /// "Allow for this session": an in-memory rule that dies with the app.
    #[serde(default)]
    pub remember_session: bool,
    /// Required for a `dangerous` command. The UI asks the user to tick an
    /// explicit "I understand" box; a client that skips it is rejected here
    /// rather than trusted to have rendered the warning.
    #[serde(default)]
    pub acknowledge_risk: bool,
}

#[tauri::command]
pub async fn decide_approval(
    state: State<'_, AppState>,
    input: ApprovalDecision,
) -> Result<(), String> {
    // Ownership check: the approval must still be pending. Without this a
    // compromised renderer could resolve an arbitrary id (another Run's
    // prompt, an already-expired row) and launder a tool call through it.
    let pending = state.engine.executor.list_pending().await;
    let Some(prompt) = pending.iter().find(|p| p.approval_id == input.id) else {
        return Err(format!("unknown or expired approval {}", input.id));
    };
    let dangerous = prompt
        .risk
        .as_ref()
        .is_some_and(mixd_tools::risk::RiskAssessment::is_dangerous);
    if dangerous {
        // "Always Allow" is exactly how a dangerous command gets laundered on
        // the next call; refuse it instead of silently downgrading, so the UI
        // can explain why the button is gone.
        if input.grant && (input.remember || input.remember_session) {
            return Err(
                "dangerous commands cannot be remembered; approve this one call explicitly"
                    .to_string(),
            );
        }
        if input.grant && !input.acknowledge_risk {
            return Err("dangerous command requires explicit acknowledgement".to_string());
        }
    }
    // Durable `remember` persists an allow rule; it must never be set on a
    // deny, which would otherwise silently lock the tool down (or, if the
    // outcome mapping ever flips, silently open it).
    if input.remember && !input.grant {
        return Err("remember requires an approval grant".to_string());
    }
    // The rule must be keyed on the Tool, not on this one-off approval id.
    let tool_name = state
        .engine
        .executor
        .tool_name_for_approval(&input.id)
        .await
        .ok_or_else(|| format!("unknown approval {}", input.id))?;

    let outcome = if input.grant {
        ApprovalOutcome::Approve {
            remember: input.remember,
            remember_session: input.remember_session,
        }
    } else {
        ApprovalOutcome::Deny
    };
    state
        .engine
        .executor
        .resolve_approval_checked(&input.id, &tool_name, outcome, input.acknowledge_risk)
        .await
        .map_err(crate::error::cmd_err)?;
    Ok(())
}

#[derive(Deserialize)]
pub struct ApprovalExtensionInput {
    /// The approval id shown on the card / in the inbox.
    pub id: String,
    /// Seconds to add. Bounded by the executor (60 s .. 24 h).
    pub seconds: i64,
}

#[derive(Serialize)]
pub struct ApprovalExtension {
    /// The prompt's new `expires_at`, so the card can restart its countdown
    /// without waiting for the next event batch.
    pub expires_at: String,
}

/// Push a pending prompt's deadline out.
///
/// Mirrors `decide_approval`'s ownership rule: only an id that is still pending
/// can move. The `tool_approval_extended` event updates every other surface
/// (approval inbox, another window) showing the same prompt.
#[tauri::command]
pub async fn extend_approval(
    state: State<'_, AppState>,
    input: ApprovalExtensionInput,
) -> Result<ApprovalExtension, String> {
    let (run_id, _tool_call_id, expires_at) = state
        .engine
        .executor
        .extend_approval(&input.id, input.seconds)
        .await
        .map_err(crate::error::cmd_err)?;
    let expires_at = expires_at.to_rfc3339();
    state
        .engine
        .emit(
            run_id,
            "tool_approval_extended",
            serde_json::json!({
                "approval_id": input.id,
                "expires_at": expires_at,
            }),
        )
        .await;
    Ok(ApprovalExtension { expires_at })
}

/// The user's answer to a structured [`mixd_tools::approval::QuestionCard`].
///
/// The answer is delivered through the same steering queue as a message typed
/// into the composer mid-Run, so the model sees it as an ordinary user turn at
/// the next turn boundary — answering never injects privileged content.
/// Returns `false` (like `steer_run`) when the Run already finished, so the UI
/// can resend the text as a normal turn instead of dropping it.
#[derive(Deserialize)]
pub struct AnswerQuestionInput {
    pub run_id: String,
    /// Correlates the answer with the card the user saw. Echoed in the
    /// `question_resolved` event; not validated against engine state yet —
    /// the producer that emits `question_requested` is still to come, so
    /// there is no pending-question store to check against.
    #[serde(default)]
    pub question_id: Option<String>,
    /// Labels of the chosen options. May be empty when `free_text` answers.
    #[serde(default)]
    pub selected: Vec<String>,
    /// Optional free-text note sent alongside (or instead of) a selection.
    #[serde(default)]
    pub free_text: Option<String>,
}

#[tauri::command]
pub async fn answer_question(
    state: State<'_, AppState>,
    input: AnswerQuestionInput,
) -> Result<bool, String> {
    let run_id = uuid::Uuid::parse_str(&input.run_id).map_err(crate::error::cmd_err)?;
    let text =
        mixd_tools::approval::format_question_answer(&input.selected, input.free_text.as_deref())?;
    let accepted = state.engine.steer(run_id, text.clone()).await;
    // Retire the card even when the Run already ended: the reducer only clears
    // `timeline.question` on this event, and a finished run would otherwise
    // leave an answerable card on screen.
    state
        .engine
        .emit(
            run_id,
            "question_resolved",
            serde_json::json!({
                "question_id": input.question_id,
                "accepted": accepted,
            }),
        )
        .await;
    Ok(accepted)
}
