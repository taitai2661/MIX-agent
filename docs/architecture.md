# アーキテクチャ

## 境界

`apps/server/mix_agent` は業務ロジックをまとめるモジュラーモノリスです。Provider Adapter、Tool Registry、Permission、Run Engineを分離しています。

```text
React → REST / SSE → FastAPI → PostgreSQL
                           → Provider Adapter
                           → Run Engine → Tool Registry → Permission → Executor
                                                                     ├ 本体（Memory、Web）
                                                                     ├ execution-runner
                                                                     └ mcp-runner
Runner → egress-proxy → 許可された公開接続先
```

FastAPIは1プロセスで起動します。メモリ内のTask管理・承認の排他は単一プロセス前提です。複数Workerへの変更にはDBによるジョブleaseと分散排他の追加が必要です。

## DB

ユーザー、Session、Run、Event、ToolCall、Approval、RunCheckpointは専用の構造化カラムを持ちます。Provider、Model、Agent、Tool、MCP、Memory等は各専用テーブル内のJSONBに拡張可能な設定を保存します。AgentのTool選択は `agents.data.tool_ids` に格納し、初期版では別の中間テーブルを持ちません。

RunとMessageはConversationに、Event/ToolCall/Approval/RunCheckpointはRunに関連付けます。会話単位の有効なRunは部分ユニークIndexにより1件に制限します。EventはRun単位の連番です。RunCheckpointはAgentモードの長時間実行向けに定期保存されるスナップショットで、Resume APIから任意のチェックポイントへの選択的再開が可能です。

MigrationはAlembicで管理し、PostgreSQL専用です。Memory本文にはpg_trgm Indexを作成し、部分一致検索を行います。

## Provider

`Adapter.stream()` は本文、公開Reasoning、最終応答・Tool Callを共通イベントへ変換します。署名・暗号化Reasoning・Tool IDなどはRun内のProvider固有履歴に保持し、UIには公開しません。モデルをまたぐ新しい会話ターンには本文の履歴を再構築します。

Capabilityは真偽/不明の3状態。手動Overrideを優先します。自動取得が失敗したモデルは消しません。モデル一覧は有料推論を実行しません。実モデルによるReasoning・Vision等の受入確認は別途必要です。

## 実行状態

`queued → running → waiting_approval → running → completed` が通常の経路です。`failed`、`cancelled`、`interrupted`、`budget_extension_pending` を別に保持します。SSE購読でRunを開始することはありません。

`budget_extension_pending` はAgentモードで予算上限に近づいたときにエンジンが一時停止し、ユーザーに延長確認を求めるための状態。承認で `running` へ、拒否またはタイムアウトで `completed` へ遷移します。

Tool Callは実行前に`executing`をcommitします。再起動後に結果不明になった操作は自動再実行しません。ユーザー確認後の再開では「結果不明」のTool結果をモデルへ渡します。Resume APIは `from_checkpoint` パラメータで任意のチェックポイントから再開でき、`unknown_actions` で `executing` のまま残った各Tool Callを `retry` / `discard` / `mark_unknown` 単位で制御します。外部APIに対するexactly-once実行の保証はありません。

Toolの引数Schema、選択Tool、Scope、現在の権限、定義Fingerprintを実行直前に確認します。実行中の副作用操作は本体で直列化します。固定Runner内のバックグラウンドProcessや外部MCPの副作用は独立して継続し得ます。

## 長時間実行の自律性（Agent モード）

Agentモード（`mode_policy.agent`）は次のポリシーフラグで実行ループを制御します：

- `checkpointing` — 中間チェックポイントを `policy.checkpoint_every_steps` ごとに保存（既定10ステップ）。最新20件を保持
- `stagnation_detection` — 直近12 Tool Callから繰り返し失敗・読み取り停滞・Provider不安定を検知し、ステップ毎に user-role で修復ガイダンスを挿入
- `strict_verification` — `update_plan` の `phase:<name>` 区切りごとに検証を要求、削除後に `files_list` / `search_files` 確認、`write_file` 上書き前の `read_file`、破壊的ターミナル操作後の check Tool を必須化
- `budget_extension` — ステップまたはツール上限到達時にユーザーへ延長確認（既定2回まで）。確認なしで停止しないよう UX を強化
- `background_processes` / `persistent_browser` — 既存通り

階層的要約（`context.tiered_summary`）は active サマリ（4000字まで）と直前の archived rows（最大12件）を分けて管理し、古い会話が要約されても task_state（goal/plan/pending/important_facts/artifacts/open_questions）を head ブロックで優先保持します。

古い会話ターンだけを要約してContextを圧縮します。現在のTool Call/Resultや署名付きブロックは切断しません。現在のターンだけで上限を超えた場合は停止し、新しい会話での続行を促します。要約呼び出しもStepに数えます。要約APIが失敗した場合は古い会話を破棄せず Run を継続し、次の compaction 機会で再試行します（Tier 2 fallback）。

## セキュリティと拡張

AES-GCMでSecretを保存し、用途を追加認証データに含めます。CookieはHttpOnly/SameSite Strict、変更APIはCSRF TokenとOriginを検証します。HTTPS設定時にはSecure Cookieを使用します。

独自APIは管理者Session向けです。外部アプリ向けBearer API KeyやOpenAI互換APIはまだありません。Knowledge、OpenAPI Tool、Pluginの実行口は将来の拡張対象です。

参考仕様: [OpenAI Reasoning](https://developers.openai.com/api/docs/guides/reasoning)、[MCP](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports)、[Docker Security](https://docs.docker.com/engine/security/)。
