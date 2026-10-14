# MIX-agent Desktop — コーディングエージェント化計画

> 状態: 計画のみ。実装なし。Phase 0-6 完了、Phase 7 未着手を前提とする。

## 1. 目的

現状の `chat / thinking / plan / agent` + Run Engine を、Claude Code / OpenCode 級の
コーディングエージェントにする。定義は以下を自律的に回せること:

1. 探索 (repo 把握) → 計画 (`user_plan`) → 複数ファイル編集 → 実行確認 (test/lint/typecheck) → 修正ループ → 報告
2. 失敗時に止まらず、検証・差分・チェックポイントで復帰できる
3. 権限ゲート (`tools/src/permission.rs`, `tools/src/execute.rs`) を壊さない

## 2. 現状 (2026-10-07 時点)

### 2.1 あるもの

- Run Engine ループ (`core/src/runs/engine.rs:857-1617`): budget、階層圧縮 (`core/src/context.rs`)、停滞検知 (`core/src/runs/stagnation.rs`)、検証台帳 (`core/src/runs/verification.rs`)、10 step 毎 checkpoint (`core/src/runs/checkpoint_store.rs`)
- Mode 4 種 (`core/src/mode.rs:4-88`): `chat 12/12/20m` / `thinking 24/24/45m` / `plan 60/150/45m` / `agent 300/750/90m`。`plan` は読取専用強制 (`core/src/runs/engine.rs:1461-1478`)
- Tools: `files` (offset/limit ページング、`tools/src/files.rs:202-251`)、`edit` (単一アンカー置換、`tools/src/edit.rs:46-86`)、`glob`/`grep` (`.gitignore` 準拠、`tools/src/search.rs`)、`terminal` (PTY 永続シェル + sentinel、`tools/src/terminal.rs:185-199`)、`todo` (per-run、`tools/src/todo.rs`)、`skill`、`agent` (単発委譲、`tools/src/agent.rs:147-212`)、`web_search`/`web_fetch`/`network` (egress + private IP deny)
- UI: 差分プレビュー (`ui/src/features/chat/DiffView.tsx`)、承認カード (`ui/src/features/chat/Transcript.tsx:626` + `ui/src/features/tools/ApprovalsPane.tsx`)、todo/検証表示 (`ui/src/features/chat/ActivityPanel.tsx`)、git 変更パネル (`ui/src/features/files/ChangesPanel.tsx`)

### 2.2 ないもの (コーディングエージェント差分)

| # | 不足 | 根拠 |
| --- | --- | --- |
| E1 | `edit` が 1-call 1-anchor のみ。multi-edit / apply_patch (複数 hunk 一括、fuzzy) なし | `tools/src/edit.rs:232-257`、`preview_of:164-187` は前後3行表示のみ |
| E2 | 検証が受動的。`verify_command`/`format_command` をモデル任せで Engine が実行しない | `core/src/runs/verification.rs:92-102` は `note_command` のみ、`engine.rs:1341-1357` はヒント文 |
| E3 | 探索が `glob/grep/read` のみ。LSP (定義ジャンプ/diagnostics/rename)、repo_map/outline 実体なし | `tools/src/registry.rs:61-99` に登録なし、`core/src/runs/stagnation.rs:203,216` が参照のみ |
| E4 | `agent` 委譲が逐次直列。並列 fan-out なし | `core/src/runs/engine.rs:1424` の `for await`、`tools/src/agent.rs:32-41` |
| E5 | git が first-class でない。`terminal git ...` 経由のみ、commit/PR 支援なし | `app/src/commands/workspace.rs:194,268` は参照系のみ |
| E6 | retrieval が Run 先頭1回のみで turn 中再検索なし。Memory は hybrid（lexical + embedding）実装済みだが、Knowledge は lexical のみ | `core/src/retrieval.rs:245-429`（`retrieve_memories_hybrid`）、`core/src/runs/engine.rs:899-914` |
| U1 | 承認時に差分を見せない (`JSON.stringify(arguments)`)、Accept/Reject/部分適用なし | `ui/src/features/chat/Transcript.tsx:651`、`ui/src/lib/runEvents.ts:279` |
| U2 | terminal 出力が JSON ダンプ (4000字切、折畳みのみ)。exit code/duration/ANSI/コピー/仮想化なし | `ui/src/features/chat/ActivityPanel.tsx:167`、`ui/src/lib/runEvents.ts:316` |
| U3 | plan/todo が右パネル埋没。チャット内インライン進捗・`user_plan` 全文カードなし | `ui/src/features/chat/ActivityPanel.tsx:188`、`ui/src/lib/runEvents.ts:253,329` |
| U4 | 履歴に作業証跡が残らない (ツール実行は Badge のみで後から見返せない) | `ui/src/features/chat/Transcript.tsx:266` |

## 3. 計画

### Phase 8.1 — 編集実行の最小一式 (P0)

- `edit` に `replace_all` 済みの上で `multi_edit` (順序付き複数アンカー、atomic、失敗時全 rollback) を追加。既存 `replace` の曖昧拒否 (`tools/src/edit.rs:77-79`)・`NoOp` 拒否は維持
- `terminal` の成功パスに `verify` フック: `RunRequest` の `verify_command` を Engine が `terminal` 経由で自動実行し、`VerificationLedger::note_command` (`core/src/runs/verification.rs:92-102`) で閉じる。モデル任せヒント (`engine.rs:1341-1357`) は残し二重実行を避けるため `verified` 済みはスキップ
- `files.write` 全文置換 + `edit` の `atomic_write` (`tools/src/edit.rs:202-216`) を維持し、破壊的判定 (`core/src/runs/verification.rs:242-284`) を拡張しない (誤検知を増やさない)

受け入れ: `cargo test --workspace` に multi_edit 原子性 + verify 自動閉鎖のケース追加、`pnpm ui:test` の既存ケース維持（件数は `pnpm ui:test` の出力で確認）。

### Phase 8.2 — 探索強化 (P0-P1)

- `symbols` ツール (正規表現ではなく構造探索): `grep` (`tools/src/search.rs:267-421`) の上位互換ではなく、① `outline <file>` (関数/型一覧)、② `references <symbol>` (grep 薄ラッパー + 除外共有) の2本に絞る。LSP フル実装はしない
- retrieval の turn 中再検索: 現状先頭1回 (`engine.rs:899-914`) に加え、停滞 (`stagnation_detected`)・検証失敗 (`verification_failed`) 時に `retrieval::retrieve_all` を再実行。embedding/FTS5 は非スコープ (lexical + CJK unigram/bigram 維持、`core/src/retrieval.rs:162-234`)
- `terminal` 長時間コマンドのポーリング表示 (現状 `MAX 600s` clamp、`tools/src/terminal.rs:368-377` を維持し UI 側で streaming 化はしない)

受け入れ: 大規模 repo で `glob → outline → read(offset/limit)` の3手で目的関数へ到達できることを手動確認。

### Phase 8.3 — 並列委譲 + git (P1)

- `agent` 並列: `engine.rs:1424` の逐次 `for` を、同ターン複数 `agent` 呼び出しのみ `join` 並列化。予算は親から分割譲渡し、`DelegateResult` (`tools/src/agent.rs:32-41`) の形状は変えない。再帰禁止 (`app/src/lib.rs:246-257`)・`TodoStore` per-run (`tools/src/todo.rs:42-52`) は維持
- git first-class (最小): `git_status` / `git_diff` (既存 `app/src/commands/workspace.rs` 参照系) を tool 化し、書込系は `terminal` 経由のまま。commit/PR 作成は非スコープ (`prompt.rs:35` の commit/push 抑止と整合)

受け入れ: 同ターン 2 `agent` 並列の `engine_loop.rs` テスト追加。git 読取が permission `allow` (読取既定) で通ること。

### Phase 8.4 — UI 証跡 (P1-P2)

- U1: `ApprovalCard` (`Transcript.tsx:626`) に `DiffView` (`DiffView.tsx:89`) を埋め込み、差分を見て許可できる。Accept/Reject は permission 決定 (`decideApproval`) のみで部分適用はしない
- U2: terminal 結果に `exit code / duration` バッジ + コピーボタン + `pre` 折返し。ANSI 色・仮想化はしない (`ActivityPanel.tsx:167` の `ToolRow` 拡張に留める)
- U3/U4: `todo_list`/`user_plan` を Transcript インラインに最小表示 (進捗 `done/total` + 現在ステップ)。履歴永続 (`MessageItem`) にツール要約1行を残す。右パネル排他 (`RightPanel.tsx`) は変えない

受け入れ: `pnpm ui:typecheck` + `ui:lint --max-warnings 0` + `ui:test` 成功。1440/1024/820px 横スクロールなし (既存確認基準を踏襲)。

## 4. 非スコープ (今回やらない)

- OS プロセス sandbox (macOS `sandbox-exec` / Windows Job / landlock)。`docs/architecture.md:122` の未実装宣言を維持し、敵対的コード実行には使わない前提を変えない
- embedding 検索 / FTS5 は Knowledge 側のみ未実装 (Memory は hybrid 検索済み、`docs/architecture.md` の「今回も未実装のもの」を参照)
- PTY 対話 terminal (stdin 送出)、`browser evaluate` の完全隔離
- auto-update 有効化 (`tauri.conf.json: active=false` のまま)、署名・`latest.json` 基盤
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
