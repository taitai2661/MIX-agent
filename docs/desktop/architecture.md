# MIX-agent Desktop — Architecture

> Desktop 版の**現行仕様**をまとめた文書です。設計判断の経緯と代替案は各 `*-plan.md` にあります。
> Docker 版の全体構成は [`../architecture.md`](../architecture.md) を参照してください。

## 目的

Docker 版（`apps/server` + `apps/web`）の中核機能（Provider Adapter / Run Engine / Tool Registry / Run イベントの `kind` 契約 / Capability 3 状態 / Checkpoint / Stagnation / Verification / Tiered Summary / Budget Extension / 暗号化 Secret）を **Tauri 2 + Rust + React 19** のネイティブアプリへ再実装する。Docker 版との **コード共有ゼロ**、仕様書のみで契約を結ぶ。

## ディレクトリ

```
MIX-agent/
├── apps/
│   ├── desktop/                # Desktop 版（Cargo + pnpm workspace）
│   │   ├── app/         Tauri 2 shell + #[tauri::command] surface
│   │   ├── core/        Run Engine / state / events / context
│   │   ├── providers/   OpenAI / Anthropic / Gemini / OpenAI-Compatible / Ollama / LM Studio
│   │   ├── tools/       Tool Registry + executor
│   │   ├── browser/     Browser Operator + chromiumoxide backend
│   │   ├── mcp/         MCP client host
│   │   ├── storage/     SQLite + Secret store
│   │   ├── scripts/     pin-chromium.mjs
│   │   └── ui/          React 19 + Vite + Tailwind (Desktop独自UI)
│   ├── server/                 # Docker 版バックエンド (FastAPI + Python)
│   └── web/                    # Docker 版フロントエンド (React 19 + Vite)
├── docs/                       # ドキュメント (desktop/ 含む)
├── scripts/                    # パリティチェック、アイコン生成など
└── compose.yaml                # Docker Compose 設定
```

### Run Engine / Terminal のモジュール分割

`core/src/runs/engine.rs` は型定義と起動・駆動・スケジューラだけを持ち、実装は
`core/src/runs/engine/` 配下へ分割されています。

| ファイル | 担当 |
| :--- | :--- |
| `engine/event_bus.rs` | イベントのバッファリング、シーケンス、DB 永続化 |
| `engine/query.rs` | Run 一覧・状態、履歴、スナップショット、steering |
| `engine/execute.rs` | 1 Run のメインループ |
| `engine/execute_turn.rs` / `execute_tools.rs` | Provider ターン、Tool 実行と承認 |
| `engine/execute_budget.rs` / `finish.rs` | 予算延長、終了処理 |
| `engine/helpers.rs` | 履歴変換、要約、並列可否などの純粋関数 |

`tools/src/terminal.rs` も `terminal/{shell,session,parsing,sandbox,tests}.rs` に
分割し、公開 API（`TerminalTool` と `truncate_to_char_boundary`）は変えていません。

## 信頼境界

```
┌──────────────────────────────────────────────────────────────┐
│ Tauri App (tokio runtime 上の一つのバイナリ)                 │
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
│                                                              │
│  └─ 子プロセスとして実 Chromium を起動（Browser Operator）    │
└──────────────────────────────────────────────────────────────┘
```

DB は同一プロセスの tokio runtime 上で in-proc に開かれます（Docker 版の
PostgreSQL コンテナに相当する分離はありません）。ただし Browser Operator は
実 Chromium を**子プロセスとして**起動するため、単一の OS プロセスではありません。

WebView からは `fs` / `shell` / `http` プラグインの権限を `app/capabilities/default.toml`
で一切付与していません（明示的な deny ではなく不許可）。すべての操作が
`#[tauri::command]` 経由となり、Rust が信頼の単一源泉です。

## Docker 版との整合点

| Docker 版 | Desktop 版 | 共有方針 |
| --- | --- | --- |
| `routes.py` の URL 契約 | `#[tauri::command]` 引数・戻り JSON | 形の対応関係を維持（対応表は [`api-mapping.md`](api-mapping.md)） |
| Run イベントの `kind` 値一覧 | `mixd_core::runs::engine::RunEvent` | 共通分は同じ `kind` 名。差分は下述 |
| Capability 3 状態 | `mixd_core::routing::CapabilityState` | 仕様踏襲 |
| Checkpoint / Stagnation / Verification | `core::runs::{checkpoint_store,stagnation,verification}` | 仕様踏襲 |
| Tiered Summary | `core::context` | 仕様踏襲 |
| 暗号化 Secret (AES-GCM + AAD) | `storage::secrets` | 仕様踏襲 |
| pg_trgm / JSONB Index | SQLite `json_extract` + FTS5（メッセージ・Knowledge・Memory） | 再実装 |
| マルチユーザー / 管理者作成 | 単一ユーザー前提 | 削除 |
| Docker container 群 | tokio runtime 上の in-proc タスク | 置換 |

### Run イベントの `kind` 差分

`app/src/run_events.rs` は Docker 版が SSE で流すのと同じ JSON 形状を、
Tauri の `run_event` というイベント名で emit します。トークン差分（`text` /
`reasoning`）は 50ms または 64 件でまとめて `run_event_batch` として送り、
マイルストーン（`status` / `tool_*` など）は遅延なく単独で送ります。UI は
両方を同じ `applyRunEvent` に流します。
ただし `kind` の一覧は **完全一致ではありません**。

共通（両エディションが使う）: `text` / `reasoning` / `message` / `status` / `error` /
`model_started` / `model_selected` / `tool_started` / `tool_result` /
`context_summary` / `checkpoint_saved` / `budget_extension_requested` /
`budget_extension_resolved` / `stagnation_detected` / `verification_required`

Desktop のみ: `tool_denied` / `tool_approval_requested` / `tool_approval_resolved` /
`destructive_tool_called` / `verification_failed` / `todo_list` / `awaiting_approval`

Docker のみ: `model_rerouted` / `plan` / `approval` / `browser_frame` / `phase_advanced`

UI 側の解釈は `ui/src/lib/runEvents.ts` の `applyRunEvent()` が持ちます。
`default` 分岐で未知の `kind` は黙って無視されるので、Desktop 側で
`kind` を増やすときは同じ switch にケースを追加してください。

## Local runtime management

`app/src/local_runtime.rs` が Ollama / LM Studio などの外部ランタイムの
ライフサイクルを所有します。検出（`mixd_providers::discovery`）は「何が
存在するか」だけを返し、状態・プロセス・ログ・ジョブはマネージャが保持します。

- **所有権**: アプリが起動した子プロセスだけを停止対象にします。外部起動の
  ランタイムは `managed = false` とし、停止ボタンを出しません。ポート番号から
  PID を推測して kill することはありません。起動・停止・再起動は slug ごとの
  ロックで直列化し、二重起動や「停止したのに子プロセスが残る」競合を防ぎます。
- **起動**: `launch_spec()` が返す絶対パス + 引数配列で `tokio::process`
  から起動します。シェルは経由しません。Ollama は `ollama serve`、LM Studio は
  `lms server start --port …` です。ポートと `OLLAMA_HOST` はプロバイダーの
  `baseUrl` に合わせて書き換えるため、カスタムポートでも二重起動しません。
- **状態**: `unknown / not_installed / stopped / starting / online / degraded /
  stopping / error / unsupported`。10 秒間隔のヘルスチェックで 2 回連続失敗
  すると `online → degraded` になり、停止中でもインストール済みのランタイムは
  30 秒ごとに再検出します。
- **モデル操作**: Ollama は `/api/pull`・`/api/delete`・`/api/generate`
  (`keep_alive`)、LM Studio は `lms get/load/unload`。進捗は `job_id` +
  `local_job_event` で流します。ログはスナップショットに同梱せず、
  `local_runtime_logs` をパネル表示中だけ取得します。
- **チャット**: Ollama は `providers/src/ollama.rs` のネイティブ `/api/chat`
  を使い、`options.num_ctx` と `keep_alive` を渡します。`num_ctx` はモデルの
  公称最大値ではなく `local_context_window`（既定 8K）で上限を切ります。
  thinking 非対応モデルでは `think` を落として 1 回だけ再試行します。
  LM Studio は OpenAI 互換のままです。
- **設定**: `local_scan_on_start` / `local_auto_start`（既定オフ）/
  `local_stop_on_exit` / `local_keep_alive` / `local_context_window`。
  プロバイダー単位の `data.local.autoStart` が優先されます。
- **ローカルの二段階判定**: `is_local_base_url` は API キー免除・Auto の
  コスト判定用で、ループバックに加えて LAN / mDNS / Tailscale も含みます。
  一方、起動・停止・自動起動・`num_ctx` 上限は
  `is_loopback_base_url`（`127.0.0.1` / `localhost` / `::1` のみ）で判定します。
  LAN 上の Ollama はローカルではあるが、このアプリが所有するプロセスでは
  ありません。両者を混ぜると LAN の GPU サーバためにローカルで二重起動し、
  8K 上限を適用してしまいます。
- **空ライブラリ**: ランタイムの指紋が一致したサーバは、モデルが 0 件でも
  `online` として扱います。`online` は「モデルがある」ではなく「このアプリが
  話せるランタイムである」の意味です。そうしないと、まっさらな Ollama を
  起動 → 最初のモデルを pull する導線が成立しません。

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
   │  navigate / snapshot+ref / click / type / press /
   │  hover / select / check / scroll / dom / extract /
   │  screenshot / evaluate / wait_for / tabs / history /
   │  click_at / type_active
   ▼
   Backend: enum BrowserBackend  ← 差し替え可能
          Chromium          = 管理対象の Chromium を起動
          SystemChromium    = システム Chromium にフォールバック
          Disabled          = 実行ファイルが見つからない
   │
   ▼
   Managed Chromium (<app-data>/chromium) or system Chromium
```

`mixd-core` は `chromiumoxide` に依存しません。backend の差し替えだけで
Browser 実装の差し替えが可能で、`BrowserBackend` は 3 状態
（`Chromium` / `SystemChromium` / `Disabled`）を持ちます。

Chromium は **常に new headless** で起動します。以前は release だけ
`with_head()` で可視ウィンドウを出していましたが、アプリ内のサイドパネルと
二重表示になり、余分なコンポジタも残るため廃止しました。表示は
チャット右側のブラウザパネル（スプリット / フルビュー）だけです。

サイドパネルは Codex 風の構成です。

- 上部ツールバー: 戻る / 進む / 再読み込み・停止 / アドレスバー /
  セキュリティ表示 / システムブラウザで開く
- ブラウザタブ: 既存の `tabs` / `new_tab` / `switch_tab` / `close_tab` を UI 化
- ライブ画面: CDP screencast（`Page.startScreencast`）を Tauri イベント
  `browser://frame` で受信し、失敗時は 2 秒ポーリングへフォールバック
- `⌘⇧B`（macOS）/ `Ctrl+Shift+B`（Windows / Linux）で表示/非表示、
  `⌘L` / `Ctrl+L` アドレスバー、`⌘T` / `Ctrl+T` 新規タブ、
  `⌘R` / `Ctrl+R` 再読み込み
- エージェントの `browser` ツール開始時に自動表示（`browserAutoOpen`、既定 ON）。
  エージェント起因のオープンはフォーカスを奪いません。

Chromium のメモリ対策として、同時に開くワークスペースは 2 個まで（LRU で
退避）、5 分間ブラウザ呼び出しとライブ取得が無ければプロセスを閉じます。
パネルを開いている間は 60 秒ごとの軽量 keepalive が LRU を更新するため、
ライブ表示を一時停止していても閉じられません。プロファイルはディスクに残るため、
次回オープン時に Cookie / ログインは復元されます。上限は
`MIX_BROWSER_MAX_OPEN`（既定 2）と `MIX_BROWSER_IDLE_SECS`（既定 900 秒、
30 秒未満は無視）で調整できます。

アクション語彙は `packages/tool_contracts/browser.json` が正です。Desktop は
`browser` 1 ツール、Docker は `browser_*` 15 ツールをこの語彙へマップし、
未対応アクションは `unsupported` として明示します。`scripts/check-browser-parity.py`
が CI で両版のズレを検出します。

`browser_allow_local`（既定 OFF）を ON にした場合だけ loopback / RFC1918 /
ULA へ到達できます。169.254.0.0/16（cloud metadata）と link-local は ON でも
拒否します。IPv4-mapped IPv6（`[::ffff:169.254.169.254]`）も同じ v4 判定に
落として拒否します。許可リストが設定されている場合、この opt-in は
loopback / private 宛先だけを免除し、公開ホストには引き続き許可リストが必要です。
`update_settings` は許可リスト変更時に `refresh_registry` を呼ぶため、再起動なしで
エージェントと手動パネルの両方へ反映されます。

ナビゲーション時の事前チェックに加えて、各ページターゲットには CDP の
`Fetch` リクエスト傍受を常時インストールします（`ensure_egress_guard`）。
ページ内の `fetch()` やサブリソース、DNS 再解決後のリダイレクトも含めて
ホスト名を解決し、非 opt-in 時の private / loopback、常時 link-local・
cloud metadata を `BlockedByClient` で拒否します。`Network.setBlockedURLs`
で metadata リテラルも二重に塞ぎます。これにより `evaluate` の文字列フィルタを
迂回されてもネットワーク層で止まります（DNS 解決と実際の接続の間の TOCTOU は
残るため、OS sandbox の代替ではありません）。

ナビゲーションは `http` / `https` と `about:blank` だけを許可します。
`file:` / `data:` / `javascript:` / `chrome:` は `navigate` と `new_tab` の両方で
拒否し、`new_tab` を抜け道にしないよう Registry 側でも同じゲートを通します。
`click_at` と `type_active` は手動操作用として契約に `manual_only` で登録し、
モデルへ渡す `browser` tool の enum からは除外しています。CDP のリクエスト単位の
強制はまだ未実装で、ページ内 JS 自体は引き続き自由にネットワークへ出られます。

## ChatGPT（Codex 使用枠）— 非公式・自己責任

> **警告**: OpenAI の公式 API ではなく、Codex CLI が利用する非公開バックエンドを
> 呼び出す **非公式機能** です。利用規約・アカウント制限に抵触する可能性、予告なき
> 停止のリスクがあります。利用は **自己責任** です。設定 ▸ **ChatGPT** の専用タブで
> 有効化・ログインします。プロバイダーのプリセットには表示されず、汎用の Provider
> 作成フォームからは行を作れません（`chatgpt_ensure` だけが専用行を作ります）。
> 通常の API キー Provider はそのまま利用できます。

- 接続は1つだけ。専用タブが `chatgpt_status` で行を読み、未作成なら
  `chatgpt_ensure` が固定 URL・固定ラベルの行を作る。汎用 `create_provider` は
  `chatgpt` を拒否する。
- 認証は **デバイスコード方式**。`chatgpt_device_start` → `chatgpt_device_poll` で
  トークンを取得し、`provider/<id>` / `oauth`（暗号化 Secret）にだけ保存する。
  ポーリング間隔はサーバー側でも `interval` を強制する。
- トークン更新は provider 行ごとのプロセス全体ロックで single-flight にし、
  ロック取得後に Secret を再読込する。複数の adapter（実行・モデル取得・残量取得）が
  同じ refresh token を同時に使ってローテーション負けするのを防ぐ。
- 推論は `providers/src/chatgpt.rs` の Responses ストリーム。
  `chatgpt-account-id` / `originator` を付け、`store: false`、
  `reasoning.encrypted_content` を要求する。ツールターンでは `Message.native` に
  保持した reasoning / function_call を次リクエストへ戻す。
- 残量は `GET https://chatgpt.com/backend-api/wham/usage`。推論レスポンスの
  `x-codex-*` ヘッダと `codex.rate_limits` SSE でもキャッシュを更新する（60 秒 TTL）。
- サブスク枠の Run は `Provider::billing() == Subscription` として $0 で記録する。
- ログアウトは `/oauth/revoke` を呼び、成功可否に関わらずローカル Secret を削除する。
- 進行中のログイン状態はプロセス内のみ（15 分）。アプリ再起動でやり直しになる。
- `originator` は既定 `mix_agent`。必要な場合のみ `MIX_CODEX_ORIGINATOR` で上書き
  する（Codex CLI の値を指定するとクライアントを偽装することになる）。
- `Message.native` は `HistoryMessage.native` として会話履歴にも保存し、Run 再開時も
  reasoning を含むツールターンをそのままリプレイする。1 ターン 256 KiB を超えた場合
  だけ保存を諦め、テキスト + function_call の再構成へフォールバックする。
- 推論リクエストは `Accept: text/event-stream` を明示し、一時的な 5xx / 接続断 /
  クォータ未達の 429 はバックオフ付きで再試行する。

## macOS Spotlight 索引

macOS の **Core Spotlight**（`CSSearchableIndex`）へ、会話・プロジェクト・
ナレッジを登録する。実装は `app/src/spotlight/` に閉じ、他プラットフォームでは
no-op になる。

- **対象**: `conversation:<id>` / `project:<id>` / `knowledge:<docId>`。
  content type は `public.text`。タイトル、短い説明、キーワード、更新日時を
  登録し、本文索引を有効にした場合だけ本文も `textContent` に入れる。
- **既定**: `spotlight_enabled = false`、`spotlight_index_body = false`。
  システム全体の検索結果に出るため opt-in。無効化すると
  `deleteAllSearchableItems` でこのアプリの索引を削除する。
- **キュー**: `spotlight_items`（`item_id` / `domain` / `source_id` /
  `content_hash` / `state`）と `spotlight_meta`。起動 5 秒後に差分
  reconcile し、変更系コマンドと Run の終了時は 2 秒デバウンスで
  バッチ索引する。失敗は `last_error` に残し、次の pass で再試行する。
- **クリック**: tao の `NSApplicationDelegate` は browsing-web 以外の
  `NSUserActivity` を `false` で落とすため、
  `application:continueUserActivity:restorationHandler:` と
  `application:willContinueUserActivityWithType:` の IMP を差し替える。
  `CSSearchableItemActivityIdentifier` を `conversation:` などの接頭辞で
  解釈し、`spotlight://open` を UI へ emit する。コールドスタート用に
  最後の target を pending スロットへ残し、UI は
  `spotlight_take_pending` で drain する。
- **開発時の制約**: `tauri dev` の非バンドル実行には
  `CFBundleIdentifier` がなく、Core Spotlight に登録できない。設定画面は
  「この環境では利用できません」と表示する。動作確認は `tauri build` で
  生成した `.app` を `/Applications` に置いて行う。手動スモークは
  `cargo build -p mix-agent-desktop --example spotlight_smoke` で作った
  バイナリを最小 `.app` に包んで実行する。
- **設定**: macOS のみ「設定 ▸ Spotlight」タブ。有効化 / 本文索引 /
  索引済み・未処理件数 / 最終同期 / 最終エラー / 再構築 / 索引から削除。
- **設計経緯**: [`spotlight-plan.md`](spotlight-plan.md)。

## セキュリティ

- **Secret store**: AES-256-GCM の暗号文は `secrets.enc.json`（0600）に保存します。macOS は **マスター seed をデータディレクトリ内の `.master_seed`（0600）** に置きます。Desktop 版は ad-hoc 署名で配布しており、ログイン Keychain は再ビルドごとに別アプリと認識してパスワードダイアログを出すため、通常起動では Keychain を読みません。旧版で Keychain に seed がある場合は初回だけ読み取り、`.master_seed` へコピーして以後はアクセスしません（旧ファイル seed は `.master_seed_backup` として残し、復号のフォールバックに使います）。Windows は従来どおり **Credential Manager にマスター seed** を置きます（同一ユーザーではプロンプトが出ないため）。Linux は Secret Service が常にあるとは限らないためファイル方式です。`MIX_SECRET_BACKEND=file|keychain|keychain-seed` で明示的に切り替えられます。
- **DB 暗号化設定 (Codex 互換)**: 標準は **OFF（平文 SQLite）** で動作し、所有者のみ読み書き可（0600）のアクセス権で保護されます（Codex parity）。設定（GeneralSettings）から保存時の暗号化を **ON / OFF** 切り替え可能で、ON に設定された場合は OS Keychain にマスター暗号化鍵（`db.master`）が生成・保管されます。診断画面（Diagnostics）でも暗号化状態が確認できます。
- **Egress allowlist + private IP deny**: `tools::network` が reqwest 実行前に検査。allowlist が空なら公開ホストのみ許可し、private IP は常に拒否します。保存済みルールは `*.example.com` のようなワイルドカードに対応します。
- **WebView CSP**: `app/tauri.conf.json` の `app.security.csp` で `connect-src 'self' ipc: http://ipc.localhost` に限定。
- **window 設定**: `app.macosPrivateApi = true` は `titleBarStyle: Overlay` +
  `hiddenTitle` + `trafficLightPosition` が macOS の private API を使うための要件です
  （無効化するとカスタムタイトルバーが正しく描画されません）。`Cargo.toml` の
  `tauri` 依存の `macos-private-api` feature と対になっています。
  Windows / Linux はネイティブのタイトルバーを無効化し（`app/tauri.windows.conf.json`
  と `app/tauri.linux.conf.json` の `decorations: false`）、`WindowControls.tsx` の
  カスタムタイトルバーだけを表示します。ネイティブ装飾を残すと OS のボタンと
  アプリ内のボタンが二重に表示されます。プラットフォーム別設定は配列ごと
  置き換わるため、ウィンドウ設定はすべての項目を両ファイルにも記載します。
- **OS プロセス sandbox**: 設定 `sandbox_terminal` で、次に起動するシェルを OS 機能で隔離します。
  - **macOS**: `sandbox-exec` でワークスペース + 一時ディレクトリ + ビルドキャッシュ以外への書き込みを拒否。`sandbox_terminal_network` で `(deny network*)` も付与。
  - **Linux**: `bubblewrap`（なければ `firejail`）でルートを読み取り専用にし、ワークスペースとビルドキャッシュだけを書き込み可能にします。ネットワーク遮断は `--unshare-net`。
  - **Windows**: `Job Object` でシェルと子プロセスを束ね、プロセス数上限・アプリ終了時の一括停止を強制します。portable-pty がプロセス作成属性を公開しないため、**ファイルシステムの書き込み隔離は未対応**です（UI にも明記）。
  - 利用できない環境（Linux で bwrap/firejail 未導入など）では設定を保存できず、理由を表示します。`browser` も外部 Chromium を操作できるため、敵対的なコードを安全に実行する用途には使わないでください。

## Managed Chromium（初回インストール方式）

同梱をやめた理由・代替案の比較・移行方針は
[`chromium-on-demand-plan.md`](chromium-on-demand-plan.md) に記録している。ここでは現行の仕様だけを扱う。

Desktop 版は Chromium をアプリバンドルに含めない。ブラウザ機能を初めて使う
ときに、アプリ内のインストーラーが Chrome for Testing を
`<app-data>/chromium/` へダウンロードする。配布物（DMG / NSIS / MSI /
AppImage / deb）にはブラウザ本体が入らない。macOS arm64 のローカルビルドでは
`.app` が 35.58 MiB（アップデータ用 `.app.tar.gz` は 12.72 MiB）になった。

### インストール先と解決順

```
<app-data>/
├── browser/                     # ワークスペースプロファイル
├── artifacts/                   # スクリーンショット
├── chromium/                    # 管理対象 Chromium
│   ├── chrome-mac/…             # macOS（CfT の .app を保持）
│   ├── chrome-win/…             # Windows
│   ├── chrome-linux/…           # Linux
│   └── chromium-version.json
└── chromium-install-state.json  # 失敗・キャンセル状態（進捗は保存しない）
```

探索は `mixd_browser::profile` が行い、`BrowserOperator::resolve()` は次の順で
最初に見つかったものを使う。

1. `MIX_CHROMIUM_HOME`（開発・検証用の明示パス）
2. `<app-data>/chromium`（管理対象）
3. 旧バンドルの `resources/chromium`（移行期間の互換）
4. システムの Chromium / Chrome / Edge
5. 無ければ `disabled`

`BrowserStatus.source` が `installed` / `bundled` / `system` / `override` /
`none` を返すので、UI は管理対象だけに更新・削除を出す。

### バージョン固定と取得

`browser/src/chromium-manifest.json` に、サポートする CfT プラットフォーム
（`mac-arm64` / `mac-x64` / `win64` / `linux64`）の URL・SHA-256・サイズを
固定する。マニフェストは `scripts/pin-chromium.mjs` で生成し、アプリの
バイナリに埋め込む。実行時に CfT の latest を解決して無検証で入れることは
しない。

```bash
pnpm chromium:pin                                  # 最新 Stable を固定
node scripts/pin-chromium.mjs --version 156.0.8078.73
```

インストーラー（`browser/src/installer.rs`）は次を行う。

1. 空き容量を確認（zip + 展開後 + 余裕）
2. HTTPS でストリーム受信し、`storage.googleapis.com` 以外へのリダイレクトを拒否
3. SHA-256 とサイズを検証
4. `chromium.staging-*` へ zip を展開（パストラバーサル・危険な symlink を拒否）
5. `chrome-mac-arm64` などを `chrome-mac` へ正規化（`.app` 内の名前は変えない）
6. `--version` を 30 秒以内に実行して起動可能性を確認
7. 旧 `chromium` を退避してから staging を rename し、最後に旧ツリーを削除

失敗時は既存のインストールを残す。アプリ終了で中断した staging / download は
次回起動時に掃除する。UI は `browser_install` / `browser_install_cancel` /
`browser_uninstall` / `browser_status` を使い、進捗は
`browser://install` イベントと 1 秒ポーリングの両方で受ける。

### 署名と供給網

Chrome for Testing の `.app` は ad-hoc / linker-signed で、上流の zip でも
`codesign --verify --deep --strict` は成功しない。そのため署名検証を必須には
せず、**アプリに埋め込んだ SHA-256** を供給網のアンカーにする。展開後は
`--version` の起動確認だけを行う。旧 `fetch-chromium.mjs` のように `.app` 内の
実行ファイル名を `Chromium` にリネームすると Mach-O 署名が壊れるため、CfT の
名前（`Google Chrome for Testing.app`）をそのまま保持する。

### ライブ表示と成果物

- ライブ表示は CDP screencast（JPEG quality 60、everyNthFrame 2）を基本にし、
  失敗時は 2 秒ごとの非永続 JPEG フレーム取得へフォールバックする。ウィンドウが
  非表示の間は停止する。エージェント操作と同じ workspace ロックは `try_lock`
  で取り、操作中はフレームを諦めて前回の画像を残す（スクリーンショット待ちで
  クリックを遅らせない）。
- ライブフレームは CSS ビューポートの 0.5 倍でキャプチャする。パネル表示幅は
  380–560px なので十分な解像度を保ちつつ、JPEG エンコード・IPC・WebView の
  デコードを約 1/4 に抑える。返す `width` / `height` は CSS サイズで、クリック
  座標変換は解像度に依存しない。成果物の PNG はフル解像度のまま。
- `browser_frame` は現在 URL・タイトル・戻る/進む可否も返し、ツールバーが
  追加のキャプチャ無しで状態を更新できる。
- ライブ画面のクリックは `object-contain` の letterbox を考慮し、描画矩形の
  中央・拡大率からページ座標へ変換する。描画矩形の外側はクリックしない。
- スクリーンショットのインデックスは 500 件で上限。退避したエントリの PNG は
  実ファイルごと削除し、起動時には実体の無いインデックス行を落とす。

### 移行と開発

- 既存の同梱版からアップデートした場合、初回にブラウザの再ダウンロードが必要。
  UI は未インストール状態を表示し、システムブラウザがあればそれをフォールバックに使う。
- `pnpm tauri dev` はリソースを持たないため、システム Chromium か
  `MIX_CHROMIUM_HOME=/path/to/chromium` を使う。
- オフライン環境では `<app-data>/chromium` へ手動で展開するか、システム
  Chromium を使う。手順はリリースノートに記載する。
- リリース CI はもう Chromium を取得・同梱しない。macOS の署名検査はアプリ本体
  だけで行い、ブラウザの CPU アーキテクチャはマニフェストと起動確認で担保する。

**Desktop版のmacOSアプリはApple Developer IDコード署名・公証を行いません。** ただし `tauri.conf.json` の `bundle.macOS.signingIdentity` に `-` を設定し、ad-hoc署名は必ず付与します。Apple Siliconでは署名のないアプリバンドルを起動できず、ダウンロードしたDMGからコピーしただけで「壊れているため開けません」と表示されるためです。ad-hoc署名でも公証チケットはないため、初回起動時にGatekeeperの「このまま開く」確認が必要です。Windowsインストーラーは未署名です。
ビルドにApple証明書・AppleアカウントSecrets・Windows署名証明書を渡さず、macOS GatekeeperとWindows SmartScreenの警告や起動制限の可能性をRelease notesに明記します。リリースワークフローはmacOSバンドルに対して `codesign --verify --deep --strict` とCPUアーキテクチャ検査を実行します。

## 更新方法（署名付き自動更新）

Tauri updater plugin を有効にし、`bundle.createUpdaterArtifacts = true` でプラットフォームごとの署名済みアップデータを生成します。

- macOS: `.app.tar.gz` + `.sig`
- Windows: NSIS `-setup.exe` + `.sig`（v2 はインストーラー自体をアップデータとして再利用）
- Linux: `AppImage` + `.sig`（`.deb` はアプリ内更新の対象外。手動更新）

`scripts/build-latest-json.mjs` がステージ済みアセットと `.sig` から `latest.json` を生成し、Releaseへ添付します。アプリは `https://github.com/taitai2661/MIX-agent/releases/latest/download/latest.json` を参照します。

署名秘密鍵は GitHub Actions の Repository Secrets `TAURI_SIGNING_PRIVATE_KEY` / `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` にのみ置きます。公開鍵は `tauri.conf.json` の `plugins.updater.pubkey` に埋め込みます。秘密鍵を失うと既存インストールへ更新を配信できなくなるため、リポジトリ外にオフラインバックアップを保管してください。

アプリ内の更新は Rust の `commands::update`（`app_version` / `check_for_update` / `download_update` / `pending_update`）を経由します。WebView には `updater:*` 権限を渡しません。起動時チェックは設定 `auto_check_updates`（既定 `true`）、自動ダウンロードは `auto_download_updates`（既定 `true`）で制御します。ダウンロードしたペイロードは `<app-data>/updates` に署名検証済みで保存し、ユーザーがアプリを通常終了したときにインストールします。再起動は自動では行わず、次回起動が新バージョンになります。Linux の deb はアプリ内更新の対象外です。

macOS は引き続き Developer ID 署名・公証なし（ad-hoc 署名のみ）、Windows インストーラーは未署名です。これはアップデータ署名とは独立で、Gatekeeper / SmartScreen の警告は残ります。配布物の `SHA256SUMS` も従来どおり添付します。

鍵の生成・Secrets 登録・ローテーション手順は [`update-signing.md`](update-signing.md) を参照してください。

## Backup / Restore

`mixd_storage::backup` が実体を実装している。

- ペイロードは `VACUUM INTO` で取った整合性のある SQLite スナップショット
  （稼働中の WAL DB をファイルコピーすると破れたページを取り込むため不可）。
- `tar` → zlib → AES-256-GCM。鍵は Argon2id でパスワードから導出。
- コンテナヘッダは `MIXBAK\x01\x00` + salt(16) + nonce(12) + ciphertext。
- **API key は含めない**。秘密ストア（`.master_seed` / OS 安全ストレージ）に
  あり、バックアップに書くと復号鍵がディスクへ平文で出るため。復元後に再入力が必要。
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

| Phase | 内容 | 状態 |
| --- | --- | --- |
| 0 | 雛形 / workspace / capabilities / CI | ✅ |
| 1 | storage + providers + 単発Chat | ✅ |
| 2 | tools + permission gate + egress | ✅ |
| 3 | Agent 自律性 + context 圧縮 / retrieval | ✅ |
| 4 | MCP host + Browser Operator | ✅ |
| 5 | Memory / Knowledge / Skills / Projects / Schedules / Agents | ✅ |
| 6 | tray / menu / notification / backup / 署名付き自動更新 + macOS・Windows・Linux リリースCI | ✅ |
| 7 | macOS 完成 + Windows x64 + Linux 配布 | ✅（v0.5.1 で Windows x64 追加、v0.5.2 で Linux 追加・アプリ内更新） |
| 8 | コーディングエージェント化（編集・検証・探索・git・UI 証跡） | ✅（8.1〜8.4。詳細は [`coding-agent-plan.md`](coding-agent-plan.md)） |
| 9 | 洗練 & パリティ（Server ツール parity / `@` 補完 / モノレポ整備） | 未着手（着手時点の計画は [`coding-agent-plan.md`](coding-agent-plan.md)） |
| 10 | コーディングエージェントの信頼性（ツール契約・評価・計画承認・ロールバック） | 10.0 完了。10.1 以降は計画（詳細は [`coding-agent-plan.md`](coding-agent-plan.md)） |

Phase 6 で Tauri updater を再有効化しました。`desktop-release.yml` は Apple Silicon／Intel macOS の ad-hoc 署名済み DMG、Windows x64 の未署名 NSIS/MSI、Linux x64 の AppImage/deb に加え、署名済みアップデータ（`.app.tar.gz` / `-setup.exe` / `.AppImage` と各 `.sig`）と `latest.json` を Release へ添付します。秘密鍵は GitHub Actions の Repository Secrets のみに置きます。
Phase 7 は v0.5.1 / v0.5.2 で配布まで到達しました。各 OS 実機での更新適用確認は [`verification.md`](../verification.md) に記録します。

## 空チャットの挨拶（2026-10-01）

スレッドが空のときの挨拶は、`ui/src/features/chat/greeting/` が Signals から文を
合成して表示します。入力は 4 系統です。

| 系統 | 内容 |
| --- | --- |
| 日時 | 時間帯 8 区分・曜日 5 種・季節と季内位置・年内の日数 |
| 暦 | 日本の祝日（固定 14 種・ハッピーマンデー・振替休日・国民の休日）、二十四節気 24 種（太陽視黄経から日本標準時 0 時単位で算出）、季節の行事 16 種 |
| ワークスペース | スレッド数・メッセージ数・プロジェクト数・最終更新からの経過・サーフェス（`chat` / `desktop-agent`）・プロジェクト選択の有無 |
| ブラウザ履歴 | `localStorage`（`mix-agent-welcome`）の訪問間隔・初訪問時刻・直近 24 件の部品 ID |

見出しは「挨拶＋状況節＋締め」の 3 層を独立抽選し、文体タグ
（`polite` / `calm` / `casual`）と文形タグ（`question` / `declarative`）で互換判定
してから連結します。文形タグは文末記号から導いており、`sentence()` ヘルパーに
直接書けないためここが単一の情報源です。日本語は間なしで、英語は空白区切りで連結し、
`t()` は文単位で適用します。言語辞書の件数は `pnpm ui:test`（`i18n.keys.test.ts` の孤立キー検査）の出力で確認してください。

Docker 版（[`docs/verification.md`](../verification.md) の 2026-10-01 節）と同じ方式ですが、
冒頭の方針どおりコードは共有していません。Desktop 版は
`スレッド` / プロジェクト / 差分 / Browser など自分の語彙で書き下し、提案も
Desktop が実際に持つ機能（差分・テスト・ブラウザ・プロジェクト）に寄せています。
既存のクイックプロンプト 4 件は提案プールに吸収したので `i18n.keys.test.ts` の
孤立キーは増えていません。

### 確認した項目

- UI の vitest（`pnpm ui:test` で件数を確認）が成功。うち greeting 配下が 7 ファイル。
- 二十四節気 24 種が、国立天文台「暦要項 令和 8 年 (2026)」の中央標準時表記と一致。
  分点瞬間は既知値（1990 / 2000 / 2026 年の春分）と 3〜8 分の差。
- 祝日 16 件（固定祝日・ハッピーマンデー・2026-05-06 の振替休日・2026-09-22 の
  国民の休日）を固定日付でアサート。
- 2 年分の 6570 通り（730 日 × 9 時間帯）の走査で、見出し 2205 通りと
  疑問符の連続・末尾記号・空白混入・英語訳の欠落がないことを確認。
  `desktop-agent` サーフェスでも同じ走査を別に実施しています。
- `pnpm ui:lint`（`--max-warnings 0`）と `pnpm ui:build` が成功。
  バンドルサイズは `pnpm ui:build` の出力で確認してください（500 kB 超の警告は
  変更前から出ています）。

### 未確認の項目

- 実機（Tauri / WebView）での画面表示、スクリーンリーダー、macOS 以外の OS での確認。
- 提案チップのクリックで送信欄が埋まる動作の実機確認。
- 英語表示での文の連結の品質確認。英訳は文単位へ対応させており、ネイティブ話者による
  レビューは未実施です。
- Rust 側のテスト（`cargo test` / `clippy` / `fmt`）は UI のみの変更のため
  実行していません。
- `localStorage` を保存できないプライベートモードでの実挙動は未確認です。

## 自律性・記憶・ツール実行の強化（2026-10-05）

Phase 3〜5 の実装に対する、コード監査で確定した欠陥の修正。

- **検証ゲートをターゲット単位化**（`core/src/runs/verification.rs`）。台帳のキーをツール名から
  効果対象（パス／コマンド）に変更し、`a.rs` を読んでも `b.rs` の未確認書き込みは解除されない。
  登録は dispatch **前**に行い、同一ターン内の `write` → `read` で正しく閉じるようにした。
  `unverified_repair_turns` は台帳が空になった時点でリセットされ、複数ファイル作業でも
  2 回で打ち切られない。破壊的コマンド判定もトークン／リダイレクトベースに置き換え、
  `"rm "` が `"confirm "` に一致する誤検知を解消し、`find -delete` / `git restore` などを追加。
- **web ツールの重複検知**（`core/src/runs/stagnation.rs`）。`read_key` が `query` / `url` を
  見るようにし、無関係な 2 回目の検索が ReadLoop と誤判定されない。
- **todo の per-run 化**（`tools/src/todo.rs`）。`TodoStore` を `run_id` でキー化し、サブエージェントや
  スケジュール実行が親のチェックリストを上書きしない。
- **resume の状態保持**（`app/src/commands/runs.rs`, `core/src/runs/{engine,checkpoint_store}.rs`）。
  Run に `workspaceId` を保存して再開時に再利用し、checkpoint に `tool_calls` を載せて
  予算カウンタを引き継ぐ。
- **キャンセル伝播**。ツール dispatch・provider ストリームを `cancelled_signal` と race させ、
  途中キャンセルを `Cancelled` として終了。打ち切り時は残りの tool call に結果を補い、
  transcript に dangling な `tool_calls` を残さない。`terminal` の `max_wall_clock_ms` は
  10 分に上限。
- **検索の改善**（`core/src/retrieval.rs`）。CJK を unigram/bigram に分割、Memory は関連度 0 を除外し
  `active`/`candidate`/`established` のみ対象、取得した Memory を `reinforce`、Knowledge は
  文書あたり 2 チャンク、出力は項目 1200 字・全体 8000 字で打ち切り、「データであって指示ではない」と明示。
- **memory ツール**（`tools/src/memory.rs`, `app/src/lib.rs`）。エージェントが
  `recall` / `remember` / `forget` / `search_knowledge` を実行できる（`MemoryDispatch` 経由）。
- **revision 復元の所有チェック**（`core/src/memory.rs`）。
- **ツール実行の安全策**（`tools/src/network.rs`, `registry.rs`, `files.rs`）。
  HTTP 本文は 2 MiB で打ち切り、クロスオリジンリダイレクトで `Authorization`/`Cookie` を除去、
  web_search の件数と snippet を制限、browser の `navigate` に egress を適用、
  `files.search` の 1 行を 500 字に制限。

### 今回も未実装のもの

- **Knowledge の embedding 検索**。Knowledge は FTS5（trigram）による lexical 検索に対応したが、ベクトル列と embedding 検索は未整備（Memory 側の hybrid 検索は実装済み）。
- PTY の対話 terminal は `terminal` の `action: "send"`（実行中シェルへの stdin 送出 + 新規出力の取得）に対応。`send` は待ち時間だけ出力を集める方式で、専用の対話 UI は未実装。
- `browser` の `evaluate` はページ内 `fetch` でローカル資源に到達し得る。`navigate` の
  egress は塞いだが、完全な隔離ではない。

## 軽量化・起動・常駐リソース（2026-11）

計画と経緯は [`performance-plan.md`](performance-plan.md) を参照。現行挙動は次のとおり。

- **起動時のバックアップ**（`storage/src/db.rs`）。`VACUUM INTO` は
  **マイグレーションが必要なときだけ** pool を開く前に実行します。通常起動では
  `<data-dir>/mix-agent.backup-state` の時刻を見て、24 時間以上経過していれば
  ウィンドウ表示後にバックグラウンドで 1 回だけ取得します。`PRAGMA quick_check(1)`
  は 7 日ごと（マーカーが無い初回・復元後は必ず）実行します。
- **起動スパン**。`target: "startup"` の `tracing` ログに storage の各段
  （backup / open / integrity / schema / data migrations）と `build_app_state`
  の各ブロックをミリ秒で出します。UI は `shell-ready` / `transcript-ready` の
  `performance.mark` を残します。
- **MCP の非同期接続**。設定の読み込み（SQLite）だけを起動時に行い、
  `connect_all` はウィンドウ表示後にバックグラウンドで実行します。最初の Run は
  `mcp_ready` を最大 3 秒待つので、接続直後の送信でも `mcp__*` ツールを落としません。
  サーバーごとの接続タイムアウトは 10 秒です。
- **Memory の FTS**（`0004_memory_search.sql`）。`memory_search`（trigram）を
  トリガーで維持し、3 文字以上のクエリは FTS の上位 2000 件を候補にしてから
  既存の `rank_score` で並べます。短いクエリと CJK の unigram/bigram は
  従来どおり全件ストリームにフォールバックします。
- **Run イベントの IPC バッチ**。`run_event_batch`（50ms / 64 件）に加え、
  UI 側は `runStore` で 60ms ごとにまとめて reducer へ適用します。
- **UI の購読境界**。`Sidebar` は `busyConversationIds` だけ、`ChatPane` は
  run id / status / approval だけを購読し、トークンで変わる timeline は
  `LiveBlock` だけが読みます。ストリーミング中の Markdown 描画は 120ms に間引きます。
- **ポーリング**。health / approvals / local runtimes / git status は
  `document.visibilityState` が `hidden` の間は止めます。`git status` は
  ツリー向けに `-unormal`、Changes ペイン向けに `-uall` を使い分け、branch 名は
  30 秒キャッシュします。
- **常駐リソース**。Chromium のアイドル終了は 5 分。LSP は 10 分アイドルで
  次のツール呼び出し時に破棄し、5 分ごとの housekeeping でも解放します。
  ログは 14 日分、`run_events` は完了済み Run の 180 日超を起動後に削除します
  （`messages` は削除しません）。`PRAGMA optimize` と passive checkpoint を
  起動後に実行します。
- **リリース最適化**。`profile.release` は `opt-level = 3`（速度優先）。
- **バンドル予算**。`scripts/check-desktop-bundle.mjs` が初期 chunk 500kB /
  JS 合計 1.5MB を CI で検査します。

## LSP / worktree / ロールバック / 大規模履歴（2026-11）

- **`lsp` ツール**（`tools/src/lsp.rs`）。rust-analyzer / typescript-language-server /
  pyright・pylsp / gopls を (workspace, language) 単位で起動し、`diagnostics`
  （LSP 3.17 の pull、未対応サーバーは `publishDiagnostics` 待ち）と `definition`
  を返します。サーバーが無い場合は Run を失敗させず、導入方法を含むツールエラーに
  します。`core/src/runs/stagnation.rs` では読み取り専用として扱い、plan モードでも
  使用できます。
- **サブエージェントの worktree 分離**（`tools/src/agent.rs`, `app/src/lib.rs`）。
  `agent` の `tasks` は既に並列実行でしたが、同じワークスペースを共有していたため
  並列編集が衝突しました。`isolate: true` で `git stash create` を基点にした
  detached worktree を作り、終了後に `git diff --cached --binary` と変更ファイル一覧を
  親へ返します。自動マージはせず、親が差分を確認して適用します。
- **Git ベースの Run ロールバック**（`app/src/workspace_baseline.rs`）。
  Run 開始時に作業ツリーを `refs/mix-agent/runs/<id>` に固定し、`revert_run_workspace`
  でトラッキング済みの変更を復元、Run が作った untracked ファイルを削除します。
  `edit` / `files` の in-memory スナップショットも従来どおり併用します。
- **会話一覧のカーソルページング**（`list_conversations_page`）。カーソルは
  `(updated_at, id)` の生テキストで、`Z` / `+00:00` の再整形によるページ境界の
  取りこぼしを避けます。`0003_conversation_page.sql` が `(updated_at DESC, id DESC)`
  のインデックスを追加します。
- **トランスクリプト仮想化**（`ui/src/features/chat/Transcript.tsx`）。
  40 メッセージ超で可視行 + overscan のみ描画し、古いページを先頭に足したときは
  スクロール位置を補正します。
- **コンテキスト圧縮の可視化**。`context_summary` の圧縮イベントを
  `RunTimeline.contextCompactions` に畳み、LiveBlock / ActivityPanel に
  「履歴を圧縮しました」カードを表示します。
- **ログイン時起動**（`app/src/autostart.rs`）。macOS LaunchAgent / Windows Run キー /
  Linux XDG autostart を直接管理し、`--hidden` でトレイ常駐起動します。

## UI/UX の改善（2026-10-05）

- **スレッドのサーフェス絞り込み**（`ui/src/features/shell/Sidebar.tsx`）。store には
  `threadFilter` / `setThreadFilter` が以前からありテストもあったが、UI が無く死んだ状態
  だった。サイドバー上部に `Segmented` を置き、会話の `mode` から所属サーフェスを引く
  `surfaceForMode()`（`ui/src/lib/ui.ts`）で「すべて / チャット / エージェント」を切り替える。
- **スレッド一覧のキーボード移動**。`↑` / `↓` / `Home` / `End` でタイトル間を移動する。
  rename 入力中は `[data-thread-open]` ガードで文字カーソル移動を妨げない。
- **ショートカット一覧**（`ui/src/features/shell/ShortcutHelp.tsx`）。`?` またはコマンド
  パレットから開き、`useAppShortcuts` のバインドを列挙する。入力欄にフォーカスがあるときは
  `?` を文字として通す（`e.target` が Element でない場合も含めてガード）。
- **失敗の可視化**。スレッドの Markdown エクスポートとアップデートのインストールは、失敗して
  も何も表示されない unhandled rejection だった。サイドバー / 更新バナーに `Alert` を出す。
- **ストリーミング時のレイアウト再計算**（`ui/src/features/shell/usePanelCap.ts`）。パネル幅の
  キャップを測る `MutationObserver` が、ストリーミング中に増えるノードごとに
  `getBoundingClientRect` を呼んでいた。1 フレームに 1 回へ間引く。

### 確認した項目

- `pnpm ui:typecheck` / `pnpm ui:lint`（`--max-warnings 0`）/ `pnpm ui:test`
  が成功（件数は `pnpm ui:test` で確認すること）。
- `pnpm --filter mix-agent-desktop-ui format:check`（prettier）が成功。
  既存ツリー全体への適用は未実施のため CI ゲートには未追加。新規ファイルは
  追加時に整形すること。
- モック IPC で Tauri を介さず dev プレビューを起動し、フィルター切替でエージェント 2 件 /
  チャット 4 件に絞られること、`?` で一覧が開き入力欄では開かないこと、
  1440 / 1024 / 820px で横スクロールが出ないことを DOM で確認。
- 実機（Tauri / WebView）での目視とスクリーンリーダーは未確認。

## 既知の残タスク

- Knowledge は FTS5（trigram）の lexical 検索に対応。embedding 検索は未実装。
- Memory の自動生成（Run 後の抽出）は Phase 2 で実装済みだが、抽出品質の改善は継続課題（[`../memory-roadmap.md`](../memory-roadmap.md) Phase 2 参照）。
- 対話 terminal は `action: "send"` まで実装済み。専用のターミナル UI とシグナル送出は未実装。
- Windows のサンドボックスは Job Object によるプロセスツリー隔離まで。ファイルシステムの
  書き込み隔離（AppContainer / restricted token）は portable-pty の制約で未対応。
- LSP は `lsp` ツールとして実装済み。対応言語は Rust / TypeScript / Python / Go で、
  サーバーは PATH または `MIX_LSP_*` で指定する。
- Database at-rest encryption（SQLCipher）は未配線。設定に互換用の
  `database_encryption` キーは残るが、常に `false` として読み書きし、UI でも
  「未対応」と表示する。実データは平文 SQLite + 所有者限定パーミッションのみ
  なので、OS のディスク暗号化（FileVault / BitLocker / LUKS）を前提とする。
- Provider プリセットは Docker 版の 53 件中 51 件（`cloudflare` と
  `azure-ai-foundry` が未移植）。`chatgpt` はプリセットではなく、専用タブの
  非公式連携として実装している。

## ライセンス

**確定: Apache License 2.0**。リポジトリ全体の `LICENSE` が Apache-2.0 であり、
Desktop 側も `apps/desktop/LICENSE` と `apps/desktop/Cargo.toml` の `license` を
含めて Apache-2.0 に統一されている（MIT ではない）。
変更する場合はルート `LICENSE` / `NOTICE` / `apps/desktop/NOTICE` と
`apps/desktop/Cargo.toml` の `license` を同時に書き換えること。

## 関連ドキュメント

- [`../README.md`](../README.md) — ドキュメント索引と役割の一覧
- [`../architecture.md`](../architecture.md) — プロジェクト全体と Docker 版の構成
- [`../memory-spec.md`](../memory-spec.md) — 両エディション共通の Memory 契約
- [`api-mapping.md`](api-mapping.md) — Docker 版 REST と Tauri コマンドの対応表
- [`ui-conventions.md`](ui-conventions.md) — UI プリミティブと表記の基準
- [`update-signing.md`](update-signing.md) — アップデート署名鍵の運用
- [`coding-agent-plan.md`](coding-agent-plan.md) — コーディングエージェント化の経緯と残計画
- [`coding-eval.md`](coding-eval.md) — コーディングエージェントの評価ハーネス
- [`local-runtime-plan.md`](local-runtime-plan.md) — ローカル LLM 連携の設計記録
- [`chromium-on-demand-plan.md`](chromium-on-demand-plan.md) — Chromium 初回インストール方式の設計記録
- [`performance-plan.md`](performance-plan.md) — 起動・描画・常駐リソースの軽量化計画
- [`agent-speed-plan.md`](agent-speed-plan.md) — エージェント 1 ターンの高速化計画
- [`../verification.md`](../verification.md) — 実機・Provider の検証記録
- [`../../apps/desktop/README.md`](../../apps/desktop/README.md) — Desktop 版の入口・セットアップ・開発手順
