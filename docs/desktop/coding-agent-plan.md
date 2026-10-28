# MIX-agent Desktop — コーディングエージェント化計画

> 状態: Phase 8.1〜8.4 完了。Phase 10.0（ツール契約の修復）完了。次は Phase 10.1（評価ハーネス）。

## 1. 目的

現状の `chat / thinking / plan / agent` + Run Engine を、Claude Code / OpenCode 級の
コーディングエージェントにする。定義は以下を自律的に回せること:

1. 探索 (repo 把握) → 計画 (`user_plan`) → 複数ファイル編集 → 実行確認 (test/lint/typecheck) → 修正ループ → 報告
2. 失敗時に止まらず、検証・差分・チェックポイントで復帰できる
3. 権限ゲート (`tools/src/permission.rs`, `tools/src/execute.rs`) を壊さない

## 2. 実装完了機能 (Phase 8.1〜8.4)

### 2.1 編集 & 検証 (Phase 8.1 完了)
- `edit` ツールへの `edits` (順序付き複数アンカー `multi_edit`、atomic、失敗時ロールバック) の追加 (`tools/src/edit.rs`)。
- `auto_verify`: `RunRequest` の `verify_command` を Engine が `terminal` 経由で自動実行し、`VerificationLedger` を自動閉鎖 (`core/src/runs/engine.rs`)。

### 2.2 コード構造探索 (Phase 8.2 完了)
- `symbols` ツール: ① `outline <file>` (関数/クラス/構造体/型一覧)、② `references <symbol>` (シンボル参照箇所検索) (`tools/src/symbols.rs`)。
- 停滞時 (`stagnation_detected`) / 検証失敗時の再検索フック。

### 2.3 Git 連携 (Phase 8.3 完了)
- `git` ツール (読取系): `status` (変更ファイル・ブランチ)、`diff` (unstaged / staged / file 差分) (`tools/src/git.rs`)。

### 2.4 UI 証跡 & 承認体験 (Phase 8.4 完了)
- `ApprovalCard` / `ApprovalDiffView`: 承認時に変更前後の差分ハイライトを直接プレビュー可能 (`ui/src/features/chat/ApprovalDiffView.tsx`)。
- `ApprovalBanner`: 画面上部への承認待ち固定バナー、`⌘+Enter` (承認) / `Esc` (拒否) キーボードショートカット。
- `TerminalResultView`: 終了コードバッジ（0: 緑、非0: 赤）、実行時間（ms/s）、ワンクリックコピー、ANSI カラー装飾。
- `RightPanel`: ドッキング（分割表示）とオーバーレイ（引き出し表示）の切り替えおよび幅制御。

## 3. 着手時点の計画 (Phase 9 — 洗練 & パリティ)

Phase 10（信頼性）を優先したため、この節は着手時点の計画として残す。

| # | 項目 | 内容 |
|---|---|---|
| P9.1 | Server 版ツールパリティ | Desktop の `symbols` / `git` 読取ツールを Docker/Server 版 (`mix_agent/tools/registry.py`) にも展開 |
| P9.2 | Composer @ 補完 | プロジェクトファイル一覧キャッシュに基づくインライン `@` ファイル補完 |
| P9.3 | モノレポ構造整備 | ルートと各 app 間の pnpm ワークスペース運用および CI 実行スクリプトの統一 |

## 4. 非スコープ (今回やらない)

- OS プロセス sandbox (macOS `sandbox-exec` / Windows Job / landlock)。[`docs/desktop/architecture.md`](architecture.md) のセキュリティ節の未実装宣言を維持し、敵対的コード実行には使わない前提を変えない
- embedding 検索 / FTS5 は Knowledge 側のみ未実装 (Memory は hybrid 検索済み、[`docs/desktop/architecture.md`](architecture.md) の「今回も未実装のもの」を参照)
- PTY 対話 terminal (stdin 送出)、`browser evaluate` の完全隔離
- 各 OS 実機での更新適用確認（アプリ内更新・署名付き `latest.json` の配布基盤は v0.5.2 で実装済み。実機確認結果は [`docs/verification.md`](../verification.md) に記録）
- Provider プリセット残2件 (`cloudflare` / `azure-ai-foundry`)、Responses API

## 5. 順序と検証

```
8.1 (Engine+edit/verify) → 8.2 (symbols+再検索) → 8.3 (並列+git読取) → 8.4 (UI)
```

- 各 Phase 毎に `pnpm rust:fmt:check` + `rust:clippy -- -D warnings` + `cargo test --workspace` + `pnpm ui:test` + `ui:typecheck`
- `core/tests/engine_loop.rs` に回帰テスト (plan 拒否 `1439-1490` の隣に追加)
- 実機 (Tauri/WebView) 目視・スクリーンリーダーは Phase 8.4 のみで実施 (既存「未確認項目」の扱いを踏襲)

## 6. リスク

- 並列化で budget/cancel 伝播が複雑化 → 8.3 を最後方にし、失敗時は逐次へ戻せる feature flag 化
- verify 自動化で無限ループ → `MAX_UNVERIFIED_REPAIR_TURNS=2` (`engine.rs:672`) を流用し打ち切り
- UI 肥大化でバンドル増 (既に 835 kB) → 8.4 は既存 `DiffView`/`ToolRow` 再利用、新規依存なし

## 7. Phase 10 — コーディングエージェントの信頼性

### 10.0 ツール契約の修復（完了 / 2026-10-28）

調査で、プロンプトとツール実装の名前が一致していないこと、plan モードが
自分が使うべきツールを拒否していることが判明した。機能追加より先に契約を直した。

- **プロンプトのツール名** (`core/src/prompt.rs`)。`outline` / `repo_map` /
  `files.read` は存在しない id だった。`symbols`（action `outline`）と
  `files`（action `read`）に修正し、探索には `grep` / `glob` を案内する。
- **分類の修正** (`core/src/runs/stagnation.rs`)。`symbols` を読み取りとして
  認識し、`todo` / `user_plan` を非 mutating に変更。これにより plan モードで
  `user_plan`（プロンプトが指示）と `todo`、`symbols` が実行できる。
  `git` は実際に公開している `status` / `diff` のみ読み取り扱い。
- **重複検知のキー**。`symbols` の outline が全て `symbols:` に潰れ、別ファイルの
  読み取りが同一と判定されていた。`git` / `workspace_check` も同様に action /
  scope / kind を含めるよう修正。
- **`user_plan` の権限** (`tools/src/user_plan.rs`)。提案の表示に承認プロンプトが
  出ないよう `is_sensitive = false`。説明文も「承認まで停止する」という未実装の
  約束を削除した（実際の停止は 10.2 で実装する）。
- **回帰テスト**。`core/tests/tool_contract.rs` が、合成されたシステムプロンプト内の
  バッククォート付きツール id をすべて registry と照合する。`stagnation` の単体
  テストと `engine_loop.rs` の plan モード E2E（`symbols` / `todo` / `user_plan`
  が通り、`edit` は拒否される）を追加。

検証: `cargo fmt --all -- --check` / `cargo clippy -p mixd-core -p mixd-tools
--all-targets -- -D warnings` / `cargo test -p mixd-core -p mixd-tools`（全緑）。

### 10.1 以降（計画）

| # | 項目 | 内容 |
|---|---|---|
| 10.1 | 評価ハーネス | 小規模 fixture repo と scripted provider で、修正・リファクタ・テスト修復・plan モード・拒否を採点 |
| 10.2 | 検証ループ | auto-verify の再アーム（上限付き）、format→lint→typecheck→test の段階実行、失敗出力の構造化 |
| 10.3 | 計画承認 | `user_plan` を本当に停止させ、Approve / Reject / Edit を UI と engine に実装 |
| 10.4 | ロールバック | 編集前スナップショットと「この run の変更を戻す」 |
| 10.5 | 探索 | `repo_map`、`symbols` の definition、grep の context、`@` 補完のインデックス化 |
| 10.6 | 性能・コスト | Anthropic の prompt cache、読み取りツールの並列化、UI の分割読み込み |
