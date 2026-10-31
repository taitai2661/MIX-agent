# Desktop コーディングエージェント評価ハーネス

`apps/desktop/core/tests/coding_eval.rs` は、小さな fixture リポジトリを実エンジン・
実ツールで解かせ、**ユーザーと同じ基準**で採点するための観測装置です。
`engine_loop.rs` が「1テスト = 1つの振る舞い」を検証するのに対し、こちらは
「1シナリオ = 1つのタスク完了」を検証します。

## 2つのモード

| モード | 実行 | Provider | 用途 |
| :--- | :--- | :--- | :--- |
| scripted | 通常の `cargo test`（CI 含む） | 固定スクリプト | エンジン／ツール／検証ゲートの回帰検出 |
| live | `--ignored` + `MIX_EVAL_*` | 実モデル（OpenAI 互換） | モデル比較、プロンプト・ツール変更の効果測定 |

scripted モードはモデルの賢さを測りません。直線的なタスクが必要以上に多くの
ターンやトークンを消費したときに落ちる、**性能の退行検出器**です。

## 実行方法

```sh
# scripted（CI）
pnpm desktop:eval
# または
cargo test --manifest-path apps/desktop/Cargo.toml -p mixd-core --test coding_eval -- --nocapture

# live（OpenAI 互換エンドポイント）
MIX_EVAL_BASE_URL=http://127.0.0.1:1234/v1 \
MIX_EVAL_MODEL=qwen2.5-coder-32b-instruct \
MIX_EVAL_API_KEY=optional \
  pnpm desktop:eval:live
```

live モードの環境変数:

| 変数 | 必須 | 内容 |
| :--- | :--- | :--- |
| `MIX_EVAL_BASE_URL` | はい | OpenAI 互換 API のベース URL |
| `MIX_EVAL_MODEL` | はい | モデル ID（ワイヤ上の名前） |
| `MIX_EVAL_API_KEY` | いいえ | API キー（ローカルランタイムでは不要なことが多い） |
| `MIX_EVAL_CONTEXT_WINDOW` | いいえ | コンテキスト長。0 / 未設定は不明として扱う |
| `MIX_EVAL_TIMEOUT_SECS` | いいえ | 1タスクあたりの上限（既定 180 秒） |
| `MIX_EVAL_MIN_PASS` | いいえ | 成功タスク数の下限。設定時のみ assert する |
| `MIX_EVAL_DEBUG` | いいえ | scripted 実行時にツール列とイベント列を stderr へ出す |

## 採点

各タスクは終了後に、**エージェントとは別プロセス**で `verify_command` を実行し、
期待する変更がファイルに残っているかを確認します。モデルの「テストが通りました」
という主張は証拠として扱いません。

記録する指標:

- `result` — 独立検証とファイル期待値の両方を満たしたか
- `steps` — Provider ターン数（予算消費の主因）
- `tools` — 実行されたツール呼び出し数
- `prompt_tok` — 全リクエストの system + messages を合計した推定トークン
  （4文字 = 1トークン。履歴の再送コストを見るための代理値）
- `wall_ms` — 実時間（マシン依存。合否には使わない）
- `claim` — 最終回答が成功を主張しているのに独立検証が失敗した回数

scripted モードの合否条件:

1. 3つのコーディングタスクがすべて成功する
2. 2つのガードレールが成功する
   - 検証失敗中に「完了」と言っても、エンジンが `verification_failed` で止める
   - plan モードの編集拒否がワークスペースを変えず、検証ゲートも arm しない
3. 各タスクが 12 ステップ以内
4. 3タスク合計の推定プロンプトが 20万トークン未満

## シナリオ

| 名前 | 内容 | 主に見るもの |
| :--- | :--- | :--- |
| `single_file_bugfix` | 1ファイルの算術バグを修正 | 最短経路の編集→検証 |
| `test_repair_after_failed_check` | 一度目の修正がテストで落ち、二度目で直す | 失敗出力を読んでの修復ループ |
| `multi_file_rename` | 2ファイルにまたがる関数名変更 | バッチ編集と複数パスの読み返し |
| `verification_gate_blocks_false_claim` | 直らない修正のまま成功を主張 | 検証ゲートの強制 |
| `plan_mode_refuses_edit` | plan モードで編集を試す | 読み取り専用ポリシー |

fixture は Python の `unittest` のみを使い、`python3 -B` で実行します
（`-B` は `.pyc` の再利用によるテスト結果の取り違えを防ぐため）。ハーネスは
Unix 限定です（`#![cfg(unix)]`）。

## タスクの追加

1. `Task` に fixture のファイル、`verify_command`、完了条件の部分文字列を書く
2. `Scenario` に scripted の軌跡（読み取り→編集→検証）を書く
3. live でも測りたいタスクは `Guardrail::None` にする
4. 期待ステップ数が増える場合は、まずエンジン側の無駄を疑う

## このハーネスが見つけた不具合

- バッチ `edit`（`edits` 配列）が検証台帳に `path:`（空パス）を登録し、
  どのファイルを読み返してもゲートが閉じなかった
- plan モードや権限で**拒否された**編集でも検証ゲートが arm され、
  何も変更していない Run が `verification_failed` で終わっていた

どちらも `core/src/runs/verification.rs` / `runs/engine/execute.rs` で修正済み。
