# MIX-agent Desktop — Architecture（Phase 5 完了時点）

## 目的

`docker/MIX-agent/` 版の中核機能（Provider Adapter / Run Engine / Tool Registry / SSE 契約 / Capability 3 状態 / Checkpoint / Stagnation / Verification / Tiered Summary / Budget Extension / 暗号化 Secret）を **Tauri 2 + Rust + React 19** のネイティブアプリへ再実装する。`docker/` との **コード共有ゼロ**、仕様書のみで契約を結ぶ。

## ディレクトリ

```
MIX-agent/
├── docker/MIX-agent/           # Docker 版（変更なし）
├── desktop/mix-agent-desktop/  # Desktop 版（Cargo + npm workspace）
│   ├── app/         Tauri 2 shell + #[tauri::command] surface
│   ├── core/        Run Engine / state / events / context
│   ├── providers/   OpenAI / Anthropic / Gemini / OpenAI-Compatible / Ollama / LM Studio
│   ├── tools/       Tool Registry + executor
│   ├── browser/     Browser Operator + chromiumoxide backend
│   ├── mcp/         MCP client host
│   ├── storage/     SQLite + Keychain
│   └── ui/          React 19 + Vite + Tailwind
└── docs/desktop/    Desktop 版設計資料（本ファイル）
```

## 信頼境界

```
┌──────────────────────────────────────────────────────────────┐
│ Tauri App (1 OSプロセス / tokio runtime)                     │
│                                                              │
│  WebView ─ invoke('cmd') ─► #[tauri::command]   ◄── emit()   │
│  (WKWebView / WebView2 / WebKitGTK)                          │
│                                                              │
│  mixd-core (Run Engine, events, context)                     │
│  mixd-providers (Provider adapters)                          │
│  mixd-tools (Tool Registry, executor)                        │
│  mixd-browser (Browser Operator, chromiumoxide backend)      │
│  mixd-mcp (rmcp host, OAuth PKCE)                            │
│  mixd-storage (sqlx+SQLite WAL, keyring-rs)                  │
└──────────────────────────────────────────────────────────────┘
```

WebView からは `fs` / `shell` / `http` プラグインを **露出しない**（`app/capabilities/default.toml` で deny）。すべての操作が `#[tauri::command]` 経由となり、Rust が信頼の単一源泉。

## Docker 版との整合点

| Docker 版 | Desktop 版 | 共有方針 |
| --- | --- | --- |
| `routes.py` の URL 契約 | `#[tauri::command]` 引数・戻り JSON | 形を完全一致 |
| SSE `kind` 値一覧 | `app::RunEvent` 列挙体 | 形を完全一致 |
| Capability 3 状態 | `mixd_core::routing::CapabilityState` | 仕様踏襲 |
| Checkpoint / Stagnation / Verification | `core::runs::{checkpoints,stagnation,verification}` | 仕様踏襲 |
| Tiered Summary | `core::context` | 仕様踏襲 |
| 暗号化 Secret (AES-GCM + AAD) | `storage::secrets` | 仕様踏襲 |
| pg_trgm / JSONB Index | SQLite `json_extract` + FTS5 | 再実装 |
| マルチユーザー / 管理者作成 | 単一ユーザー前提 | 削除 |
| Docker container 群 | tokio runtime 上の in-proc タスク | 置換 |

## Browser Operator

```
Agent (mixd-core::runs::engine)
   │
   ▼ call_tool("browser", ...)
Browser Tool (mixd-tools)
   │
   ▼ trait BrowserTool
Browser Tool Interface (mixd-browser::interface)
   │
   ▼
Browser Operator (mixd-browser::operator)   ← Workspace Profile 保持
   │  navigate / click / type / scroll /
   │  dom / screenshot / evaluate / wait_for
   ▼
Backend: enum BrowserBackend (chromiumoxide)  ← 差し替え可能
   │
   ▼
Bundled Chromium (~+100MB)
```

macOS で先行実装。Phase 4 で `chromiumoxide` を導入し、`BrowserBackend::Chromium` から起動する。

## セキュリティ

- **OS Keychain**: `keyring-rs` で Provider Key / DB 暗号化鍵 / MCP token / OAuth refresh token を用途別 AAD で保存。
- **DB 透過暗号化**: WAL + AES-GCM（鍵は Keychain から取得）。
- **Egress allowlist + private IP deny**: `tools::network` で reqwest の Proxy に自前実装。
- **WebView CSP**: `app/tauri.conf.json` の `security.csp` で `connect-src 'self' ipc: http://ipc.localhost` に限定。
- **Sandbox**: macOS `sandbox-exec`、Windows Job Object、Linux landlock + bubblewrap（Phase 2 以降）。

## Bundled Chromium

Desktop 版はブラウザを同梱する。取得は `scripts/fetch-chromium.mjs` が
Chrome for Testing の公開エンドポイントから行い、`app/resources/chromium/` に
下列レイアウトで展開して `.gitignore` 対象（1プラットフォーム約 370 MB）。

| OS | パス |
| --- | --- |
| macOS | `chromium/chrome-mac/Chromium.app/Contents/MacOS/Chromium` |
| Windows | `chromium/chrome-win/chrome.exe` |
| Linux | `chromium/chrome-linux/chrome` |

`tauri.conf.json` の `bundle.resources` は **map 形式**（source → target）で
`resources/chromium` を `Resources/chromium` に配置する。解決は
`mixd_browser::profile::find_bundled_chromium()` が行い、
`app.path().resource_dir()` を基準に解決する（`app_data` から推測しない）。
同梱が無い場合は system Chromium（macOS / Windows / Linux の既知パス）に
フォールバックし、それも無ければ Browser 機能を `disabled` として報告する。

```bash
node scripts/fetch-chromium.mjs              # 安定版を取得
MIX_SKIP_CHROMIUM=1 cargo check              # 取得をスキップ
MIX_CHROMIUM_VERSION=154.0.8037.57 node scripts/fetch-chromium.mjs  # 固定
```

`pnpm tauri build` の `beforeBuildCommand` が UI ビルド後に自動で取得する。
macOS 配布時は同梱の `Chromium.app` も署名対象になる。

## 自動更新

`app/src/updater.rs` が起動 20 秒後に更新を確認し、新しい版があれば
`update_available` を emit する。UI はそれを受け取ってバナーを表示し、
「Install & restart」でダウンロード → インストール → 再起動を行う。
`Help ▸ Check for Updates` からの手動確認も可能。

**既定では無効**（`tauri.conf.json` の `plugins.updater.active = false`、
`endpoints: []`）。この状態では `is_configured()` が false を返し、起動時の
チェックもインストールも no-op になる。ローカルビルドが壊れないための意図的な
仕様で、endpoint がないのに起動を失敗させてはならない。

有効化するには署名鍵とエンドポイントを用意して設定する:

```bash
# 1) 署名鍵を生成する（機密: リポジトリにコミットしないこと）
cargo tauri signer generate -w ~/.tauri/mix-agent.key \
  -p "MIX-agent Desktop" --force

# 2) 更新マニフェストを配信できるサーバー
#    https://v2.tauri.app/plugin/updater/ を参照

# 3) app/tauri.conf.json を編集する
#    plugins.updater.active           = true
#    plugins.updater.endpoints        = ["https://example.com/updates/{{target}}/{{arch}}/{{current_version}}"]
#    plugins.updater.pubkey           = "生成された公開鍵"
```

`bundle.createUpdaterArtifacts: true` を有効にしておかないと、署名済み更新
アーティファクト（`.tar.gz` / `.msi.zip` / `.app.tar.gz`）が生成されない。

## Backup / Restore

`mixd_storage::backup` が実体を実装している。

- ペイロードは `VACUUM INTO` で取った整合性のある SQLite スナップショット
  （稼働中の WAL DB をファイルコピーすると破れたページを取り込むため不可）。
- `tar` → zlib → AES-256-GCM。鍵は Argon2id でパスワードから導出。
- コンテナヘッダは `MIXBAK\x01\x00` + salt(16) + nonce(12) + ciphertext。
- **API key は含めない**。OS Keychain にあり、バックアップに書くと復号鍵が
  ディスクへ平文で出るため。復元後に再入力が必要。
- Restore は復号を検証したうえで `mix-agent.restored.db` に**ステージング**
  する。実行中プロセスは SQLite プールを開いているため、その場でファイルを
  差し替えると破損する危険がある。`Storage::open` が起動時に
  `mix-agent.db.pre-restore` へ退避してから差し替えるので、再起動 1 回で反映される。

## Schedule

`mixd_core::schedule::CronSchedule` が 5 フィールド / 6 フィールド（秒付き）と
`@daily` などのマクロを解釈し、`@daily` 等の Vixie cron の「日-of-month と
day-of-week の両方指定時は OR」 semantics を再現する。

`app/src/scheduler.rs` の daemon が 20 秒ごとに起床し、
`(anchor, now]`（anchor = 最終実行時刻、初回は作成時刻）に落入る発火時刻が
あれば Run を開始する。発火時に `lastRunAt = now` を書くため同じスロットは
二度実行されず、アプリ停止中に取りこぼしたジョブは次回起動時に**ちょうど1回**
追従する（取りこぼし分をまとめて再生しない）。

UI は cron 式を入力中に `preview_cron` で次回5回を表示し、誤記は登録時に拒否する。

## フェーズ

| Phase | 内容 |
| --- | --- |
| 0 | 雛形 / workspace / capabilities / CI |
| 1 | storage + providers + 単発Chat |
| 2 | tools + permission gate + egress |
| 3 | Agent 自律性 + context 圧縮 / retrieval |
| 4 | MCP host + Browser Operator |
| 5 | Memory / Knowledge / Skills / Projects / Schedules / Agents |
| 6 | tray / menu / notification / auto-update / backup / release CI |

## 既知の残タスク

- 署名鍵の生成と `latest.json` 発行基盤（auto-update を有効化する前作業）。
- Knowledge は lexical 検索のみ。embedding 検索は未実装。
- Memory の自動生成（Run 後の抽出）は未実装。手動 CRUD + revision のみ。
- シグナル / PTY ベースの対話 terminal（現在は timeout 付きの実行のみ）。

## ライセンス

**確定: Apache License 2.0**。リポジトリ全体の `LICENSE` が Apache-2.0 であり、
Desktop 側も `desktop/mix-agent-desktop/LICENSE` と `Cargo.toml` の `license` を
含めて Apache-2.0 に統一されている（MIT ではない）。
変更する場合はルート `LICENSE` / `NOTICE` / `desktop/mix-agent-desktop/NOTICE` と
`desktop/mix-agent-desktop/Cargo.toml` の `license` を同時に書き換えること。

## 関連ドキュメント

- [`docker/MIX-agent/docs/architecture.md`](../../docker/MIX-agent/docs/architecture.md)（Docker 版アーキテクチャ）
- [`desktop/README.md`](../../desktop/README.md)（Desktop 版入口・セットアップ / 開発フェーズ）
