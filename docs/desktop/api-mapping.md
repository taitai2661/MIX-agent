# MIX-agent Desktop — API Mapping

Docker 版の `routes.py` URL と Desktop 版の `#[tauri::command]` 名を対応付ける表。

Desktop 側は HTTP ではなく Tauri の `invoke` Surface なので、URL は
`POST /runs/{key}/cancel` のような「メソッド + パス」形で書いています。
Desktop 版に Docker 版と同じ REST 層はありません。

`Desktop 版` の列の値は `app/src/lib.rs` の `tauri::generate_handler!` に
登録済みのコマンド名です。登録漏れや改名は CI の `rust:check` では検出できないため、
ここを編集するときは必ず `app/src/lib.rs` を確認してください。

Docker 版にない Desktop 固有のコマンドは「—」として示します。

`Phase` 列は「そのコマンドが属する機能領域が Desktop 版のどのフェーズで
導入されたか」を [`architecture.md`](architecture.md) のフェーズ表から割り当てた
ものです（Feature 表そのものではありません）。フェーズ 0〜6 は完了、7 は
macOS 完成 + Windows α + Linux α の未着手です。

## Health

| Docker 版 | Desktop 版 | Phase |
| --- | --- | --- |
| `GET /health` | `health::health()` | 1 |

## Providers / 接続先検出

| Docker 版 | Desktop 版 | Phase |
| --- | --- | --- |
| `GET /providers` | `providers::list_providers()` | 1 |
| `POST /providers` | `providers::create_provider()` | 1 |
| `PATCH /providers/{key}` | `providers::update_provider()` | 1 |
| `DELETE /providers/{key}` | `providers::delete_provider()` | 1 |
| `GET /provider-presets` | `providers::provider_presets()` | 1 |
| `POST /providers/{key}/{action}` | `providers::connect_provider()` | 1 |
| — | `discovery::scan_local_runtimes()` | 5 |
| — | `discovery::probe_local_endpoint()` | 5 |
| — | `discovery::import_local_runtime()` | 5 |

`connect_provider()` は Docker 版 `{action}` のうち「モデル取得」だけを担います。
接続テストに相当する操作はプロバイダの作成・更新時に行われます。

## Models

| Docker 版 | Desktop 版 | Phase |
| --- | --- | --- |
| `GET /models` | `models::list_models()` | 1 |
| `POST /models/{key}/verify-tools` | `models::verify_tools()` | 1 |

## Conversations

| Docker 版 | Desktop 版 | Phase |
| --- | --- | --- |
| `GET /conversations` | `conversations::list_conversations()` | 1 |
| `POST /conversations` | `conversations::create_conversation()` | 1 |
| `PATCH /conversations/{key}` | `conversations::rename_conversation()` | 1 |
| `POST /conversations/{key}/messages` | `conversations::send_message()` | 1 |
| `GET /conversations/{key}/messages` | `conversations::list_messages()` | 1 |
| `DELETE /conversations/{key}` | `conversations::delete_conversation()` | 1 |
| `GET /conversations/{key}/markdown` | `conversations::export_markdown()` | 5 |
| `PATCH /conversations/{key}/state` (`project_id`) | `conversations::set_conversation_project()` | 5 |

## Runs

| Docker 版 | Desktop 版 | Phase |
| --- | --- | --- |
| `GET /runs/{key}` | `runs::get_run()` | 1 |
| `GET /runs` | `runs::list_runs()` | 3 |
| `POST /runs/{key}/cancel` | `runs::cancel_run()` | 1 |
| `GET /runs/{key}/events` (SSE) | `RunEngine.subscribe()` + `run_events::spawn()` | 1 |
| `GET /runs/{key}/events` (再接続時の巻き戻し) | `runs::list_run_events()` | 1 |
| `GET /runs/{key}/events` (同じイベントの重複再生) | `runs::replay_run_events()` | 3 |
| `GET /runs/{key}/checkpoints` | `runs::list_checkpoints()` | 3 |
| `POST /runs/{key}/resume` | `runs::resume_run()` | 3 |
| `POST /runs/{key}/budget-extension` | `runs::budget_extension()` | 3 |

Desktop 版は SSE ではなく `app::RunEvent` を `emit` で配信します。UI は
`runs::list_run_events()` で過去分を補完し、`runs::replay_run_events()` で
途中のイベントから再購読します。

## Approvals

| Docker 版 | Desktop 版 | Phase |
| --- | --- | --- |
| `POST /approvals/{key}/decision` | `approvals::decide_approval()` | 2 |
| `GET /approvals` | `tools::list_pending_approvals()` | 2 |

## Tools / 権限

| Docker 版 | Desktop 版 | Phase |
| --- | --- | --- |
| `GET /tools` | `tools::list_tools()` | 2 |
| `POST /permission-rules` | `tools::set_permission_rule()` | 2 |
| `GET /permission-rules` | `tools::list_permission_rules()` | 2 |

## MCP

| Docker 版 | Desktop 版 | Phase |
| --- | --- | --- |
| `GET /mcp/connections` | `mcp::list_connections()` | 4 |
| `POST /mcp/connections` | `mcp::create_connection()` | 4 |
| `DELETE /mcp/connections/{key}` | `mcp::delete_connection()` | 4 |
| `POST /mcp/connections/{key}/oauth/start` | `mcp::start_oauth()` | 4 |
| `GET /mcp/oauth/callback` | `mcp::handle_oauth_callback()` | 4 |
| `POST /mcp/connections/{key}/{action}` | `mcp::connect()` | 4 |

## Browser

| Docker 版 | Desktop 版 | Phase |
| --- | --- | --- |
| `GET /browser/status` | `browser::browser_status()` | 4 |
| `GET /browser/profiles` | `browser::browser_profiles()` | 4 |
| `POST /browser/profiles/{key}/reset` | `browser::browser_reset_profile()` | 4 |
| `POST /runs/{key}/browser/{action}` | `browser::browser_invoke()` | 4 |

`GET /browser/install` / `POST /browser/enable` / `POST /browser/update` に対応する
Desktop コマンドはありません。同梱 Chromium は `scripts/fetch-chromium.mjs` が
ビルド時に取得し、UI から実行時にインストールする経路は提供しません。

## Memories

| Docker 版 | Desktop 版 | Phase |
| --- | --- | --- |
| `GET /memories` | `memories::list_memories()` | 5 |
| `POST /memories` | `memories::create_memory()` | 5 |
| `PATCH /memories/{key}` | `memories::update_memory()` | 5 |
| `DELETE /memories/{key}` | `memories::delete_memory()` | 5 |
| `GET /memories/{key}/revisions` | `memories::list_memory_revisions()` | 5 |
| `POST /memories/{key}/restore/{revision}` | `memories::restore_memory_revision()` | 5 |
| `POST /memories/{key}/resolve` | `memories::resolve_conflict()` | 5 |
| `GET /memories/{key}/associations` | `memories::memory_associations()` | 5 |
| `GET /memories-debug/search` | `memories::search_memories()` | 5 |
| `POST /memories-debug/actions` | `memories::verify_memory()` | 5 |

## Knowledge

| Docker 版 | Desktop 版 | Phase |
| --- | --- | --- |
| — | `knowledge::list_knowledge()` | 5 |
| — | `knowledge::ingest_knowledge()` | 5 |
| — | `knowledge::import_knowledge_file()` | 5 |
| — | `knowledge::delete_knowledge()` | 5 |
| — | `knowledge::search_knowledge()` | 5 |

**両エディションとも Knowledge に REST リソースはありません。** Docker 版も
`docs/openapi.json` に `/api/v1/knowledge` は存在せず、Agent tool
（`knowledge_add` / `knowledge_search` / `knowledge_delete`）としてのみ公開して
います。Desktop 版は tool に加えて上記 5 コマンドで UI から直接操作します。

## Skills

| Docker 版 | Desktop 版 | Phase |
| --- | --- | --- |
| `GET /skills` | `skills::list_skills()` | 5 |
| `POST /skills/import` | `skills::import_skill()` | 5 |
| `GET /skills/{key}/export.zip` | `skills::export_skill()` | 5 |
| `DELETE /skills/{key}` | `skills::delete_skill()` | 5 |
| `GET /skills/{key}/resources` | `skills::list_skill_resources()` | 5 |
| `GET /skills/{key}/resources/{path}` | `skills::read_skill_resource()` | 5 |

## Projects

| Docker 版 | Desktop 版 | Phase |
| --- | --- | --- |
| `GET /projects` | `projects::list_projects()` | 5 |
| `POST /projects` | `projects::create_project()` | 5 |
| `PATCH /projects/{key}` | `projects::update_project()` | 5 |
| `DELETE /projects/{key}` | `projects::delete_project()` | 5 |

## Workspace / Git / ファイル (Desktop only)

Docker 版にはローカル作業フォルダを直接扱う equivalent がないため、全て Desktop 固有です。

| Docker 版 | Desktop 版 | Phase |
| --- | --- | --- |
| — | `workspace::list_dir()` | 5 |
| — | `workspace::read_workspace_file()` | 5 |
| — | `workspace::git_status()` | 5 |
| — | `workspace::git_diff()` | 5 |
| — | `fs::preview_file_diff()` | 5 |
| — | `fs::read_attachment()` | 5 |
| — | `fs::save_text_file()` | 5 |
| — | `fs::save_base64_file()` | 5 |
| — | `fs::open_external()` | 6 |

`fs::open_external()` は `http` / `https` の URL のみを受け付け、OS の既定
ブラウザで開きます（`file://` などは拒否）。

## Schedules

| Docker 版 | Desktop 版 | Phase |
| --- | --- | --- |
| `GET /scheduled-jobs` | `schedules::list_jobs()` | 5 |
| `POST /scheduled-jobs` | `schedules::create_job()` | 5 |
| `PATCH /scheduled-jobs/{key}` | `schedules::set_job_enabled()` | 5 |
| `DELETE /scheduled-jobs/{key}` | `schedules::delete_job()` | 5 |
| — | `schedules::preview_cron()` | 5 |

`POST /scheduled-jobs/{key}/run`（手動実行）に対応する Desktop コマンドは
ありません。

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
| `DELETE /agents/{key}` | `agents::delete_agent()` | 5 |

## Backups

| Docker 版 | Desktop 版 | Phase |
| --- | --- | --- |
| `POST /backups` | `backups::create_backup()` | 6 |
| `POST /backups/restore` | `backups::restore_backup()` | 6 |
| — | `backups::relaunch_app()` | 6 |

## Diagnostics

| Docker 版 | Desktop 版 | Phase |
| --- | --- | --- |
| `GET /settings/diagnostics` | `diagnostics::collect_diagnostics()` | 6 |

## 自動更新 (Desktop only)

| Docker 版 | Desktop 版 | Phase |
| --- | --- | --- |
| — | `update::app_version()` | 6 |
| — | `update::check_for_update()` | 6 |
| — | `update::install_update()` | 6 |

Tauri updater plugin は Rust 側からのみ駆動し、WebView には `updater:*` 権限を渡しません。`check_for_update` は `latest.json` を取得して新バージョンの有無を返し、`install_update` は再取得・署名検証・ダウンロード・インストールを行います（進捗は `update://progress` イベント）。詳細は [`architecture.md`](architecture.md) の「更新方法」を参照してください。

## 対応していない Docker 版エンドポイント

Desktop 版に意図的に対応が無いもの。`docs/openapi.json` に存在する
`/api/v1` パスから、このドキュメントの各表で Desktop 側を割り当てていないものを
抜き出したものです。

| Docker 版 | 理由 |
| --- | --- |
| `/auth/*`（login / logout / me / login-history / sessions / password / username） | 単一ユーザー前提。OS アカウントで運用する |
| `/setup`, `/setup/admin` | 初回管理者作成が不要 |
| `/notifications/*`, `/push/*`, `/push-sw.js` | Desktop は OS ローカル通知のみ（`app/src/push.rs`）。Web Push は無い |
| `/conversation-folders/*` | 未実装 |
| `/conversations/{key}/restore`, `/conversations/{key}/tool-calls` | 会話の復元と Tool 実行履歴一覧は未実装 |
| `/mcp/registry/*`, `/mcp/oauth/client-metadata.json` | MCP パッケージレジストリの参照・導入は未実装（接続設定の編集のみ） |
| `/mcp/connections/{key}/{reinstall,uninstall,update}` | MCP パッケージのライフサイクル操作は未実装 |
| `/runs/{key}/browser-manual`, `/browser-pause`, `/browser-return` | Browser の手動制御は未実装 |
| `/settings/statistics`, `/settings/usage` | 未実装 |
| `/skills/{key}/revisions`, `/skills/{key}/restore/{revision}`, `/skills/import/archive`, `/skills/import/directory` | Skill の revision 履歴と Server ディレクトリ取り込みは未実装（Memory には revision がある） |
| `/messages/{key}/feedback` | 未実装 |
| `/artifacts/*`, `/runs/{key}/browser-frames*` | Artifact / Browser frame の配信は未実装（Browser のフレームは Desktop 側で直接取得） |
