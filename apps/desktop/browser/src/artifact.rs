//! Screenshot / PDF artifact metadata for the Browser Operator.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserArtifact {
    pub id: String,
    pub run_id: uuid::Uuid,
    pub kind: ArtifactKind,
    pub mime_type: String,
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Screenshot,
    Pdf,
}
