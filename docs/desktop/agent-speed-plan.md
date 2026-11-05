# MIX-agent Desktop — エージェント高速化計画（品質維持）

> 状態: 計画（未着手）
> 対象: `apps/desktop` の **Run 1 回の待ち時間**（Run Engine / Tools / Providers / UI のライブ表示）
> 前提: 速さのために検証ゲートやプロンプトの規律を外さない。品質は
> [`coding-eval.md`](coding-eval.md) の合格率・虚偽完了・ステップ数で判定する。
>
> **スコープ外**: 起動時間、初回描画、常駐 RSS、DB の保持期間、記憶検索の索引化は
> [`performance-plan.md`](performance-plan.md) が所有する。本計画はそこに乗る
> 「エージェント 1 ターンの速度」だけを扱い、重複する項目は参照に留める。

---

## 0. 結論 — 着手順

速度のボトルネックは「モデルが賢いか」より、**毎ターン運ぶトークン量**、**待ちの直列化**、
**やり直しの回数**にある。効果の大きい順に次を実施する。

| # | 施策 | 主な効果 | 期待削減 | リスク | 工数 |
| :-- | :--- | :--- | :--- | :--- | :--- |
| 1 | ツール出力の予算化（read / terminal / grep / IPC） | 入力トークンと読み込み時間 | プロンプト 25〜40% | 低（続きの読み方を返す） | 2〜3日 |
| 2 | プロンプト接頭辞の安定化（キャッシュ） | TTFT・入力課金 | 長い会話で TTFT 20〜50% | 中（プロンプト順序の変更） | 2日 |
| 3 | 検証・整形の段階化（LSP → 対象 → 全体） | 編集ステップの待ち時間 | 1ステップ 2〜30秒 | 中（検証漏れ） | 3〜4日 |
| 4 | 読み取り並列の拡張と重複排除 | ツール待ち時間 | 読み取り中心のターン 30〜50% | 低 | 2日 |
| 5 | 永続化・IPC をクリティカルパスから外す | ツールごとの固定費 | 5〜30ms/呼び出し | 低 | 1〜2日 |
| 6 | 取得（retrieval）のタイムアウト | Run 開始の待ち | 50〜200ms | 低 | 0.5日 |
| 7 | Auto ルーティングに実測速度を入れる | モデル選択 | 5〜15% | 中（品質） | 3日 |
| 8 | ローカルランタイムのウォームアップ・能力キャッシュ | 初回応答 | 3〜30秒 | 低 | 2日 |
| 9 | ストリーミング中の先回りツール実行 | ツール中心ターン | 10〜20% | 中（順序） | 3〜5日 |

最初の PR は **1 + 5 + 6**（品質への影響がほぼなく、計測もしやすい）にする。
2 と 3 は効果が大きい代わりにプロンプト／検証の契約に触れるため、評価ハーネスを
先に拡張してから入れる。

---

## 1. 現状のボトルネック（コード根拠）

### 1.1 1 ステップ = 1 モデル応答 + 全ツール完了

`core/src/runs/engine/execute.rs` のループは、`run_turn` がストリームを最後まで
読み切ってからツールを実行する。読み取りツールは
`flush_concurrent_reads`（`execute_tools.rs`）で最大 4 並列だが、**モデルの生成が
終わるまで 1 つも始まらない**。ツール呼び出しが多いターンほど、生成時間と
ツール時間が完全に直列になる。

### 1.2 1 回の読み取りが大きすぎる

`tools/src/files.rs`:

- `MAX_READ_BYTES = 256 * 1024`
- `DEFAULT_READ_LINES = 2_000`

大きいファイルを 1 回読むと 25〜60K トークン（4 文字 = 1 トークン換算）が
そのまま履歴に入る。`core/src/context.rs` の `TOOL_RESULT_CHAR_BUDGET = 60_000` を
1 件で超えるため、他のツール結果が追い出され、さらに次のターンでも同じ
トークンを再送する。**品質面でも、関係のない行がコンテキストを薄める。**

`tools/src/terminal.rs` も `MAX_OUTPUT_BYTES = 200 * 1024`、既定
`max_wall_clock_ms = 30_000`。失敗したテストの全ログがそのまま履歴に入る。

### 1.3 ツールイベントが大きい

`execute_tools.rs` の `tool_started` は `arguments` 全体、`tool_result` は
`result` 全体を Tauri イベントに載せる。`edit` の `new_string` や `terminal` の
200KB 出力が IPC・JSON シリアライズ・React の状態更新を通る。UI 側は
`runStore.ts` で 60ms バッチしているが、1 イベントの重さは残る。

### 1.4 永続化が待ち合わせになっている

`execute_tools.rs` はツール実行後に `record_tool_call` を await してから
`tool_result` を emit する。`execute.rs` はツールメッセージを 1 件ずつ
`append_message`（`query.rs` の INSERT）で書く。SQLite は WAL +
`synchronous=NORMAL`（`storage/src/db.rs`）ですでに速いが、**結果を UI に返す前に
書き込みを待つ必要はない**。

### 1.5 動的な head がキャッシュ接頭辞を壊す

`execute.rs` は毎ステップ、`<task_state>` + todo チェックリストを
`Message::text("system", head)` として **履歴の先頭**に置く。Anthropic は
system ロールのメッセージをトップレベルの `system` に畳み込む
（`providers/src/anthropic.rs` の `build_messages`）ため、todo が変わるたびに
system が変わり、**それ以前の会話のプレフィックスキャッシュが無効になる**。
OpenAI 互換の自動プレフィックスキャッシュでも同じ。

`cache_control` 自体は system / tools / 履歴末尾に付いている（10.6 で実装済み）が、
先頭が可変だと履歴部分が毎回作り直しになる。

### 1.6 検証と整形が毎ステップ走る

`execute.rs`:

- `step_mutated` のたびに `format_command` を `terminal` で実行（955 行付近）。
- 最初の変更後に `verify_command`（プロジェクト全体のチェック）を 1 回自動実行（1017 行付近）。
- 検証が閉じないと最大 `MAX_UNVERIFIED_REPAIR_TURNS = 2` 回、モデルターンを追加。

整形は「毎ステップ」、全体検証は「Run に 1 回」だが、どちらもコマンド次第で
数秒〜数十秒かかる。モノレポでは `cargo test` や `pnpm test` がそのまま走る。

### 1.7 探索の再計算

`tools/src/symbols.rs` の `map` は呼ぶたびに `build_repo_map` でリポジトリを
走査する。`grep` / `glob` も毎回 `ignore` ウォーカーを回す。会話が長いほど
同じ探索を繰り返しやすい。

### 1.8 取得（Memory / Knowledge）が Run 開始を待たせる

`execute.rs` は最初の Provider 呼び出しの前に `retrieve_all` を await する
（`retrieval.rs`）。メモリ行が増えると全件スコアリングのコストが増える。
索引化そのものは [`performance-plan.md`](performance-plan.md) の Phase 3.1 が所有する。
本計画では「遅い環境でも Run を待たせすぎない」タイムアウトだけを扱う。

### 1.9 ローカルランタイムの二度手間

`providers/src/ollama.rs` は thinking 非対応モデルに `think: true` を送った場合、
エラー後に 1 回だけ再試行する。**モデルごとに capability を覚えていない**ため、
同じモデルで毎ターンその往復が起き得る。モデルのロード待ち（初回）も
会話を開いた時点では温めていない。

### 1.10 Auto は速度を実測していない

`core/src/auto.rs` には `AutoPriority::Speed` と `speed_score` があるが、
`app/src/commands/auto.rs` は常に `AutoPriority::Balanced` を渡し、候補の
`speed_score` も静的なシグナル（ベンチマーク等）に依存している。実際の
TTFT / tokens/s を Run からフィードバックしていない。

---

## 2. 品質を落とさないための原則

1. **モデルを弱くしない。** 削るのは「無駄なトークン」「重複した探索」「待ちの直列化」。
2. **出力を削るときは続きを返す。** read は `offset`/`limit`、terminal は
   全文 artifact へのパス、grep は `truncated` フラグを必ず付ける。
3. **検証の最終ゲートは残す。** 段階化するのは途中の確認であり、
   「変更したのに全体チェックを成功させずに完了」は引き続き拒否する。
4. **変更はフラグ化する。** `AgentModePolicy` または設定で ON/OFF できるようにし、
   A/B とロールバックを可能にする。
5. **評価なしに既定値を変えない。** 既定値の変更は `coding_eval` の
   scripted + live の両方を通したものだけ。

### 品質ゲート（live 評価）

| 指標 | 条件 |
| :--- | :--- |
| 合格率 | baseline 以上（低下 0） |
| 虚偽完了（`claim`） | 0 を維持 |
| `steps` | baseline + 5% 以内 |
| `prompt_tok` | baseline − 20% 以上を目標 |
| `wall_ms` p50 / p95 | baseline − 20% / − 25% を目標 |
| 失敗時の復帰 | `test_repair_after_failed_check` を落とさない |

scripted モードはマシン非依存の退行検出器として CI に残す（時間はゲートにしない）。

---

## 3. Phase 0 — 計測とベースライン（2日）

### 3.1 位相別の計測を入れる

起動・描画の計測は [`performance-plan.md`](performance-plan.md) の Phase 0 が持つ。
ここでは **Run 1 回の中の位相**だけを追加する。

`RunPerf`（新規、`core/src/runs/perf.rs` など）に以下を記録する。

| 位相 | 計測点 |
| :--- | :--- |
| `retrieval` | `retrieve_all` の所要時間、注入文字数 |
| `compress` | `compress_with_budget` の所要時間、要約呼び出しの有無 |
| `provider_ttft` | `model_started` → 最初の `text` / `reasoning` / `tool_call` |
| `provider_total` | `run_turn` の総時間、入力・出力・cache read/write トークン |
| `tool_total` | `dispatch_tool` の合計、ツール別 |
| `format` / `verify` | 自動実行コマンドの所要時間と終了コード |
| `persist` | `append_message` / `record_tool_call` / event flush の合計 |
| `step` | 1 ステップの合計（モデル + ツール + 検証 + 保存） |

- Run 終了時に `runs.data.perf` へ 1 行だけ保存する（イベントは増やさない）。
- `MIX_PERF=1` のとき `tracing` に JSON で出す。UI の診断画面にも p50/p95 を出す。
- `coding_eval.rs` のレポートに位相別 p50/p95 と cache ヒット率を追加する。

### 3.2 ベースラインを取る

```sh
# scripted（CI・決定的）
pnpm desktop:eval

# live（モデル別）
MIX_EVAL_BASE_URL=... MIX_EVAL_MODEL=... MIX_EVAL_API_KEY=... \
  pnpm desktop:eval:live
```

モデルは最低 3 系統: 高速クラウド、高品質クラウド、ローカル（Ollama / LM Studio）。
各 3 回実行し、中央値と p95 を記録する。**この数値がない状態で最適化を始めない。**

2026-11-04 の scripted ベースライン（`pnpm desktop:eval`）:

| task | result | steps | tools | prompt_tok | wall_ms |
| :--- | :--- | --: | --: | --: | --: |
| `single_file_bugfix` | PASS | 4 | 3 | 2,404 | 709 |
| `test_repair_after_failed_check` | PASS | 8 | 6 | 9,998 | 214 |
| `multi_file_rename` | PASS | 6 | 5 | 4,406 | 144 |
| `verification_gate_blocks_false_claim` | PASS | 5 | 2 | 4,728 | 143 |
| `plan_mode_refuses_edit` | PASS | 3 | 2 | 1,750 | 3 |
| **合計** | 3/3 + guardrails 2/2 | 平均 5.2 | — | **23,286** | — |

scripted の `wall_ms` はモデル呼び出しがないため速度指標にならない。
速度の比較は live モードの `wall_ms` p50/p95 で行い、scripted は
`steps` / `prompt_tok` の退行検出に使う。特に
`test_repair_after_failed_check`（8 steps / 9,998 tokens）が最初の改善対象。

### 3.3 受け入れ基準の固定

上の品質ゲートを `docs/desktop/coding-eval.md` に追記し、PR テンプレートの
チェック項目にする。

---

## 4. Phase 1 — 即効・低リスク（1週間）

### 4.1 ツール出力の予算化

**read**（`tools/src/files.rs`）

- 既定を `DEFAULT_READ_LINES = 400` に下げ、新設の `DEFAULT_READ_BYTES = 48 * 1024` で
  バイト側からも切る。
- `MAX_READ_BYTES` は 128KiB に下げ、超えたら先頭 + 末尾だけ返す。
- 応答に `totalLines`, `returnedRange`, `truncated`, `nextOffset` を必ず含める。
- 既存の `fingerprint` は維持する（stale write 検出に必要）。

**terminal**（`tools/src/terminal.rs`）

- 既定の返却を 48KiB にし、超えた分は
  `<app-data>/artifacts/<run>/<call>.log` に保存してパスを返す。
- 失敗時は先頭 8KiB + 末尾 24KiB（スタックトレースの末尾を残す）。
- ANSI 除去は既存の `strip_terminal_escape_sequences` を使う。
- `max_wall_clock_ms` の既定 30 秒は維持。長いテストはモデルが明示的に延ばす。

**grep / glob**（`tools/src/search.rs`）

- 1 行 500 字 → 300 字。既定 `max_results` 100 → 60。
- `truncated` と「絞り込みの提案」を返す。

**イベント**（`execute_tools.rs`）

- `tool_started.arguments` は 2KiB に切り詰める（`edit` / `files write` は差分プレビュー）。
- `tool_result.result` は 8KiB に切り詰める。全文は DB と artifact に残し、
  モデル履歴は 4.1 の上限に従う。
- 承認カード用の `tool_approval_requested` は従来どおり全文（または差分）を渡す。

> IPC と DB 保存の上限（巨大 base64 / HTML を載せない）は
> [`performance-plan.md`](performance-plan.md) の Phase 3.4 が所有する。
> ここでの目的は **モデルに送る履歴そのもの**を小さくし、続きの読み方を返すこと。

**期待**: 長い Run の `prompt_tok` が 25〜40% 減り、TTFT と生成時間が比例して短くなる。
品質は「読み足りない」ではなく「続きを読む」導線で担保する。

### 4.2 プロンプト接頭辞の安定化

- `head`（`<task_state>` + todo）を履歴の先頭から外し、**そのステップの送信リクエストだけ**の
  末尾にユーザーメッセージとして付ける。`messages` には積まず、前ステップの head を
  置き換える（履歴に head を重複させない）。`compress` の保護対象は別途
  `request.head` として渡し、圧縮で落とさない。
- system / tools / 過去ターンをステップ間でバイト同一に保つ。
- Anthropic は動的 tail の **直前**に `cache_control` を置き、tail はキャッシュしない。
- `usage` の `cache_read_input_tokens` / `cache_write_input_tokens` を
  `RunPerf` に記録し、ヒット率を可視化する。
- 回帰テスト: 同じ履歴 + 変更した todo で 2 回リクエストを組み立て、
  先頭プレフィックスが一致することを検証する。

**期待**: 長い会話での TTFT 20〜50% 減、入力課金の削減。プロンプトの意味は変えない。

### 4.3 永続化をクリティカルパスから外す

- `dispatch_tool` は先に `tool_result` を emit し、`record_tool_call` は
  バックグラウンドの有界ライター（`mpsc` + 1 タスク）に投げる。
- `execute.rs` のツールメッセージ保存を 1 トランザクションにまとめる
  （`append_messages` を追加、既存の `append_message` は残す）。
- `run_turn` 完了時も、次のプロバイダー呼び出しを始める前に保存完了を待たない
  （Run 終了時とチェックポイント時に flush を保証する）。
- `query.rs` の INSERT は prepared statement を再利用する。

**期待**: ツール 1 回あたり 5〜30ms、多いターンで 100ms 前後。UI の詰まりも減る。

### 4.4 読み取り並列と重複排除

- `MAX_CONCURRENT_READS` を 4 → 8。ツール別のセマフォを入れる
  （`grep` は 2、`files read` は 8、`symbols map` は 1）。
- 同一ターン内で `read_key(name, args)` が同じ読み取りは 1 回だけ実行し、
  2 回目は「このターンで取得済み」として結果を再利用する。
- `symbols map` の結果を `(workspace, git HEAD)` で TTL 30 秒キャッシュし、
  `edit` / `files write` 成功時に無効化する。
- `skills_prompt` は既にキャッシュ済み（`tools/src/registry.rs`）なので触らない。

**期待**: 読み取り中心のターンでツール待ち 30〜50% 減。重複探索による
`RepeatedRead` の誤検知も減る。

### 4.5 取得のタイムアウト

- `retrieve_all` 全体を 150ms のタイムアウトで包み、間に合わなければ
  その時点で得られた上位 N 件（または空）で続行する。失敗は Run を止めない。
- 空クエリ・3 文字未満は retrieval をスキップする。
- FTS による候補絞り込み・索引化は [`performance-plan.md`](performance-plan.md) の
  Phase 3.1 で行う。ランキング契約（`memory-spec.md`）を変えるため、
  `scripts/check-memory-parity.py` と共有 fixture の更新とセットにする。

**期待**: Run 開始の 50〜200ms 短縮、メモリ件数が多い環境での突発的な待ちを防止。
ランキング結果は変えない。

---

## 5. Phase 2 — 検証・整形の段階化（1週間）

### 5.1 fast / full の 2 段階にする

`RunRequest` に `fast_verify_command: Option<String>` を追加する。

- **fast**: 変更ファイルに対する `lsp diagnostics`、または対象を絞ったテスト
  （例: `cargo test <module>`、`pytest <file>`、`vitest related`）。
- **full**: 既存の `verify_command`。完了前の最終ゲートとして必須。

ループの変更:

1. 変更を伴うステップの後は fast だけ実行する。
2. fast が失敗したら、その出力をモデルに返して修正させる（full は走らせない）。
3. fast が成功し、モデルが「完了」と言った時点で full を 1 回実行する。
4. full が失敗したら従来どおり修復ターン（最大 2 回）。
5. 最終的に full が成功していない Run は `verification_failed` で止める（現状維持）。

`fast_verify_command` が未設定なら従来どおり `verify_command` を 1 回だけ実行する
（後方互換）。

### 5.2 整形を差分に限定する

- `format_command` を「パスを受け取れるコマンド」と「プロジェクト全体のコマンド」に分ける。
- パス対応（`rustfmt`, `ruff format`, `prettier --write`, `gofmt`）は変更ファイルだけに適用。
- 同一 Run 内で同じファイルを 2 回整形しない。
- 整形の既定タイムアウトを 60 秒にし、失敗はモデルへのヒントに留める（完了は妨げない）。

### 5.3 全体検証の重複を避ける

- 変更がドキュメント・設定だけ（`*.md`, `*.json`, `*.toml` など）なら full をスキップできる
  ポリシーを追加する（既定 OFF、設定で ON）。
- `auto_verified` は「自動実行を試した」の意味のまま残し、**full の試行回数**を
  別カウンタで上限付きにする。失敗のたびに無制限に再実行しない。
- 検証失敗時の再取得（`retrieve_all`）は Run に 1 回だけにする。

**期待**: 編集 1 ステップあたり 2〜30 秒の短縮。最終ゲートは残るため品質は落ちない。

---

## 6. Phase 3 — モデル・ローカルランタイム（1週間）

### 6.1 Auto に実測速度を入れる

- `RunPerf` から `(provider_id, model_id)` ごとの TTFT / tokens/s を EWMA で保存する。
- `auto.rs` の `speed_score` に実測値を反映する（静的なベンチマークより優先）。
- 設定に `AutoPriority`（`speed` / `balanced` / `quality`）を出し、
  `app/src/commands/auto.rs` の固定 `Balanced` を設定値にする。
- 既定は `balanced`。`speed` はユーザーが明示したときだけ。

### 6.2 推論量の段階制御（任意設定）

- `ReasoningControl` を Run 全体で固定せず、「計画・編集」は要求値、
  「ツール結果を受けただけの継続ターン」は 1 段下げる選択肢を追加する。
- 既定 OFF。`speed` 優先のときだけ ON。live 評価で品質低下がないことを確認する。

### 6.3 ローカルランタイム

- Ollama の thinking 非対応を `(base_url, model)` でキャッシュし、再試行を 1 回に固定する。
- 会話を開いたとき / Run 開始時に `keep_alive` 付きの軽い prewarm を行う（設定で OFF 可）。
- `num_ctx` は 8K 固定ではなく、モデルの公称値とプロンプト長から決める
  （小さすぎると圧縮が頻発し、大きすぎるとロードが遅い）。
- LSP サーバーはワークスペースを開いた時点で prewarm する（`tools/src/lsp.rs` は
  `(workspace, language)` でセッションを再利用済み）。

**期待**: ローカル初回応答 3〜30 秒短縮、クラウドはルーティングで 5〜15%。

---

## 7. Phase 4 — ストリーミング・パイプライン（1週間）

### 7.1 ツール呼び出しの完了を早く知る

各アダプタに「この tool call の引数が揃った」イベントを追加する。

- Anthropic: `content_block_stop`（`tool_use` ブロック）。
- OpenAI 互換: `finish_reason == "tool_calls"` の直前、引数 JSON が valid になった時点
  （安全のため `finish_reason` まで待つフォールバックも用意）。
- Ollama / Gemini: 各ストリームの tool call 完了通知。

### 7.2 読み取りだけ先に走らせる

- 完了した読み取り専用 call を、モデルが残りの call を生成している間に dispatch する。
- 結果は `index` で保持し、現在の「呼び出し順で再構成」する方式を維持する。
- 最初の mutating call の直前で必ず flush する（既存の `flush_concurrent_reads` と同じ保証）。
- キャンセル・承認・検証の意味は変えない。

**期待**: ツール中心のターンで 10〜20%。効果はモデルの tool call の出し方に依存するため、
Phase 0 の計測で `provider_ttft` と `tool_total` の比率を見てから着手する。

---

## 8. Phase 5 — ロールアウト

### 8.1 フラグ

`AgentModePolicy` に追加する（既定は安全側）。

```rust
pub struct AgentModePolicy {
    // 既存…
    pub tool_output_budget: bool,        // 4.1
    pub stable_prompt_prefix: bool,      // 4.2
    pub async_persistence: bool,         // 4.3
    pub parallel_read_limit: u8,         // 4.4（既定 8）
    pub tiered_verification: bool,       // 5.1
    pub format_changed_paths_only: bool, // 5.2
}
```

設定 UI は Phase 2 以降で「設定 ▸ 一般」のエージェント項目に追加する。Phase 1 は
環境変数（`MIX_AGENT_*`）または設定ファイルのフラグで A/B できるようにし、既定は
Phase 1 のものだけ ON、Phase 2 以降は評価が通ってから ON にする。

### 8.2 スケジュール

| 週 | 内容 | 完了条件 |
| :-- | :--- | :--- |
| 1 | Phase 0 + 4.1 + 4.3 | scripted 緑、live で prompt_tok −20% |
| 2 | 4.2 + 4.4 + 4.5 | live で TTFT −20%、合格率維持 |
| 3 | Phase 2 | 編集タスクの wall_ms −20%、検証失敗の見逃し 0 |
| 4 | Phase 3 + 7.1 | ローカル初回応答改善、Auto の品質維持 |
| 5 | 7.2 + 仕上げ | ツール中心ターン −10%、全ゲート緑 |

### 8.3 ロールバック

各フラグを OFF に戻すだけで従来動作に戻ること。`RunRequest` の追加フィールドは
`Option` / 既定値付きにし、古い呼び出し元（テスト・スケジュール・サブエージェント）が
壊れないようにする。

---

## 9. やらないこと

- **検証ゲートの撤廃。** 速くなるが「動かないのに完了」が増える。
- **プロンプトから規律（読んでから直す、検証してから完了）を削る。** 手戻りが増え、
  結局遅くなる。
- **無条件の `max_tokens` 削減。** 出力が途中で切れ、再試行で遅くなる。
- **mutating ツールの並列実行。** 同一ファイルの競合と検証順序が壊れる。
- **ツール結果の黙った切り捨て。** 必ず `truncated` と続きの取得方法を返す。
- **モデルを速いものに固定するだけの最適化。** 品質ゲート（合格率・虚偽完了）を
  通らない場合は採用しない。

---

## 10. 検証コマンド

```sh
# Rust
cargo fmt --manifest-path apps/desktop/Cargo.toml --all -- --check
cargo clippy --manifest-path apps/desktop/Cargo.toml -p mixd-core -p mixd-tools --all-targets -- -D warnings
cargo test --manifest-path apps/desktop/Cargo.toml -p mixd-core -p mixd-tools

# 評価
pnpm desktop:eval
MIX_EVAL_BASE_URL=... MIX_EVAL_MODEL=... pnpm desktop:eval:live

# UI
pnpm --dir apps/desktop/ui test
pnpm --dir apps/desktop/ui typecheck
pnpm --dir apps/desktop/ui lint
```

追加する回帰テスト:

- `files read` の切り詰めと `nextOffset` の往復。
- `terminal` の先頭 + 末尾返却と artifact パス。
- 同一ターン内の重複読み取りが 1 回だけ dispatch されること。
- todo を変えてもプロンプトの先頭プレフィックスが一致すること（キャッシュ）。
- `fast_verify_command` 成功 → full 1 回、失敗 → full を走らせないこと。
- `tool_result` イベントが上限を超えないこと。
- 非同期永続化の flush が Run 終了時に完了していること。

---

## 11. 最初の PR（スコープを切った実装単位）

1. `RunPerf` の追加と `coding_eval` レポートの拡張（Phase 0 の一部）。
2. `files` / `terminal` / `grep` の出力予算と `nextOffset` / artifact。
3. `tool_started` / `tool_result` イベントの上限。
4. `record_tool_call` のバックグラウンド化とツールメッセージの一括 INSERT。
5. `retrieve_all` の 150ms タイムアウト（索引化は performance-plan 側）。

この 5 つはプロンプトの意味と検証契約を変えないため、live 評価の合格率を
落とさずに `prompt_tok` と `wall_ms` を下げられる見込みが高い。

---

## 12. 関連計画

- [`performance-plan.md`](performance-plan.md) — 起動・描画・常駐・検索索引化。
  本計画の 4.5 と 4.1 の IPC 部分はそちらと重ならない範囲だけを持つ。
- [`coding-agent-plan.md`](coding-agent-plan.md) — エージェント機能の Phase 8〜10。
  10.6（prompt cache / 並列読み取り / steering）の延長として本計画を位置づける。
- [`coding-eval.md`](coding-eval.md) — 品質と速度の測定方法。
