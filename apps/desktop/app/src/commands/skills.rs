use crate::AppState;
use mixd_core::skills::{Skill, SkillResource, SkillService};
use serde::{Deserialize, Serialize};
use tauri::State;

#[derive(Serialize)]
pub struct SkillView {
    pub name: String,
    pub description: String,
    pub body_markdown: String,
    pub allowed_tools: Vec<String>,
    pub resource_paths: Vec<String>,
    pub updated_at: String,
}

impl From<Skill> for SkillView {
    fn from(s: Skill) -> Self {
        Self {
            resource_paths: s.resources.iter().map(|r| r.path.clone()).collect(),
            name: s.name,
            description: s.description,
            body_markdown: s.body_markdown,
            allowed_tools: s.allowed_tools,
            updated_at: s.updated_at.to_rfc3339(),
        }
    }
}

#[tauri::command]
pub async fn list_skills(state: State<'_, AppState>) -> Result<Vec<SkillView>, String> {
    let skills = SkillService::list(state.storage.as_ref())
        .await
        .map_err(crate::error::cmd_err)?;
    Ok(skills.into_iter().map(SkillView::from).collect())
}

#[derive(Deserialize)]
pub struct SkillImportInput {
    /// `"paste"` | `"zip"` | `"directory"`
    pub source: String,
    pub payload: Option<String>,
    pub path: Option<String>,
    pub name: Option<String>,
}

#[tauri::command]
pub async fn import_skill(
    state: State<'_, AppState>,
    input: SkillImportInput,
) -> Result<SkillView, String> {
    let mut resources: Vec<SkillResource> = Vec::new();
    let raw = match input.source.as_str() {
        "paste" => input
            .payload
            .ok_or("payload is required for paste import")?,
        "zip" => {
            let bytes = read_input(&input).await?;
            // Zip inflation and the directory walk are CPU / I/O heavy and
            // fully synchronous, so they run on the blocking pool instead of
            // stalling the Tauri async runtime.
            let (skill_md, extra) =
                tokio::task::spawn_blocking(move || crate::commands::zip_util::unzip_skill(&bytes))
                    .await
                    .map_err(crate::error::cmd_err)?
                    .map_err(crate::error::cmd_err)?;
            resources = extra;
            skill_md
        }
        "directory" => {
            let dir = input
                .path
                .clone()
                .ok_or("path is required for directory import")?;
            let (skill_md, extra) = tokio::task::spawn_blocking(move || {
                crate::commands::zip_util::read_skill_dir(&dir)
            })
            .await
            .map_err(crate::error::cmd_err)?
            .map_err(crate::error::cmd_err)?;
            resources = extra;
            skill_md
        }
        other => return Err(format!("unknown source: {}", other)),
    };
    let skill = SkillService::import_markdown(state.storage.as_ref(), &raw, resources)
        .await
        .map_err(crate::error::cmd_err)?;
    state.engine.invalidate_catalogues().await;
    Ok(SkillView::from(skill))
}

async fn read_input(input: &SkillImportInput) -> Result<Vec<u8>, String> {
    use base64::Engine;
    match (&input.payload, &input.path) {
        (Some(p), _) => base64::engine::general_purpose::STANDARD
            .decode(p.as_bytes())
            .map_err(crate::error::cmd_err),
        (None, Some(p)) => tokio::fs::read(p).await.map_err(crate::error::cmd_err),
        _ => Err("either payload (base64) or path is required".into()),
    }
}

#[tauri::command]
pub async fn export_skill(
    state: State<'_, AppState>,
    name: String,
    as_zip: bool,
) -> Result<String, String> {
    if !as_zip {
        return SkillService::export(state.storage.as_ref(), &name)
            .await
            .map_err(crate::error::cmd_err);
    }
    let skill = SkillService::get(state.storage.as_ref(), &name)
        .await
        .map_err(crate::error::cmd_err)?
        .ok_or_else(|| format!("skill not found: {}", name))?;
    let bytes = tokio::task::spawn_blocking(move || crate::commands::zip_util::zip_skill(&skill))
        .await
        .map_err(crate::error::cmd_err)?
        .map_err(crate::error::cmd_err)?;
    use base64::Engine;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

#[tauri::command]
pub async fn delete_skill(state: State<'_, AppState>, name: String) -> Result<(), String> {
    SkillService::delete(state.storage.as_ref(), &name)
        .await
        .map_err(crate::error::cmd_err)?;
    // The catalogue is cached in-process; without this a deleted skill would
    // keep being advertised to the model until the app restarted.
    state.engine.invalidate_catalogues().await;
    Ok(())
}

#[derive(Serialize)]
pub struct SkillResourceView {
    pub path: String,
    pub content: String,
}

/// Every resource bundled with a skill, so the UI can list them without
/// guessing the paths.
#[tauri::command]
pub async fn list_skill_resources(
    state: State<'_, AppState>,
    name: String,
) -> Result<Vec<SkillResourceView>, String> {
    let skill = SkillService::get(state.storage.as_ref(), &name)
        .await
        .map_err(crate::error::cmd_err)?
        .ok_or_else(|| format!("skill not found: {}", name))?;
    Ok(skill
        .resources
        .into_iter()
        .map(|r| SkillResourceView {
            path: r.path,
            content: r.content,
        })
        .collect())
}

#[tauri::command]
pub async fn read_skill_resource(
    state: State<'_, AppState>,
    name: String,
    path: String,
) -> Result<SkillResourceView, String> {
    let skill = SkillService::get(state.storage.as_ref(), &name)
        .await
        .map_err(crate::error::cmd_err)?
        .ok_or_else(|| format!("skill not found: {}", name))?;
    skill
        .resources
        .into_iter()
        .find(|r| r.path == path)
        .map(|r| SkillResourceView {
            path: r.path,
            content: r.content,
        })
        .ok_or_else(|| format!("resource not found: {}", path))
}
