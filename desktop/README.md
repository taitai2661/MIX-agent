# MIX-agent Desktop

Tauri 2 + Rust + React 19 で再実装されたローカル AI ワークスペース。
[Docker 版](../docker/MIX-agent/) の中核機能（Provider Adapter / Run Engine / Tool Registry / SSE 契約）を踏襲しつつ、**単一ユーザー・OS Keychain 連携**のネイティブアプリとして動作します。

> **重要**: `desktop/` は `docker/` へ一切依存しません。両者は仕様書（[`docs/desktop/architecture.md`](../docs/desktop/architecture.md) と [`docker/MIX-agent/docs/architecture.md`](../docker/MIX-agent/docs/architecture.md)）のみで結ばれます。

## 構成

```
desktop/mix-agent-desktop/
├── Cargo.toml            # Cargo workspace root（app / core / providers / tools / browser / mcp / storage）
├── package.json          # npm workspace root
├── pnpm-workspace.yaml   # ui/ のみメンバー
├── app/                  # Tauri 2 shell（バイナリ）
├── core/                 # Run Engine / state / events / context
├── providers/            # OpenAI / Anthropic / Gemini / OpenAI-Compatible / Ollama / LM Studio
├── tools/                # Tool Registry + executor
├── browser/              # Browser Operator + chromiumoxide backend
├── mcp/                  # MCP client host
├── storage/              # SQLite + Keychain
└── ui/                   # React 19 + Vite + Tailwind
```

## 必要環境

| ツール | バージョン | 用途 |
| --- | --- | --- |
| Rust | 1.77 以上（edition 2021） | Cargo workspace |
| Node.js | 22 以上 | UI |
| pnpm | 10.x | UI パッケージマネージャ |
| Tauri CLI | 2.x | `tauri dev` / `tauri build` |

OS 別追加要件:

- **macOS**: Xcode Command Line Tools
- **Windows**: Visual Studio Build Tools 2022 (C++ workload) + WebView2
- **Linux**: `libwebkit2gtk-4.1-dev`, `libssl-dev`, `librsvg2-dev`, `libayatana-appindicator3-dev`

## クイックスタート

```sh
cd desktop/mix-agent-desktop
pnpm install
pnpm tauri dev
```

初回起動時は WebView が Cargo workspace をビルドするため数分かかります。`tauri.conf.json` の `beforeDevCommand` で `pnpm --filter mix-agent-desktop-ui dev` が走り、Vite dev サーバー（http://localhost:5173）に Tauri WebView が接続します。

## ビルド

```sh
cd desktop/mix-agent-desktop
pnpm tauri build
```

成果物は `app/target/release/bundle/` に生成されます。

## スクリプト

| コマンド | 内容 |
| --- | --- |
| `pnpm rust:check` | `cargo check --workspace --all-targets` |
| `pnpm rust:fmt:check` | `cargo fmt --all -- --check` |
| `pnpm rust:clippy` | `cargo clippy --workspace --all-targets -- -D warnings` |
| `pnpm rust:test` | `cargo test --workspace --all-targets` |
| `pnpm ui:dev` / `pnpm ui:build` / `pnpm ui:test` / `pnpm ui:typecheck` | UI サブセット |
| `pnpm lint` / `pnpm test` / `pnpm check` | 全体チェック（`lint` = rustfmt + clippy + eslint + tsc） |

## ツール

Agent が実際に使えるツールです。`chat` モードはツールを持たず、`thinking` / `agent` は以下を利用します。

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

## 開発フェーズ

| Phase | 内容 | 状態 |
| --- | --- | --- |
| 0 | 雛形 / workspace / capabilities / CI ベース | ✅ |
| 1 | storage + providers + 単発Chat | ✅ |
| 2 | tools + permission + egress | ✅ |
| 3 | Agent 自律性 (checkpoint / stagnation / verification / budget) | ✅ |
| 4 | MCP host + Browser Operator (chromiumoxide) | ✅ |
| 5 | Memory / Knowledge / Skills / Agents / Projects / Schedules | ✅ |
| 6 | macOS 完成 + Windows α + Linux α | ⏳ |

### Provider 対応

`providers/` は OpenAI / Anthropic / Gemini / OpenAI-Compatible / Ollama / LM Studio の 6 種を実装し、プリセットは 50 件です。3 つの主要プロバイダは tool-use の往復をそれぞれの wire 形式（`tool_calls` / `tool_use`+`tool_result` / `functionCall`+`functionResponse`）で正しく符号化します。429 / 5xx / ネットワーク断は指数バックオフ（`Retry-After` 優先）で再試行します。

### テスト

`cargo test --workspace`（Rust）と `pnpm ui:test`（UI / vitest）で実行します。件数は実行結果に従ってください（テストは継続的に追加されています）。うち `core/tests/engine_loop.rs` は Run Engine のループ・権限ゲート・検証・チェックポイント・transcript 永続化を、スクリプト化された provider に対して検証します。

## ライセンス

[Apache License 2.0](../LICENSE) / [NOTICE](../NOTICE)
