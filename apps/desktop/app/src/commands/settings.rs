use crate::AppState;
use serde::{Deserialize, Serialize};
use tauri::State;

/// Context ceiling used for locally hosted models when the user has not
/// chosen one. Chosen to fit an 8 GB GPU with a 7–8B Q4 model; the model's
/// advertised maximum (often 128k) is a capability, not a sensible default.
pub const DEFAULT_LOCAL_CONTEXT_WINDOW: u32 = 8192;

/// The single settings document.
///
/// `#[serde(default)]` sits on the container rather than on individual fields
/// so one rule covers the whole struct: any key missing from the stored JSON
/// falls back to `Default` instead of failing the parse. Without it a
/// hand-edited or truncated row would deserialize to an error and
/// [`read_settings`] would throw away *every* stored preference, not just the
/// unreadable one. It also means a field added in a future version reads back
/// as its default rather than blanking the document written by an older build.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct SettingsView {
    pub default_model_id: Option<String>,
    pub default_mode: Option<String>,
    pub egress_allowlist: Vec<String>,
    /// Browser-only opt-in for loopback / private-network navigation.
    ///
    /// This is separate from the egress allowlist on purpose: the agent needs
    /// `localhost:5173` to verify a dev server, but the user must say so
    /// explicitly. Link-local/metadata addresses stay blocked either way.
    pub browser_allow_local: bool,
    /// UI language override: `"ja" | "en"`; `None` follows the browser.
    pub language: Option<String>,
    /// UI theme: `"light" | "dark" | "system"`; `None` follows the OS.
    pub theme: Option<String>,
    /// Default parent folder for auto-generated project workspaces
    /// (Projectless chats, legacy backfills). `None` →
    /// `~/Documents/MIX-agent Projects`.
    pub projects_root: Option<String>,
    /// Codex-compatible `project_doc_max_bytes`: combined byte cap for the
    /// `AGENTS.md` chain. `None` → 32 KiB.
    pub project_doc_max_bytes: Option<u64>,
    /// Codex-compatible `project_doc_fallback_filenames`: extra instruction
    /// filenames consulted after `AGENTS.override.md` / `AGENTS.md`.
    pub project_doc_fallback_filenames: Vec<String>,
    /// App-wide default for the project's own check, used by any project that
    /// does not override it. `None` leaves the verification gate off.
    pub verify_command: Option<String>,
    /// App-wide default formatter/lint command reported to the model.
    pub format_command: Option<String>,
    /// Refuse shell commands that write outside the workspace, and any
    /// privilege-escalation wrapper, instead of merely flagging them.
    pub sandbox_terminal: bool,
    /// Also deny network inside the macOS terminal sandbox. Off by default:
    /// package installs and test suites need it. Only takes effect when
    /// `sandbox_terminal` is on and the platform is macOS.
    pub sandbox_terminal_network: bool,
    /// Whether skills under the user's home (`~/.mix/skills`,
    /// `~/.claude/skills`, `~/.agents/skills`) are discovered and offered to
    /// the agent. Project skills are always discovered.
    ///
    /// Defaults to `true`: a personal skill should apply across projects, and
    /// a row written before this key existed has no explicit `false`.
    pub discover_user_skills: bool,
    /// Whether finished turns are mined for durable Memory afterwards
    /// (memory spec §6 自動形成).
    ///
    /// Ships enabled: the spec defines `false` as the opt-out, and a row
    /// written before this key existed must keep behaving like a fresh
    /// install. That is also why `Default` is implemented by hand below — a
    /// derived one would give every `bool` `false` and silently disable the
    /// feature for every existing user.
    pub memory_auto_formation: bool,
    /// §4 source (3): also write embedding-neighbour links after extraction.
    ///
    /// **Defaults `false`**, because §4 says so in as many words (「(3) は既定
    /// オフ」): links made here grow with the square of the store. `false` is
    /// also what `embedding_neighbor_links_if_enabled` answers for a row that
    /// predates this key, so a document written by an older build and a fresh
    /// install behave the same — which is why `Default` below spells it out
    /// rather than inheriting a derived `false` by accident of it being `false`
    /// once. It must be a member of this struct either way: `write_settings`
    /// replaces the stored document wholesale, so a key the struct does not
    /// carry would be dropped from the row the next time Settings was saved.
    pub memory_link_embedding_neighbors: bool,
    /// Opt-in for the unofficial ChatGPT / Codex integration.
    ///
    /// Defaults to `false`: the preset is hidden and the device flow refuses
    /// to start until the operator turns it on in the Providers pane. Existing
    /// rows stay visible so they can be logged out or deleted.
    pub chatgpt_enabled: bool,
    /// Reserved for the future SQLCipher migration; **not available yet**.
    ///
    /// Kept in the document so the key survives a round trip and an older
    /// client cannot erase it, but [`read_settings`] always answers `false`
    /// and [`validate_settings`] rejects `true`. The database is plain SQLite
    /// protected only by owner-only file permissions; OS disk encryption is
    /// the actual at-rest boundary today.
    pub database_encryption: bool,
    /// Whether the first-run wizard has been finished or explicitly deferred.
    ///
    /// `None` is the same as `false` to the UI: a fresh install has never
    /// written this key, and the wizard must appear. It is persisted rather
    /// than kept in `localStorage` so that "あとで" survives a webview data
    /// wipe, travels with a backup/restore, and does not resurrect the wizard
    /// on another machine once the user has already set the app up.
    ///
    /// This is *not* a mirror of "a provider exists": deleting the only
    /// provider would otherwise re-arm the wizard for someone who already
    /// finished it.
    pub onboarding_completed: Option<bool>,
    /// Check GitHub Releases for a newer build once per launch.
    ///
    /// Defaults to `true`: the updater verifies every payload against the
    /// embedded public key, and the check is a single manifest request. The
    /// user can still turn it off (and check manually) in Settings → Updates.
    pub auto_check_updates: bool,
    /// Download a newer build in the background once the launch check finds
    /// one.
    ///
    /// Defaults to `true`, matching `auto_check_updates`. The payload is only
    /// staged: it is installed when the user quits the app normally, so the
    /// next launch runs the new build. This setting never makes the app quit
    /// or restart by itself.
    pub auto_download_updates: bool,
    /// Sweep loopback for local runtimes once after launch so the Providers
    /// pane and auto-start do not begin from an empty table.
    pub local_scan_on_start: bool,
    /// Stop only the processes this app started when it exits. Runtimes the
    /// user launched themselves are never touched.
    pub local_stop_on_exit: bool,
    /// Start an imported local provider automatically when the app launches.
    /// A provider row can override this with `data.local.autoStart`.
    ///
    /// Defaults to `false`: spawning `ollama serve` for someone who manages
    /// Ollama through Homebrew/systemd is a surprise, and the Providers pane
    /// has an explicit Start button for the users who want it.
    pub local_auto_start: bool,
    /// Keep-alive sent to Ollama. Used both for explicit load requests and for
    /// each chat turn of a locally hosted Ollama provider.
    pub local_keep_alive: Option<String>,
    /// Ceiling for `num_ctx` on locally hosted models.
    ///
    /// Discovery records the model's *maximum* context (often 128k+), and
    /// sending that as `num_ctx` makes Ollama reserve a KV cache large enough
    /// to fail or crawl on consumer GPUs. `None` uses
    /// [`DEFAULT_LOCAL_CONTEXT_WINDOW`]; a run never asks a local model for
    /// more than this.
    pub local_context_window: Option<u32>,
    /// Give locally hosted models a modest bonus in Auto routing.
    pub local_prefer: bool,
    /// Publish conversation/project/knowledge metadata to the macOS Spotlight
    /// index. **Defaults off**: thread titles can contain customer or project
    /// names, and a system-wide search surface is a privacy decision the user
    /// should make explicitly.
    pub spotlight_enabled: bool,
    /// Also send conversation and knowledge body text to Spotlight. Separate
    /// from [`Self::spotlight_enabled`] because body text can contain secrets;
    /// credentials are redacted before indexing, but the metadata-only mode is
    /// still the safer default.
    pub spotlight_index_body: bool,
}

impl Default for SettingsView {
    fn default() -> Self {
        Self {
            default_model_id: None,
            default_mode: None,
            egress_allowlist: Vec::new(),
            browser_allow_local: false,
            language: None,
            theme: None,
            projects_root: None,
            project_doc_max_bytes: None,
            project_doc_fallback_filenames: Vec::new(),
            verify_command: None,
            format_command: None,
            sandbox_terminal: false,
            sandbox_terminal_network: false,
            // Personal skills apply across projects; explicit opt-out only.
            discover_user_skills: true,
            // Memory auto formation ships on (memory spec §6): only an
            // explicit `false` in the stored document stops it, and a
            // document written before the key existed has no such `false`.
            memory_auto_formation: true,
            // §4 「(3) は既定オフ」 — off until this row says otherwise.
            memory_link_embedding_neighbors: false,
            // Unofficial integration: explicit opt-in, off by default.
            chatgpt_enabled: false,
            // Codex parity: defaults to false (plain SQLite).
            database_encryption: false,
            onboarding_completed: None,
            auto_check_updates: true,
            auto_download_updates: true,
            local_scan_on_start: true,
            local_stop_on_exit: true,
            local_auto_start: false,
            local_keep_alive: None,
            local_context_window: None,
            local_prefer: false,
            // Opt-in: the OS index is visible outside the app.
            spotlight_enabled: false,
            spotlight_index_body: false,
        }
    }
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<SettingsView, String> {
    read_settings(state.storage.pool()).await
}

#[tauri::command]
pub async fn update_settings(
    state: State<'_, AppState>,
    input: SettingsView,
) -> Result<SettingsView, String> {
    validate_settings(&input)?;
    let view = write_settings(state.storage.pool(), input).await?;
    // Turning Spotlight on/off (or changing body indexing) applies now, not at
    // the next launch. The service reacts to the transition: enable -> rebuild,
    // disable -> delete, body toggle -> rebuild.
    state.spotlight.apply_settings(&view);
    // Apply the browser opt-in immediately; waiting for a relaunch would make
    // the setting look broken.
    state
        .browser
        .set_allow_local(view.browser_allow_local)
        .await;
    // `default_model_id` decides which provider embeds Memory text, so moving
    // the default can change the embedding source; re-derive it on the next
    // recall instead of embedding with the previous provider until relaunch.
    state.embeddings.invalidate();
    // Re-read the allowlist and rebuild the registry. Without this a change to
    // the egress list took effect only after an MCP connect or a browser
    // install, and the manual pane kept the startup copy until a relaunch.
    crate::refresh_registry(&state).await;
    // New shells pick this up; a shell that is already running keeps the
    // profile it was spawned with.
    state
        .engine
        .set_terminal_sandbox(view.sandbox_terminal)
        .await;
    state
        .engine
        .set_terminal_sandbox_network(view.sandbox_terminal_network)
        .await;
    Ok(view)
}

/// What the host can actually enforce, so the settings pane can disable a
/// toggle instead of accepting one that would silently run unsandboxed.
#[derive(Serialize)]
pub struct SandboxStatus {
    pub available: bool,
    pub reason: Option<String>,
    /// `macos` | `linux` | `windows` | `other`
    pub platform: String,
    /// Whether the stricter no-network profile is supported on this platform.
    pub network_supported: bool,
}

#[tauri::command]
pub fn sandbox_status() -> SandboxStatus {
    let platform = if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(windows) {
        "windows"
    } else {
        "other"
    };
    SandboxStatus {
        available: mixd_tools::terminal::os_sandbox_available(),
        reason: mixd_tools::terminal::os_sandbox_unavailable_reason().map(str::to_string),
        platform: platform.to_string(),
        network_supported: cfg!(any(target_os = "macos", target_os = "linux")),
    }
}

/// Reject empty, huge or malformed values before they reach the store.
fn validate_settings(input: &SettingsView) -> Result<(), String> {
    if input.sandbox_terminal && !mixd_tools::terminal::os_sandbox_available() {
        return Err(mixd_tools::terminal::os_sandbox_unavailable_reason()
            .unwrap_or("the terminal sandbox is unavailable on this machine")
            .to_string());
    }
    if input.sandbox_terminal_network && !cfg!(any(target_os = "macos", target_os = "linux")) {
        return Err(
            "the terminal network sandbox is only available on macOS and Linux".to_string(),
        );
    }
    if input.sandbox_terminal_network && !input.sandbox_terminal {
        return Err("enable the terminal sandbox before denying its network".to_string());
    }
    // At-rest encryption is staged but not wired (see `mixd_storage::db`):
    // SQLCipher is not applied to the connection, so accepting `true` here
    // would tell the user their database is encrypted while it stays plaintext.
    // Keep the key in the document for forward compatibility, but only ever
    // accept the honest value until the migration ships.
    if input.database_encryption {
        return Err(
            "database encryption is not available in this build; keep the SQLite file behind \
             disk-level encryption (FileVault / BitLocker / LUKS)"
                .to_string(),
        );
    }
    if let Some(lang) = &input.language {
        match lang.as_str() {
            "ja" | "en" => {}
            _ => return Err("invalid language (expected \"ja\" or \"en\")".to_string()),
        }
    }
    if let Some(theme) = &input.theme {
        match theme.as_str() {
            "light" | "dark" | "system" => {}
            _ => {
                return Err("invalid theme (expected \"light\", \"dark\" or \"system\")".to_string())
            }
        }
    }
    if let Some(model) = &input.default_model_id {
        if model.trim().is_empty() {
            return Err("default model must not be empty".to_string());
        }
        if model.chars().count() > 256 {
            return Err("default model is too long".to_string());
        }
    }
    if let Some(mode) = &input.default_mode {
        if mode.trim().is_empty() {
            return Err("default mode must not be empty".to_string());
        }
        mode.parse::<mixd_core::Mode>()
            .map_err(|_| "invalid mode (expected chat, thinking, plan or agent)".to_string())?;
    }
    if input.egress_allowlist.len() > 100 {
        return Err("egress allowlist has too many entries (max 100)".to_string());
    }
    for entry in &input.egress_allowlist {
        if entry.trim().is_empty() {
            return Err("egress allowlist must not contain empty entries".to_string());
        }
        if entry.chars().count() > 253 {
            return Err(format!("egress allowlist entry too long: {entry}"));
        }
    }
    if let Some(max) = input.project_doc_max_bytes {
        if max == 0 || max > 10 * 1024 * 1024 {
            return Err("project_doc_max_bytes must be between 1 and 10485760".to_string());
        }
    }
    if input.project_doc_fallback_filenames.len() > 20 {
        return Err("too many fallback filenames (max 20)".to_string());
    }
    for name in &input.project_doc_fallback_filenames {
        if name.trim().is_empty() {
            return Err("fallback filenames must not contain empty entries".to_string());
        }
        if name.chars().count() > 256 {
            return Err("fallback filename too long".to_string());
        }
    }
    for cmd in [&input.verify_command, &input.format_command]
        .into_iter()
        .flatten()
    {
        if cmd.chars().count() > 2000 {
            return Err("command too long (max 2000 chars)".to_string());
        }
    }
    if let Some(cap) = input.local_context_window {
        if !(512..=1_048_576).contains(&cap) {
            return Err("local_context_window must be between 512 and 1048576".to_string());
        }
    }
    if let Some(keep_alive) = &input.local_keep_alive {
        if !valid_keep_alive(keep_alive) {
            return Err(
                "local_keep_alive must be -1 or a duration such as 5m, 30m, 2h".to_string(),
            );
        }
    }
    Ok(())
}

/// Ollama accepts `-1` (never unload) or a duration string (`5m`, `30s`, `2h`).
/// Keep the stored value to that vocabulary instead of forwarding an arbitrary
/// string as `OLLAMA_KEEP_ALIVE` and as a request option.
fn valid_keep_alive(value: &str) -> bool {
    let value = value.trim();
    if value == "-1" {
        return true;
    }
    if value.is_empty() || value.len() > 32 {
        return false;
    }
    let numeric = value.strip_suffix(['s', 'm', 'h']).unwrap_or(value);
    !numeric.is_empty() && numeric.chars().all(|ch| ch.is_ascii_digit())
}

/// Read the single settings row, falling back to the defaults.
///
/// Kept separate from the command so the `ON CONFLICT` semantics below can be
/// exercised against a real database.
pub async fn read_settings(pool: &sqlx::Pool<sqlx::Sqlite>) -> Result<SettingsView, String> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT data FROM app_settings WHERE id = 'singleton'")
            .fetch_optional(pool)
            .await
            .map_err(crate::error::cmd_err)?;
    Ok(match row {
        Some((data,)) => match serde_json::from_str::<SettingsView>(&data) {
            // Older builds exposed an at-rest encryption toggle that only
            // staged a Keychain key; the database was never encrypted. Read
            // such a row back as `false` so the UI and diagnostics cannot keep
            // reporting a protection that does not exist.
            Ok(mut view) => {
                view.database_encryption = false;
                view
            }
            Err(err) => {
                // A corrupt row must stay visible instead of silently
                // resetting every preference to the defaults: log it and
                // surface an error so the caller can preserve the row.
                tracing::error!(%err, "app_settings singleton is corrupt; preserving row");
                return Err(format!("settings data is corrupt: {err}"));
            }
        },
        None => SettingsView::default(),
    })
}

/// Replace the single settings row.
///
/// The incoming document is merged over the stored one (input wins) instead of
/// blindly overwriting it, so a key written by a newer build — or a
/// concurrent writer — is preserved rather than dropped by a read-modify-write
/// race. Unknown keys survive because the merge happens on raw JSON values.
pub async fn write_settings(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    input: SettingsView,
) -> Result<SettingsView, String> {
    validate_settings(&input)?;
    let mut merged = serde_json::Value::Object(Default::default());
    let existing: Option<(String,)> =
        sqlx::query_as("SELECT data FROM app_settings WHERE id = 'singleton'")
            .fetch_optional(pool)
            .await
            .map_err(crate::error::cmd_err)?;
    if let Some((raw,)) = existing {
        if let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(&raw)
        {
            merged = serde_json::Value::Object(map);
        }
    }
    let input_value = serde_json::to_value(&input).map_err(crate::error::cmd_err)?;
    if let (serde_json::Value::Object(base), serde_json::Value::Object(patch)) =
        (&mut merged, input_value)
    {
        for (k, v) in patch {
            base.insert(k, v);
        }
    }
    let json = serde_json::to_string(&merged).map_err(crate::error::cmd_err)?;
    sqlx::query(
        "INSERT INTO app_settings (id, owner_id, data) VALUES ('singleton', ?, ?) \
         ON CONFLICT(id) DO UPDATE SET data = excluded.data",
    )
    .bind(mixd_storage::records::SINGLE_USER_ID)
    .bind(&json)
    .execute(pool)
    .await
    .map_err(crate::error::cmd_err)?;
    Ok(input)
}

#[cfg(test)]
mod tests {
    use super::{valid_keep_alive, SettingsView, DEFAULT_LOCAL_CONTEXT_WINDOW};

    #[test]
    fn keep_alive_accepts_only_ollama_durations() {
        for value in ["-1", "0", "5s", "30m", "2h"] {
            assert!(valid_keep_alive(value), "{value}");
        }
        for value in ["", " ", "5d", "-2", "5m30s", "infinity", "5 m"] {
            assert!(!valid_keep_alive(value), "{value}");
        }
    }

    #[test]
    fn auto_start_is_opt_in() {
        let settings = SettingsView::default();
        assert!(!settings.local_auto_start);
        assert_eq!(settings.local_context_window, None);
        assert_eq!(DEFAULT_LOCAL_CONTEXT_WINDOW, 8192);
        // A system-wide search surface is opt-in.
        assert!(!settings.spotlight_enabled);
        assert!(!settings.spotlight_index_body);
    }
}
