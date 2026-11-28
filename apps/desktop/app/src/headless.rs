//! `mix-agent run` — a real headless agent run.
//!
//! The GUI owns a window, a tray and a WebView, but none of those are needed
//! to drive the engine: storage, providers, tools and the Run Engine are plain
//! Rust. This module wires the same pieces `build_app_state` does (minus the
//! browser and local-runtime manager, which need Tauri) and streams the Run's
//! events to stdout.
//!
//! It is deliberately a *separate* entry point rather than a flag on the GUI
//! launch: a CI job must not open a window, and a half-wired runner would be
//! worse than none. The run is ephemeral by default (the conversation and run
//! rows are removed afterwards) so scheduled/machine use does not fill the
//! user's history; `--keep` opts out.

use std::io::{IsTerminal, Read, Write};
use std::sync::Arc;

use mixd_core::mode::Mode;
use mixd_core::runs::budget::Budget;
use mixd_core::runs::engine::{RunEngine, RunRequest};
use mixd_core::runs::events::RunEvent;
use mixd_core::runs::state::RunStatus;
use mixd_storage::{SecretStore, Storage};
use serde_json::json;
use uuid::Uuid;

use crate::state;

#[derive(Default)]
struct Args {
    workspace: Option<String>,
    prompt: Option<String>,
    model: Option<String>,
    provider: Option<String>,
    mode: Option<String>,
    yes: bool,
    json: bool,
    keep: bool,
    max_steps: Option<u32>,
    help: bool,
}

pub fn run_if_requested() -> Option<i32> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.first().map(String::as_str) != Some("run") {
        return None;
    }
    let args = match parse_args(&argv[1..]) {
        Ok(args) => args,
        Err(err) => {
            eprintln!("mix-agent run: {err}\n\n{}", run_help());
            return Some(2);
        }
    };
    if args.help {
        println!("{}", run_help());
        return Some(0);
    }
    Some(run(args))
}

fn run_help() -> &'static str {
    "USAGE:\n    mix-agent run [OPTIONS] [PROMPT]\n\n\
     OPTIONS:\n\
     \x20   -C, --workspace <DIR>   Workspace folder (default: current directory)\n\
     \x20   -p, --prompt <TEXT>     Prompt (also read from stdin when piped)\n\
     \x20       --model <ID>        Model id (default: first enabled model)\n\
     \x20       --provider <ID>     Provider id (inferred from the model when omitted)\n\
     \x20       --mode <MODE>       chat | thinking | plan | agent (default: agent)\n\
     \x20   -y, --yes               Approve every tool call (including dangerous ones)\n\
     \x20       --json              Stream Run events as NDJSON\n\
     \x20       --keep              Keep the conversation and run rows\n\
     \x20       --max-steps <N>     Step ceiling for this run\n\
     \x20   -h, --help              Show this help\n\n\
     Exit codes: 0 success, 1 run failed, 2 usage error, 130 cancelled."
}

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut args = Args::default();
    let mut positional: Vec<String> = Vec::new();
    let mut index = 0;
    while index < argv.len() {
        let value = argv[index].as_str();
        match value {
            "-C" | "--workspace" => args.workspace = Some(take_value(argv, &mut index, value)?),
            "-p" | "--prompt" => args.prompt = Some(take_value(argv, &mut index, value)?),
            "--model" => args.model = Some(take_value(argv, &mut index, value)?),
            "--provider" => args.provider = Some(take_value(argv, &mut index, value)?),
            "--mode" => args.mode = Some(take_value(argv, &mut index, value)?),
            "--max-steps" => {
                let raw = take_value(argv, &mut index, value)?;
                args.max_steps = Some(
                    raw.parse()
                        .map_err(|_| format!("--max-steps must be a positive integer: {raw}"))?,
                );
            }
            "-y" | "--yes" => args.yes = true,
            "--json" => args.json = true,
            "--keep" => args.keep = true,
            "-h" | "--help" => args.help = true,
            other if other.starts_with('-') => {
                return Err(format!("unknown option: {other}"));
            }
            other => positional.push(other.to_string()),
        }
        index += 1;
    }
    if args.prompt.is_none() && !positional.is_empty() {
        args.prompt = Some(positional.join(" "));
    }
    Ok(args)
}

fn take_value(argv: &[String], index: &mut usize, name: &str) -> Result<String, String> {
    *index += 1;
    argv.get(*index)
        .cloned()
        .ok_or_else(|| format!("{name} requires a value"))
}

fn run(args: Args) -> i32 {
    // The GUI logs at info by default; a CLI's stdout is a contract and should
    // not be buried in storage/migration lines. `RUST_LOG` still wins.
    if std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var("RUST_LOG", "warn");
    }
    crate::startup::init_tracing();
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("mix-agent run: could not start the async runtime: {err}");
            return 1;
        }
    };
    match runtime.block_on(execute(args)) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("mix-agent run: {err:#}");
            1
        }
    }
}

async fn execute(mut args: Args) -> anyhow::Result<i32> {
    let prompt = match args.prompt.take() {
        Some(prompt) if !prompt.trim().is_empty() => prompt,
        _ if !std::io::stdin().is_terminal() => {
            let mut buffer = String::new();
            std::io::stdin().read_to_string(&mut buffer)?;
            if buffer.trim().is_empty() {
                anyhow::bail!("no prompt: pass one as an argument or pipe it on stdin");
            }
            buffer
        }
        _ => anyhow::bail!("no prompt: pass one as an argument or pipe it on stdin"),
    };
    let mode: Mode = args
        .mode
        .as_deref()
        .unwrap_or("agent")
        .parse()
        .map_err(|err| anyhow::anyhow!("unknown mode: {err}"))?;
    let workspace =
        std::path::PathBuf::from(args.workspace.clone().unwrap_or_else(|| ".".to_string()))
            .canonicalize()
            .map_err(|err| anyhow::anyhow!("workspace not found: {err}"))?;
    if !workspace.is_dir() {
        anyhow::bail!("workspace is not a directory: {}", workspace.display());
    }

    let data_dir = crate::cli::data_dir();
    std::fs::create_dir_all(&data_dir)?;
    let storage = Arc::new(Storage::open(&data_dir).await?);
    let secrets = Arc::new(SecretStore::open(&data_dir));
    let settings = crate::commands::settings::read_settings(storage.pool())
        .await
        .unwrap_or_default();
    let egress = state::load_egress_policy(storage.as_ref()).await;

    let mcp = Arc::new(crate::McpHost::new());
    let agent_runner = Arc::new(crate::EngineAgentRunner::default());
    let embeddings = Arc::new(crate::embedding::EmbeddingResolver::new(
        storage.clone(),
        secrets.clone(),
    ));
    let tools = state::build_registry(
        &storage,
        None,
        &mcp,
        &agent_runner,
        &embeddings,
        egress,
        settings.discover_user_skills,
    );
    let engine = Arc::new(RunEngine::with_storage_and_tools(
        storage.clone(),
        secrets.clone(),
        tools,
    ));
    agent_runner.attach(&engine, &secrets, "");
    // A keychain-less CI host still needs its stored API-key providers.
    state::register_providers_from_storage(&engine.providers, &storage, &secrets).await?;
    let sandbox = settings.sandbox_terminal && mixd_tools::terminal::os_sandbox_available();
    engine.set_terminal_sandbox(sandbox).await;

    // Non-interactive policy: `--yes` approves, otherwise every prompt is
    // denied immediately. Without this the run would park on a prompt whose
    // TTL is a day.
    engine.executor.set_headless_approval(Some(args.yes));
    if !args.yes {
        eprintln!(
            "mix-agent run: tool approvals are denied (pass --yes to approve them; read-only tools still run)"
        );
    }

    let (provider_id, model_id, context_window) =
        resolve_model(&storage, args.model.as_deref(), args.provider.as_deref()).await?;
    let workspace_id = workspace_id_for(&storage, &workspace).await;
    let (conversation_id, run_id) = create_rows(
        &storage,
        &workspace,
        workspace_id,
        &prompt,
        mode,
        &provider_id,
        &model_id,
        args.max_steps,
    )
    .await?;
    // Write the user turn before the Run starts, matching the app layer. This
    // only matters when `--keep` is used (otherwise the conversation is
    // removed), but it keeps the transcript coherent for an inspected run.
    insert_message(
        &storage,
        conversation_id,
        run_id,
        "user",
        &prompt,
        &model_id,
    )
    .await?;

    let request = RunRequest {
        run_id,
        conversation_id,
        workspace_id,
        root_path: Some(workspace.to_string_lossy().to_string()),
        additional_paths: Vec::new(),
        mode,
        model_id: model_id.clone(),
        provider_id: provider_id.clone(),
        system: None,
        verify_command: None,
        format_command: None,
        context_window,
        temperature: None,
        reasoning: None,
        max_tokens: None,
        keep_alive: None,
        temporary: false,
        tools: std::sync::Arc::new(engine.tool_specs().await),
        history: Vec::new(),
        user_message: prompt.clone(),
        resume_from: None,
        auto_fallbacks: Vec::new(),
        step_models: Vec::new(),
        monthly_pressure: 0.0,
    };

    let mut events = engine.subscribe();
    let execute = engine.execute(request);
    tokio::pin!(execute);
    let mut events_open = true;
    let outcome = loop {
        tokio::select! {
            result = &mut execute => break result?,
            event = events.recv(), if events_open => {
                match event {
                    Ok(event) => print_event(&event, args.json),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        eprintln!("mix-agent run: dropped {skipped} live event(s)");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        events_open = false;
                    }
                }
            }
        }
    };
    while let Ok(event) = events.try_recv() {
        print_event(&event, args.json);
    }

    if args.json {
        println!(
            "{}",
            serde_json::to_string(&json!({
                "type": "result",
                "run_id": run_id,
                "conversation_id": conversation_id,
                "status": outcome.status.as_str(),
                "text": outcome.text,
                "tool_calls": outcome.tool_calls.len(),
                "model_id": model_id,
                "provider_id": provider_id,
            }))?
        );
    } else {
        if !outcome.text.is_empty() && !outcome.text.ends_with('\n') {
            println!();
        }
        eprintln!(
            "mix-agent run: {} ({} tool call(s))",
            outcome.status.as_str(),
            outcome.tool_calls.len()
        );
    }

    if args.keep {
        insert_message(
            &storage,
            conversation_id,
            run_id,
            "assistant",
            &outcome.text,
            &model_id,
        )
        .await?;
    } else {
        cleanup_rows(&storage, conversation_id, run_id).await;
    }

    Ok(match outcome.status {
        RunStatus::Completed => 0,
        RunStatus::Cancelled => 130,
        _ => 1,
    })
}

async fn resolve_model(
    storage: &Storage,
    model: Option<&str>,
    provider: Option<&str>,
) -> anyhow::Result<(String, String, u32)> {
    type Row = (String, Option<String>, i64);
    let pool = storage.pool();
    if let Some(model_id) = model {
        let row: Option<Row> = sqlx::query_as(
            "SELECT id, json_extract(data, '$.providerId'), \
                    COALESCE(json_extract(data, '$.contextWindow'), 0) \
               FROM models WHERE id = ?",
        )
        .bind(model_id)
        .fetch_optional(pool)
        .await?;
        let Some((id, stored_provider, context_window)) = row else {
            anyhow::bail!("model not found: {model_id} (open the app once so it is discovered)");
        };
        let provider_id = provider
            .map(str::to_string)
            .or(stored_provider)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("model {id} has no provider; pass --provider"))?;
        return Ok((provider_id, id, context_window.max(0) as u32));
    }

    let row: Option<Row> = if let Some(provider_id) = provider {
        sqlx::query_as(
            "SELECT id, json_extract(data, '$.providerId'), \
                    COALESCE(json_extract(data, '$.contextWindow'), 0) \
               FROM models WHERE json_extract(data, '$.providerId') = ? \
               ORDER BY created_at LIMIT 1",
        )
        .bind(provider_id)
        .fetch_optional(pool)
        .await?
    } else {
        sqlx::query_as(
            "SELECT m.id, json_extract(m.data, '$.providerId'), \
                    COALESCE(json_extract(m.data, '$.contextWindow'), 0) \
               FROM models m JOIN providers p ON p.id = json_extract(m.data, '$.providerId') \
              WHERE COALESCE(json_extract(p.data, '$.enabled'), 1) = 1 \
              ORDER BY m.created_at LIMIT 1",
        )
        .fetch_optional(pool)
        .await?
    };
    let Some((id, provider_id, context_window)) = row else {
        anyhow::bail!(
            "no model is configured; add a provider and discover its models in the app, or pass --model/--provider"
        );
    };
    let provider_id = provider_id
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("model {id} has no provider; pass --provider"))?;
    Ok((provider_id, id, context_window.max(0) as u32))
}

async fn workspace_id_for(storage: &Storage, workspace: &std::path::Path) -> Uuid {
    let root = workspace.to_string_lossy().to_string();
    let existing: Option<String> = sqlx::query_scalar(
        "SELECT id FROM projects WHERE json_extract(data, '$.rootPath') = ? LIMIT 1",
    )
    .bind(&root)
    .fetch_optional(storage.pool())
    .await
    .ok()
    .flatten();
    existing
        .and_then(|id| Uuid::parse_str(&id).ok())
        .unwrap_or_else(Uuid::new_v4)
}

#[allow(clippy::too_many_arguments)]
async fn create_rows(
    storage: &Storage,
    workspace: &std::path::Path,
    workspace_id: Uuid,
    prompt: &str,
    mode: Mode,
    provider_id: &str,
    model_id: &str,
    max_steps: Option<u32>,
) -> anyhow::Result<(Uuid, Uuid)> {
    let conversation_id = Uuid::new_v4();
    let run_id = Uuid::new_v4();
    let now = chrono::Utc::now().to_rfc3339();
    let title: String = prompt.trim().chars().take(60).collect();
    let conversation = json!({
        "title": if title.is_empty() { "CLI run" } else { &title },
        "mode": mode.as_str(),
        "modelId": model_id,
        "workspaceId": workspace_id.to_string(),
        "projectId": null,
        "createdAt": now,
        "updatedAt": now,
    });
    sqlx::query("INSERT INTO conversations (id, owner_id, data) VALUES (?, ?, ?)")
        .bind(conversation_id.to_string())
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(conversation.to_string())
        .execute(storage.pool())
        .await?;

    let mut budget = Budget::for_mode(mode);
    if let Some(steps) = max_steps {
        budget.step_limit = steps;
    }
    let run_data = json!({
        "providerId": provider_id,
        "modelId": model_id,
        "workspaceId": workspace_id.to_string(),
        "status": "running",
        "stepCount": 0,
        "toolCallCount": 0,
        "startedAt": now,
        "updatedAt": now,
        "summary": "",
        "budget": budget.to_json(),
        "temporaryMode": false,
        "cli": true,
        "rootPath": workspace.to_string_lossy(),
    });
    sqlx::query(
        "INSERT INTO runs (id, owner_id, data, conversation_id, status, request_key) \
         VALUES (?, ?, ?, ?, 'running', ?)",
    )
    .bind(run_id.to_string())
    .bind(mixd_storage::records::SINGLE_USER_ID)
    .bind(run_data.to_string())
    .bind(conversation_id.to_string())
    .bind(run_id.to_string())
    .execute(storage.pool())
    .await?;
    Ok((conversation_id, run_id))
}

async fn insert_message(
    storage: &Storage,
    conversation_id: Uuid,
    run_id: Uuid,
    role: &str,
    content: &str,
    model_id: &str,
) -> anyhow::Result<()> {
    let data = json!({
        "role": role,
        "content": content,
        "modelId": model_id,
        "runId": run_id.to_string(),
        "toolCalls": [],
        "attachments": [],
        "kind": role,
    });
    sqlx::query("INSERT INTO messages (id, owner_id, data, conversation_id) VALUES (?, ?, ?, ?)")
        .bind(Uuid::new_v4().to_string())
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(data.to_string())
        .bind(conversation_id.to_string())
        .execute(storage.pool())
        .await?;
    Ok(())
}

async fn cleanup_rows(storage: &Storage, conversation_id: Uuid, run_id: Uuid) {
    let pool = storage.pool();
    for sql in [
        "DELETE FROM approvals WHERE run_id = ?",
        "DELETE FROM tool_calls WHERE run_id = ?",
        "DELETE FROM run_events WHERE run_id = ?",
        "DELETE FROM run_checkpoints WHERE run_id = ?",
        "DELETE FROM runs WHERE id = ?",
        "DELETE FROM messages WHERE conversation_id = ?",
        "DELETE FROM conversations WHERE id = ?",
    ] {
        let bind = if sql.contains("conversation") {
            conversation_id.to_string()
        } else {
            run_id.to_string()
        };
        let _ = sqlx::query(sql).bind(bind).execute(pool).await;
    }
}

fn print_event(event: &RunEvent, json: bool) {
    if json {
        if let Ok(line) = serde_json::to_string(event) {
            println!("{line}");
        }
        return;
    }
    match event.kind.as_str() {
        "text" => {
            if let Some(delta) = event.data.get("delta").and_then(|value| value.as_str()) {
                print!("{delta}");
                let _ = std::io::stdout().flush();
            }
        }
        "tool_started" | "tool_call_started" => {
            let name = event
                .data
                .get("name")
                .and_then(|value| value.as_str())
                .unwrap_or("tool");
            eprintln!("[tool] {name}");
        }
        "error" => {
            let message = event
                .data
                .get("message")
                .and_then(|value| value.as_str())
                .unwrap_or("run error");
            eprintln!("[error] {message}");
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn parses_workspace_prompt_and_flags() {
        let args = parse_args(&argv(&[
            "-C",
            "/tmp/project",
            "--mode",
            "plan",
            "--yes",
            "--json",
            "fix the failing test",
        ]))
        .expect("parse");
        assert_eq!(args.workspace.as_deref(), Some("/tmp/project"));
        assert_eq!(args.mode.as_deref(), Some("plan"));
        assert_eq!(args.prompt.as_deref(), Some("fix the failing test"));
        assert!(args.yes);
        assert!(args.json);
        assert!(!args.keep);
    }

    #[test]
    fn rejects_an_unknown_flag_instead_of_treating_it_as_prompt() {
        assert!(parse_args(&argv(&["--nope"])).is_err());
    }

    #[test]
    fn max_steps_requires_a_number() {
        assert!(parse_args(&argv(&["--max-steps", "many", "hi"])).is_err());
        let args = parse_args(&argv(&["--max-steps", "7", "hi"])).expect("parse");
        assert_eq!(args.max_steps, Some(7));
    }
}
