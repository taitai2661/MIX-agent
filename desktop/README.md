# MIX-agent Desktop

Tauri 2 + Rust + React 19 で再実装されたローカル AI ワークスペース。
[Docker 版](../docker/MIX-agent/) の中核機能（Provider Adapter / Run Engine / Tool Registry / SSE 契約）を踏襲しつつ、**単一ユーザー・OS Keychain 連携**のネイティブアプリとして動作します。

> **重要**: `desktop/` は `docker/` へ一切依存しません。両者は仕様書（[`docs/desktop/architecture.md`](../docs/desktop/architecture.md) と [`docs/desktop/api-mapping.md`](../docs/desktop/api-mapping.md)、[`docker/MIX-agent/docs/architecture.md`](../docker/MIX-agent/docs/architecture.md)）のみで結ばれます。

## 構成

```
desktop/mix-agent-desktop/
├── Cargo.toml            # Cargo workspace root（app / core / providers / tools / browser / mcp / storage）
├── rust-toolchain.toml   # 固定ツールチェーン（stable + rustfmt / clippy）
├── rustfmt.toml          # rustfmt 設定
├── clippy.toml           # clippy 設定
├── package.json          # pnpm workspace root
├── pnpm-workspace.yaml   # ui/ のみメンバー
├── app/                  # Tauri 2 shell（バイナリ）
├── core/                 # Run Engine / state / events / context
├── providers/            # wire 実装（OpenAI / Anthropic / Gemini）+ ラッパー（OpenAI-Compatible / Ollama / LM Studio）
├── tools/                # Tool Registry + executor
├── browser/              # Browser Operator + chromiumoxide backend
├── mcp/                  # MCP client host
├── storage/              # SQLite + Keychain
├── scripts/              # fetch-chromium.mjs（同梱 Chromium の取得）
└── ui/                   # React 19 + Vite + Tailwind
```

## 必要環境

| ツール | バージョン | 用途 |
| --- | --- | --- |
| Rust | 1.77 以上（edition 2021） | Cargo workspace |
| Node.js | 22 以上（`engines.node: ">=22.0.0"`） | UI |
| pnpm | 10.x（`engines.pnpm: ">=10.0.0"` / `packageManager: pnpm@10.11.0`） | UI パッケージマネージャ |
| Tauri CLI | 2.x（`@tauri-apps/cli ^2.1.0`） | `tauri dev` / `tauri build` |

Node.js / pnpm の下限は `package.json` の `engines` で宣言されています（`.nvmrc` は用意していません）。
CI は Node 22 系に固定しています。

`rust-toolchain.toml` は `channel = "stable"`（`components = ["rustfmt", "clippy"]`）を固定し、
CI の `dtolnay/rust-toolchain@stable` と揃えています。

OS 別追加要件:

- **macOS**: Xcode Command Line Tools（実行には macOS 11.0 以上）
- **Windows**: Visual Studio Build Tools 2022 (C++ workload) + WebView2
- **Linux**: `libwebkit2gtk-4.1-dev`, `libssl-dev`, `librsvg2-dev`, `libappindicator3-dev`, `libayatana-appindicator3-dev`, `libxdo-dev`, `libgtk-3-dev`, `patchelf`（`.github/workflows/desktop-release.yml:73-75` の apt 一覧に対応）

## クイックスタート

```sh
cd desktop/mix-agent-desktop
pnpm install
pnpm tauri dev
```

初回起動時は Tauri が Cargo workspace をビルドするため数分かかります。`tauri.conf.json` の `beforeDevCommand` で `pnpm --filter mix-agent-desktop-ui dev` が走り、Vite dev サーバー（http://localhost:5173、`--strictPort`）に Tauri WebView が接続します。

## ビルド

```sh
cd desktop/mix-agent-desktop
pnpm tauri build
```

`beforeBuildCommand` は UI ビルドに加えて `node scripts/fetch-chromium.mjs` を実行し、
同梱 Chromium を `app/resources/chromium/` へ取得します（`.gitignore` 対象、
1 プラットフォーム約 370 MB）。`chromium` は `bundle.resources` に宣言されているため
（`app/tauri.conf.json:51`）、この取得を飛ばすと同梱 Chromium を含まないアプリになり、
実行時はシステムブラウザへフォールバックします。`MIX_SKIP_CHROMIUM=1` を指定すれば
ダウンロード自体をスキップできますが、その場合はこの点を踏まえて利用してください。
その他の指定は `MIX_CHROMIUM_CHANNEL=beta|canary|dev` と `MIX_CHROMIUM_VERSION=<固定版>` です。

成果物は Cargo workspace の target ディレクトリ（`desktop/mix-agent-desktop/target/`）
配下、`release/bundle/` に生成されます（`app/target/` ではありません）。
ローカルビルドでは `target/release/bundle/`、CI のクロスコンパイルでは
target triple 付きの `target/<triple>/release/bundle/` になります。
`bundle.targets` は `"all"` ですが、CI はプラットフォーム별로
`app` / `dmg` / `msi` / `nsis` / `deb` / `appimage` を指定して `--bundles` を渡します。

## スクリプト

| コマンド | 内容 |
| --- | --- |
| `pnpm rust:check` | `cargo check --workspace --all-targets` |
| `pnpm rust:fmt` / `pnpm rust:fmt:check` | `cargo fmt --all` / `cargo fmt --all -- --check` |
| `pnpm rust:clippy` | `cargo clippy --workspace --all-targets -- -D warnings` |
| `pnpm rust:test` | `cargo test --workspace --all-targets` |
| `pnpm rust:clean` / `pnpm rust:clean:all` | `cargo clean --profile dev` / `cargo clean` |
| `pnpm ui:dev` / `pnpm ui:build` / `pnpm ui:test` / `pnpm ui:lint` / `pnpm ui:typecheck` | UI サブセット（`pnpm --filter mix-agent-desktop-ui <task>`） |
| `pnpm tauri:dev` / `pnpm tauri:build` | アプリの起動 / 配布ビルド（`pnpm tauri dev` / `pnpm tauri build` と同じ） |
| `pnpm lint` | `rust:fmt:check` + `rust:clippy` + `ui:lint` + `ui:typecheck` |
| `pnpm test` | `rust:test` + `ui:test` |
| `pnpm check` | `rust:check` + `ui:build` |

上記はすべて `desktop/mix-agent-desktop/` で実行するスクリプトです。`desktop/` 直下には
`test` / `lint` / `check` のみを `pnpm --dir mix-agent-desktop <task>` へ委譲する
薄いルート `package.json` があり、CI も `desktop/mix-agent-desktop/` を
`working-directory` として実行します。

## ツール

Agent が実際に使えるツールです。`chat` モードはツールを使わず、`thinking` / `agent` は以下を利用します。
なお Run Engine 側はモードに関係なく spec 全体を返すので、`chat` での絞り込みは
app コマンド層（`app/src/commands/conversations.rs:487-492`）で行われます。

| ツール | 概要 | 既定権限 |
| --- | --- | --- |
| `files` | read / write / list / search / delete。`read` は `offset` / `limit` でページング | 読み=allow / 書込=ask |
| `edit` | 最小差分の置換・新規作成・追記。アンカーが曖昧なら推測せず拒否 | ask |
| `glob` | glob パターンでファイル検索。`.gitignore` 準拠 | allow |
| `grep` | 正規表現で内容検索。バイナリ除外・glob フィルタ対応 | allow |
| `terminal` | 永続シェル（PTY）。`cd` や環境変数が次の呼び出しに引き継がれる | ask |
| `todo` | 複数ステップ作業のチェックリスト。各ターンの先頭に再注入される | allow |
| `skill` | インストール済み `SKILL.md` の手順本文を取得 | allow |
| `agent` | 保存済みエージェントプロファイルへサブdelegate | ask |
| `web_search` | DuckDuckGo 検索 | allow |
| `web_fetch` / `network` | URL 取得。egress allowlist と private IP 拒否 | ask |
| `image_vision` | 添付画像の検証 | allow |
| `user_plan` | 構造化プランの提示 | ask |
| `browser` | 実 Chromium の操作（workspace ごとに永続プロファイル） | ask |
| `mcp__<conn>__<tool>` | MCP サーバーが公開するツール | ask |

権限はツール単位ではなく**呼び出し単位**で決まります。保存済みルールがあれば常にそれが優先し、無い場合の既定値は「読み取りは allow、書き込み・シェル・ネットワークは ask」です。

> `browser` と `mcp__<conn>__<tool>` は Tool Registry に登録されず dispatch フック経由で
> 実行されるため、判定は「未知のツール」として `ask` にフォールバックします
> （`tools/src/registry.rs:238-260`）。他の行のような保存ルールの明示一致ではありません。

## 開発フェーズ

| Phase | 内容 | 状態 |
| --- | --- | --- |
| 0 | 雛形 / workspace / capabilities / CI ベース | ✅ |
| 1 | storage + providers + 単発Chat | ✅ |
| 2 | tools + permission gate + egress | ✅ |
| 3 | Agent 自律性 (checkpoint / stagnation / verification / budget / 文脈圧縮) | ✅ |
| 4 | MCP host + Browser Operator (chromiumoxide) | ✅ |
| 5 | Memory / Knowledge / Skills / Agents / Projects / Schedules | ✅ |
| 6 | tray / menu / notification / auto-update / backup / release CI | ✅ |
| 7 | macOS 完成 + Windows α + Linux α | ⏳ |

auto-update は基盤（`updater.rs`、署名付きアーティファクト設定、メニュー項目、
`desktop-release.yml`）まで用意されていますが、`tauri.conf.json` では
`plugins.updater.active = false` / `endpoints: []` のままなので、出荷版は更新を
確認しません。有効化の手順は [`docs/desktop/architecture.md`](../docs/desktop/architecture.md) を参照してください。

### モード別リミット

Run Engine の既定上限はモードごとに次のように決まります（`core/src/mode.rs:31-53`）。

| Mode | step 上限 | tool-call 上限 | wall clock |
| --- | --- | --- | --- |
| `chat` | 12 | 12 | 20 分 |
| `thinking` | 24 | 24 | 45 分 |
| `agent` | 300 | 750 | 90 分 |

checkpoint 間隔（既定 10 ステップ）や停滞判定・検証などの自律性調整は
`core/src/runs/mode_policy.rs` の `AgentModePolicy` に集約されています。

### Deep link

`mix-agent-desktop://` スキームが deep-link プラグインで受理されます
（`app/tauri.conf.json:93-99`）。single-instance プラグインと併用され、
2 起動目以降は既存インスタンスへ転送されます。

### Provider 対応

`providers/` は 3 つの wire 形式（OpenAI / Anthropic / Gemini）を実装します。OpenAI-Compatible / Ollama / LM Studio は base URL を差し込んだ `OpenAiProvider` の薄いラッパーで、独立した wire 実装ではありません（`providers/src/ollama.rs`、`providers/src/lmstudio.rs` のコメントのとおり「we reuse the OpenAI client」）。Responses API への対応は未実装です（`providers/src/openai_compat_wire.rs`）。

プリセット `ProviderKind` は 7 種で、残る OpenRouter は OpenAI-Compatible の経路を通ります。プリセットは 50 件（Docker 版の 52 件のうち `cloudflare` と `azure-ai-foundry` を除く）です。

3 つの主要プロバイダは tool-use の往復をそれぞれの wire 形式（`tool_calls` / `tool_use`+`tool_result` / `functionCall`+`functionResponse`）で正しく符号化します。429 / 5xx / ネットワーク断は指数バックオフ（`Retry-After` 優先）で再試行します（`providers/src/reliability.rs`）。

### テスト

`cargo test --workspace`（Rust）と `pnpm ui:test`（UI / vitest）で実行します。現在の件数は
**Rust 403 件**（`#[test]` / `#[tokio::test]`）と **UI 91 ケース / 13 ファイル**です。
テストは継続的に追加されているため、数値はテストの追加・削除に合わせて更新してください。

Rust 側は workspace 内に 7 つの `tests/` ディレクトリを持ちます。

| パス | 対象 |
| --- | --- |
| `core/tests/engine_loop.rs` | Run Engine（下記参照） |
| `app/tests/` | history / log_file / project_runtime / providers / runs / settings の 6 ファイル |
| `providers/tests/` `tools/tests/` `mcp/tests/` `browser/tests/` `storage/tests/` | 各クレートの統合テスト |

`core/tests/engine_loop.rs` は Run Engine のループ・権限ゲート・検証・チェックポイント・
transcript 永続化に加え、budget 拡張・キャンセル・resume と履歴再生・workspace root からの
脱出拒否・tool 失敗の伝播を、スクリプト化された provider に対して検証します。

## ライセンス

[Apache License 2.0](../LICENSE) / [NOTICE](../NOTICE)
