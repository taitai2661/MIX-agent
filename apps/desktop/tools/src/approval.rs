//! Approval inbox: serialises a Tool call's pending permission decision so the
//! UI can present an Allow / Deny / Always Allow prompt.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::risk::RiskAssessment;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingApproval {
    pub approval_id: String,
    pub run_id: uuid::Uuid,
    pub tool_call_id: String,
    pub tool_name: String,
    pub arguments: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub remember: bool,
    /// Command-level grading. `None` on approvals created before this field
    /// existed (and for Tools that are not graded).
    #[serde(default)]
    pub risk: Option<RiskAssessment>,
}

/// The user's answer to a pending approval prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalOutcome {
    /// Run the Tool once.
    Approve {
        /// Persist the rule to disk ("Always Allow").
        remember: bool,
        /// Allow the same tool/target until the app exits. Neither a one-off
        /// approval nor a durable rule: useful for a run that needs the same
        /// command several times without granting it forever.
        remember_session: bool,
    },
    Deny,
}

/// One selectable answer on a structured confirmation card.
///
/// Serialised with snake_case keys (`label`, `description`); the TypeScript
/// reducer maps them onto the camelCase `QuestionOption` it renders.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionOption {
    pub label: String,
    /// One-line elaboration shown under the label. May be empty.
    #[serde(default)]
    pub description: String,
}

/// A structured question the agent asks the user mid-Run.
///
/// Unlike an approval (Allow / Deny on a Tool call), this carries an arbitrary
/// question with a fixed set of options. The user's answer is delivered back
/// to the Run as steering text (see [`format_question_answer`]), so answering
/// never bypasses the normal turn boundary — the model sees it exactly where
/// it would see a message typed into the composer mid-Run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionCard {
    pub question: String,
    pub options: Vec<QuestionOption>,
    /// When true the user may pick several options; otherwise exactly one
    /// (plus an optional free-text note).
    #[serde(default)]
    pub multi_select: bool,
    /// Label of the suggested option, if any. Must match one of `options`.
    #[serde(default)]
    pub recommended: Option<String>,
}

impl QuestionCard {
    /// Maximum options a card may carry. Enough for a real choice, few enough
    /// that the card stays scannable and the model cannot dump a menu.
    pub const MAX_OPTIONS: usize = 6;
    /// Maximum labels the answer may carry, mirroring `MAX_OPTIONS`.
    pub const MAX_SELECTED: usize = 6;

    /// Reject a malformed card before it is ever shown or answered.
    pub fn validate(&self) -> Result<(), String> {
        if self.question.trim().is_empty() {
            return Err("question is empty".to_string());
        }
        if self.options.is_empty() {
            return Err("question has no options".to_string());
        }
        if self.options.len() > Self::MAX_OPTIONS {
            return Err(format!(
                "question has too many options ({} > {})",
                self.options.len(),
                Self::MAX_OPTIONS
            ));
        }
        let mut seen = std::collections::HashSet::new();
        for option in &self.options {
            if option.label.trim().is_empty() {
                return Err("question option has an empty label".to_string());
            }
            if !seen.insert(option.label.trim().to_string()) {
                return Err(format!(
                    "question has a duplicate option: {}",
                    option.label.trim()
                ));
            }
        }
        if let Some(recommended) = &self.recommended {
            let wanted = recommended.trim();
            if !self.options.iter().any(|o| o.label.trim() == wanted) {
                return Err(format!(
                    "recommended option is not one of the choices: {recommended}"
                ));
            }
        }
        Ok(())
    }
}

/// Render the user's card answer as steering text for the Run.
///
/// The `selected` labels are echoed verbatim so the model can match them back
/// to the options it offered; `free_text` is appended as a note. Returns
/// `Err` when there is nothing to send (no selection and no note).
pub fn format_question_answer(
    selected: &[String],
    free_text: Option<&str>,
) -> Result<String, String> {
    let picked: Vec<String> = selected
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if picked.len() > QuestionCard::MAX_SELECTED {
        return Err(format!(
            "too many selected options ({} > {})",
            picked.len(),
            QuestionCard::MAX_SELECTED
        ));
    }
    let note = free_text.map(str::trim).unwrap_or_default().to_string();
    if picked.is_empty() && note.is_empty() {
        return Err("answer is empty: pick an option or write a note".to_string());
    }
    let mut text = String::from("[user answer]");
    if !picked.is_empty() {
        text.push_str(" selected: ");
        text.push_str(&picked.join("; "));
    }
    if !note.is_empty() {
        if !picked.is_empty() {
            text.push('\n');
        } else {
            text.push(' ');
        }
        text.push_str("note: ");
        text.push_str(&note);
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card() -> QuestionCard {
        QuestionCard {
            question: "どれを使いますか?".to_string(),
            options: vec![
                QuestionOption {
                    label: "A".to_string(),
                    description: "first".to_string(),
                },
                QuestionOption {
                    label: "B".to_string(),
                    description: String::new(),
                },
            ],
            multi_select: false,
            recommended: Some("A".to_string()),
        }
    }

    #[test]
    fn valid_card_passes() {
        assert!(card().validate().is_ok());
    }

    #[test]
    fn validation_rejects_bad_cards() {
        let mut bad = card();
        bad.question = "  ".to_string();
        assert!(bad.validate().is_err());
        let mut bad = card();
        bad.options.clear();
        assert!(bad.validate().is_err());
        let mut bad = card();
        bad.options[1].label = "A".to_string();
        assert!(bad.validate().is_err());
        let mut bad = card();
        bad.recommended = Some("Z".to_string());
        assert!(bad.validate().is_err());
    }

    #[test]
    fn answer_formatting_covers_selection_and_note() {
        let text =
            format_question_answer(&["A".to_string(), "B".to_string()], Some("おまかせ")).unwrap();
        assert!(text.contains("selected: A; B"));
        assert!(text.contains("note: おまかせ"));
        let text = format_question_answer(&[], Some("自由回答")).unwrap();
        assert!(text.contains("自由回答"));
        assert!(format_question_answer(&[], None).is_err());
        assert!(format_question_answer(&[], Some("  ")).is_err());
    }
}
