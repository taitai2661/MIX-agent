use crate::AppState;
use mixd_tools::approval::ApprovalOutcome;
use serde::Deserialize;
use tauri::State;

#[derive(Deserialize)]
pub struct ApprovalDecision {
    /// The approval id shown in the Approvals pane.
    pub id: String,
    pub grant: bool,
    /// "Always Allow": persist a permission rule so later calls skip the prompt.
    pub remember: bool,
}

#[tauri::command]
pub async fn decide_approval(
    state: State<'_, AppState>,
    input: ApprovalDecision,
) -> Result<(), String> {
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
        }
    } else {
        ApprovalOutcome::Deny
    };
    state
        .engine
        .executor
        .resolve_approval(&input.id, &tool_name, outcome)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}
