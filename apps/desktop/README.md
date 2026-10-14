# MIX-agent Desktop

Tauri 2 + Rust + React 19 で再実装されたローカル AI ワークスペース。
[Docker 版](../docker/MIX-agent/) の中核機能（Provider Adapter / Run Engine / Tool Registry / SSE 契約）を踏襲しつつ、**単一ユーザー・OS Keychain 連携**のネイティブアプリとして動作します。

> **重要**: `desktop/` は `docker/` へ一切依存しません。両者は仕様書（[`docs/architecture.md`](docs/architecture.md) と [`docs/api-mapping.md`](docs/api-mapping.md)、[`docker/MIX-agent/docs/architecture.md`](../docker/MIX-agent/docs/architecture.md)）のみで結ばれます。

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
| Rust | 1.82 以上（edition 2021） | Cargo workspace |
| Node.js | 22 以上（`engines.node: ">=22.0.0"`） | UI |
| pnpm | 10.x（`engines.pnpm: ">=10.0.0"` / `packageManager: pnpm@10.11.0`） | UI パッケージマネージャ |
| Tauri CLI | 2.x（`@tauri-apps/cli ^2.12.1`） | `tauri dev` / `tauri build` |

Node.js / pnpm の下限は `package.json` の `engines` で宣言されています（`.nvmrc` は用意していません）。
CI は Node 22 系に固定しています。

`rust-toolchain.toml` は `channel = "1.91.0"`（`components = ["rustfmt", "clippy"]`）にピン留めし、
CI の `dtolnay/rust-toolchain` の `toolchain: "1.91.0"` と揃えています（`stable` 追従だと
新規 lint で `-D warnings` が非決定的に赤くなるため）。実効下限は 1.90
（tray-icon 0.25 / edition2024 依存のため 1.82 未満では lockfile を解釈できません）。
`Cargo.toml` の `rust-version = "1.82"` と `clippy.toml` の `msrv = "1.82"` は
あくまで宣言上の下限であり、実際にビルドが通るのは 1.90 以上です。

OS 別追加要件:

- **macOS**: Xcode Command Line Tools（実行には macOS 12.0 以上。
  `minimumSystemVersion: 12.0` は同梱 Chromium（Chrome for Testing）の要件に合わせている）
- **Windows**: Visual Studio Build Tools 2022 (C++ workload) + WebView2
- **Linux**: `libwebkit2gtk-4.1-dev`, `libssl-dev`, `librsvg2-dev`, `libappindicator3-dev`, `libayatana-appindicator3-dev`, `libxdo-dev`, `libgtk-3-dev`, `patchelf`（`../../.github/workflows/desktop-release.yml:95-100` の apt 一覧に対応）

## クイックスタート

```sh
cd desktop/mix-agent-desktop
pnpm install
pnpm tauri dev
```

初回起動時は Tauri が Cargo workspace をビルドするため数分かかります。`tauri.conf.json` の `beforeDevCommand` で `node scripts/ensure-chromium-dir.mjs`（`mix-agent-desktop/scripts/` 配下、以下同）の後に `pnpm --filter mix-agent-desktop-ui dev` が走り、Vite dev サーバー（http://localhost:5173、`--strictPort`）に Tauri WebView が接続します。

`ensure-chromium-dir.mjs` は `bundle.resources` に宣言された `app/resources/chromium/` が無い場合に空ディレクトリを作成します。`tauri-build` は宣言されたリソースの欠落をビルドエラーにするため、`.gitignore` 対象のこのディレクトリが無いクリーンなチェックアウトでは `tauri dev` が通りません。dev では同梱 Chromium（約 370 MB）を取得せず、実行時にシステム Chromium へフォールバックします。

## ビルド

```sh
cd desktop/mix-agent-desktop
pnpm tauri build
```

`beforeBuildCommand` は UI ビルドに加えて `node scripts/fetch-chromium.mjs`（`mix-agent-desktop/scripts/` 配下）を実行し、
同梱 Chromium を `app/resources/chromium/` へ取得します（`.gitignore` 対象、
1 プラットフォーム約 370 MB）。`chromium` は `bundle.resources` に宣言されているため
（`app/tauri.conf.json:51`）、この取得を飛ばすと同梱 Chromium を含まないアプリになり、
実行時はシステムブラウザへフォールバックします。`MIX_SKIP_CHROMIUM=1` を指定すれば
ダウンロード自体をスキップできますが、dev 専用であり release ビルドでは
`MIX_REQUIRE_CHROMIUM=1` が fail-closed（browser-less 出荷を拒否）になります。
release CI は `MIX_CHROMIUM_VERSION` をピン留めし、`chromium-version.json` と
バイナリの存在を `tauri build` 前にアサートして `chromium-version` を artifact に上げます。
その他の指定は `MIX_CHROMIUM_CHANNEL=beta|canary|dev` と `MIX_CHROMIUM_VERSION=<固定版>` です。
リトライ（3 回・指数バックオフ）と `MIX_CHROMIUM_SHA256` / `MIX_CHROMIUM_SIZE` による
検証、`unzip` / `Expand-Archive` 不在時の明示エラーに対応しています。

成果物は Cargo workspace の target ディレクトリ（`desktop/mix-agent-desktop/target/`）
配下、`release/bundle/` に生成されます（`app/target/` ではありません）。
ローカルビルドでは `target/release/bundle/`、CI のクロスコンパイルでは
target triple 付きの `target/<triple>/release/bundle/` になります。
`bundle.targets` は `"all"` ですが、CI はプラットフォーム별로
`app` / `dmg` / `msi` / `nsis` / `deb` / `appimage` を指定して `--bundles` を渡します。
CI は `beforeBuildCommand` に UI ビルド＋Chromium 取得を一任しているため、
別途 `pnpm build` を回さず Vite の二重ビルドを避けています。

リリースタグは Docker 版（`v*` → `../../.github/workflows/release.yml`）と Desktop 版
（`desktop-v*` → `../../.github/workflows/desktop-release.yml`）で分離しています。
Desktop の対応範囲は α として x64/arm64 macOS・x64 Windows・x64 Linux のみで、
`aarch64-linux` / `universal-macos` / `win-arm64` は明示的に defer しています
（`desktop-release.yml` の matrix コメント参照）。必須バンドル欠落時は
`if-no-files-found: error` で失敗し、artifact retention は 14 日です。

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

Agent が実際に使えるツールです。`chat` は委任 (`agent`) と長期計画 (`todo` / `user_plan`) を除く
読み取り・検索・ファイル・実行系を利用します (Docker版 `allowed_modes` 互換)。
`thinking` は `agent` のみ除外、`agent` / `plan` は全件です。
なお Run Engine 側はモードに関係なく渡された spec を実行するので、モードの絞り込みは
app コマンド層 (`app/src/commands/conversations.rs` の `build_tool_specs`、
`tools/src/registry.rs` の `specs_for_mode`) と system prompt
(`core/src/prompt.rs` の `compose_system_for_run`) で行われます。

| ツール | 概要 | 既定権限 |
| --- | --- | --- |
| `files` | read / write / list / search / delete。`read` は `offset` / `limit` でページング | 読み=allow / 書込=ask |
| `edit` | 最小差分の置換・新規作成・追記。アンカーが曖昧なら推測せず拒否 | ask |
| `glob` | glob パターンでファイル検索。`.gitignore` 準拠 | allow |
| `grep` | 正規表現で内容検索。バイナリ除外・glob フィルタ対応 | allow |
| `terminal` | 永続シェル（PTY）。`cd` や環境変数が次の呼び出しに引き継がれる | ask |
| `workspace_check` | シェルなし固定チェック。`git_status` / `git_diff_check` / `python_syntax` | allow |
| `knowledge` | `add` / `search` / `delete` / `clip`（Web取得→保存の一括、Docker `web_clip_save` 相当） | 検索=allow / 書込=ask |
| `todo` | 複数ステップ作業のチェックリスト。各ターンの先頭に再注入される | allow |
| `skill` | インストール済み `SKILL.md` の手順本文を取得 | allow |
| `agent` | 保存済みエージェントプロファイルへサブdelegate | ask |
| `web_search` | DuckDuckGo 検索 | allow |
| `web_fetch` / `network` | URL 取得。egress allowlist と private IP 拒否 | ask |
| `web_fetch_pdf` | 公開PDFのテキスト抽出 (最大20MB・100ページ・5万字)。HTML偽装は拒否 | ask |
| `image_vision` | 添付画像の検証 | allow |
| `user_plan` | 構造化プランの提示 | ask |
| `browser` | 実 Chromium の操作（workspace ごとに永続プロファイル） | ask |
| `mcp__<conn>__<tool>` | MCP サーバーが公開するツール | ask |

権限はツール単位ではなく**呼び出し単位**で決まります。保存済みルールがあれば常にそれが優先し、無い場合の既定値は「読み取りは allow、書き込み・シェル・ネットワークは ask」です。

承認カードの「常に許可」は、そのツール全体ではなく**その呼び出しの範囲**（Run の workspace と、ツールが公開する対象＝パス・コマンド・URL・browser の action・MCP 接続 ID）に絞ったルールを保存します。`terminal` の 1 コマンドを許可しても、別のコマンドや別の workspace、`files` の削除、`browser` の `evaluate` は引き続き確認されます。ツールに対象が無い場合は workspace 単位の許可になります。

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
| 6 | tray / menu / notification / backup / release CI（auto-update 基盤のみ、配布更新なし） | ✅ |
| 7 | macOS 完成 + Windows α + Linux α | ⏳ |

auto-update は基盤（`updater.rs`、署名付きアーティファクト設定、メニュー項目、
`desktop-release.yml`）まで用意されていますが、`tauri.conf.json` では
`plugins.updater.active = false` / `endpoints: []` のままなので、出荷版は更新を
確認しません。Phase 6 は「更新なしの release CI」として確定しています。
有効化の手順は [`docs/architecture.md`](docs/architecture.md) を参照してください。

### 空チャットの挨拶

スレッドが空のとき、見出しは時間帯・曜日・季節の行事・二十四節気・日本の祝日・
スレッド数・最終活動・訪問間隔・サーフェス・プロジェクト有無から文を合成して表示します
（`ui/src/features/chat/greeting/`）。部品は約250文で、「挨拶＋状況節＋締め」の3層を
文体タグと文形タグで互換判定してから連結します。見出しの直下には補足の1文と、
文脈から選ぶ開始提案が3枚並びます。提案は送信欄へ差し込むだけなので、自動送信はしません。

英訳は日本語原文ではなく部品 ID で引くため（`greeting/translations.ts`）、
原文の言い回しを直しても英語表示は壊れません。祝日・二十四節気・季節の行事は
日本の暦に基づくため、表示言語が日本語のときだけ状況節として使います。

`localStorage`（`mix-agent-welcome`）に訪問間隔と直近24件の部品 ID を保持し、
同じ文面の連続を避けます。

```sh
cd ui && pnpm ui:test      # greeting/ を含むケース（件数は `pnpm ui:test` で確認）
```

Docker 版も同じ方式で実装していますが、両者は [`docs/architecture.md`](docs/architecture.md)
の方針どおりコードを共有せず、Desktop 版は `スレッド` / プロジェクト / 差分 /
Browser など自分の語彙で書き下しています。

### モデル選択と一時モード

チャット送信欄のモデル選択は、検索・プロバイダー別グルーピング・能力アイコン
（Vision / Tools / Reasoning）・コンテキスト長を表示するポップオーバーです
（`ui/src/features/chat/ModelPicker.tsx`）。お気に入り（ピン）と最近使ったモデル、
サーフェスごとの最後の選択は `localStorage` に保持し（`ui/src/lib/modelPrefs.ts`）、
モデルが未設定のときはプロバイダー追加へ、下部の「モデルを管理」からは設定の
モデル一覧へ直接移動できます。`ModelsPane` / `ProvidersPane` も検索・グルーピング・
能力表示・接続テスト・インライン編集に対応しました。設定 → 一般の「既定モデル」も
同じピッカーです。

Docker 版と同じ「一時モード」をモデルピッカーから有効にできます。有効な送信では
Memory / Knowledge の検索をスキップし（`core/src/runs/engine.rs` の `temporary`）、
Tool は既定で無効、許可しても Memory / Skill 系は除外されます。Run 終了後はその
ターンのメッセージ・添付・Run 行を削除し、新規スレッドだった場合はスレッドごと
削除します（`app/src/commands/conversations.rs` の `purge_temporary_run`）。

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
2 起動目以降は既存インスタンスへ転送されます。転送された URL は
`deep_link::dispatch`（OAuth コールバック処理）へ渡され、`deep-link`
イベントとしても emit されます。single-instance 側は capability 不要です
（deep-link の `deep-link:default` とは非対称）。
OS 別の登録: macOS は URL scheme（Info.plist 経由）、Windows はレジストリ、
Linux は `.desktop` の `MimeType=x-scheme-handler/mix-agent-desktop` 関連付けで
行われます（いずれも Tauri が `tauri.conf.json` の scheme 定義から生成）。

### Provider 対応

`providers/` は 3 つの wire 形式（OpenAI / Anthropic / Gemini）を実装します。OpenAI-Compatible / Ollama / LM Studio は base URL を差し込んだ `OpenAiProvider` の薄いラッパーで、独立した wire 実装ではありません（`providers/src/ollama.rs`、`providers/src/lmstudio.rs` のコメントのとおり「we reuse the OpenAI client」）。Responses API への対応は未実装です（`providers/src/openai_compat_wire.rs`）。

プリセット `ProviderKind` は 7 種で、残る OpenRouter は OpenAI-Compatible の経路を通ります。プリセットは 50 件（Docker 版の 52 件のうち `cloudflare` と `azure-ai-foundry` を除く）です。

3 つの主要プロバイダは tool-use の往復をそれぞれの wire 形式（`tool_calls` / `tool_use`+`tool_result` / `functionCall`+`functionResponse`）で正しく符号化します。429 / 5xx / ネットワーク断は指数バックオフ（`Retry-After` 優先）で再試行します（`providers/src/reliability.rs`）。

### テスト

`cargo test --workspace`（Rust）と `pnpm ui:test`（UI / vitest）で実行します。
件数は変動するため、絶対数は記載しません（`cargo test --workspace` /
`pnpm ui:test` の出力で確認してください）。バージョンの 4 箇所
（workspace `Cargo.toml` / `app` / `tauri.conf.json` / `package.json` +
`ui/package.json`、いずれも `0.1.0`）は `app_version_matches_all_manifests`
テストと CI で同期を検証します。

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
