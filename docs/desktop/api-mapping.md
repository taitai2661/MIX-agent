# MIX-agent Desktop — API Mapping (placeholder)

Docker 版の `routes.py` URL と Desktop 版の `#[tauri::command]` 名を対応付ける表。
実装が進むにつれ Phase 1 / Phase 2 / … で追記していく。

## Health

| Docker 版 | Desktop 版 |
| --- | --- |
| `GET /health` | `health()` |

## Providers

| Docker 版 | Desktop 版 | Phase |
| --- | --- | --- |
| `GET /providers` | `list_providers()` | 1 |
| `POST /providers` | `create_provider()` | 1 |
| `PATCH /providers/{key}` | `update_provider()` | 1 |
| `DELETE /providers/{key}` | `delete_provider()` | 1 |
| `GET /provider-presets` | `provider_presets()` | 1 |
| `POST /providers/{key}/{action}` | `run_provider_action()` | 1 |

## Models

| Docker 版 | Desktop 版 | Phase |
| --- | --- | --- |
| `GET /models` | `list_models()` | 1 |
| `POST /models/{key}/verify-tools` | `verify_tools()` | 1 |

## Conversations

| Docker 版 | Desktop 版 | Phase |
| --- | --- | --- |
| `GET /conversations` | `list_conversations()` | 1 |
| `POST /conversations/{key}/messages` | `send_message()` | 1 |
| `DELETE /conversations/{key}` | `delete_conversation()` | 1 |
| `GET /conversations/{key}/markdown` | `export_markdown()` | 5 |

## Runs

| Docker 版 | Desktop 版 | Phase |
| --- | --- | --- |
| `GET /runs/{key}` | `get_run()` | 1 |
| `POST /runs/{key}/cancel` | `cancel_run()` | 1 |
| `GET /runs/{key}/checkpoints` | `list_checkpoints()` | 3 |
| `POST /runs/{key}/resume` | `resume_run()` | 3 |
| `POST /runs/{key}/budget-extension` | `budget_extension()` | 3 |
| `GET /runs/{key}/events` (SSE) | `RunEngine.subscribe()` + `app::emit()` | 1 |

## Approvals

| Docker 版 | Desktop 版 | Phase |
| --- | --- | --- |
| `POST /approvals/{key}/decision` | `decide_approval()` | 2 |

## Tools

| Docker 版 | Desktop 版 | Phase |
| --- | --- | --- |
| `GET /tools` | `list_tools()` | 2 |
| `POST /permission-rules` | `set_permission_rule()` | 2 |

## MCP

| Docker 版 | Desktop 版 | Phase |
| --- | --- | --- |
| `GET /mcp/connections` | `mcp::list_connections()` | 4 |
| `POST /mcp/connections` | `mcp::create_connection()` | 4 |
| `DELETE /mcp/connections/{key}` | `mcp::delete_connection()` | 4 |
| `POST /mcp/connections/{key}/oauth/start` | `mcp::start_oauth()` | 4 |

## Memories

| Docker 版 | Desktop 版 | Phase |
| --- | --- | --- |
| `GET /memories` | `memories::list_memories()` | 5 |
| `POST /memories` | `memories::create_memory()` | 5 |
| `PATCH /memories/{key}` | `memories::update_memory()` | 5 |

## Skills

| Docker 版 | Desktop 版 | Phase |
| --- | --- | --- |
| `GET /skills` | `skills::list_skills()` | 5 |
| `POST /skills/import` | `skills::import_skill()` | 5 |
| `GET /skills/{key}/export.zip` | `skills::export_skill()` | 5 |

## Projects

| Docker 版 | Desktop 版 | Phase |
| --- | --- | --- |
| `GET /projects` | `projects::list_projects()` | 5 |
| `POST /projects` | `projects::create_project()` | 5 |
| `PATCH /projects/{key}` | `projects::update_project()` | 7 |
| `DELETE /projects/{key}` | `projects::delete_project()` | 5 |
| `PATCH /conversations/{key}/state` (`project_id`) | `conversations::set_conversation_project()` | 7 |

## Workspace / Git (Desktop only)

| Docker 版 | Desktop 版 | Phase |
| --- | --- | --- |
| — | `workspace::list_dir()` | 7 |
| — | `workspace::read_workspace_file()` | 7 |
| — | `workspace::git_status()` | 7 |
| — | `workspace::git_diff()` | 7 |

## Schedules

| Docker 版 | Desktop 版 | Phase |
| --- | --- | --- |
| `GET /scheduled-jobs` | `schedules::list_jobs()` | 5 |
| `POST /scheduled-jobs` | `schedules::create_job()` | 5 |
| `DELETE /scheduled-jobs/{key}` | `schedules::delete_job()` | 5 |

## Settings

| Docker 版 | Desktop 版 | Phase |
| --- | --- | --- |
| `GET /settings` | `settings::get_settings()` | 1 |
| `PUT /settings` | `settings::update_settings()` | 1 |

## Agents

| Docker 版 | Desktop 版 | Phase |
| --- | --- | --- |
| `GET /agents` | `agents::list_agents()` | 5 |
| `POST /agents` | `agents::create_agent()` | 5 |
| `PATCH /agents/{key}` | `agents::update_agent()` | 5 |

## Backups

| Docker 版 | Desktop 版 | Phase |
| --- | --- | --- |
| `POST /backups` | `backups::create_backup()` | 6 |
| `POST /backups/restore` | `backups::restore_backup()` | 6 |

## Diagnostics

| Docker 版 | Desktop 版 | Phase |
| --- | --- | --- |
| `GET /settings/diagnostics` | `diagnostics::collect_diagnostics()` | 6 |
