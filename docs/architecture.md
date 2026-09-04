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

ユーザー、Session、Run、Event、ToolCall、Approvalは専用の構造化カラムを持ちます。Provider、Model、Agent、Tool、MCP、Memory等は各専用テーブル内のJSONBに拡張可能な設定を保存します。AgentのTool選択は `agents.data.tool_ids` に格納し、初期版では別の中間テーブルを持ちません。

RunとMessageはConversationに、Event/ToolCall/ApprovalはRunに関連付けます。会話単位の有効なRunは部分ユニークIndexにより1件に制限します。EventはRun単位の連番です。

MigrationはAlembicで管理し、PostgreSQL専用です。Memory本文にはpg_trgm Indexを作成し、部分一致検索を行います。

## Provider

`Adapter.stream()` は本文、公開Reasoning、最終応答・Tool Callを共通イベントへ変換します。署名・暗号化Reasoning・Tool IDなどはRun内のProvider固有履歴に保持し、UIには公開しません。モデルをまたぐ新しい会話ターンには本文の履歴を再構築します。

Capabilityは真偽/不明の3状態。手動Overrideを優先します。自動取得が失敗したモデルは消しません。モデル一覧は有料推論を実行しません。実モデルによるReasoning・Vision等の受入確認は別途必要です。

## 実行状態

`queued → running → waiting_approval → running → completed` が通常の経路です。`failed`、`cancelled`、`interrupted` を別に保持します。SSE購読でRunを開始することはありません。

Tool Callは実行前に`executing`をcommitします。再起動後に結果不明になった操作は自動再実行しません。ユーザー確認後の再開では「結果不明」のTool結果をモデルへ渡します。外部APIに対するexactly-once実行の保証はありません。

Toolの引数Schema、選択Tool、Scope、現在の権限、定義Fingerprintを実行直前に確認します。実行中の副作用操作は本体で直列化します。固定Runner内のバックグラウンドProcessや外部MCPの副作用は独立して継続し得ます。

古い会話ターンだけを要約してContextを圧縮します。現在のTool Call/Resultや署名付きブロックは切断しません。現在のターンだけで上限を超えた場合は停止し、新しい会話での続行を促します。要約呼び出しもStepに数えます。

## セキュリティと拡張

AES-GCMでSecretを保存し、用途を追加認証データに含めます。CookieはHttpOnly/SameSite Strict、変更APIはCSRF TokenとOriginを検証します。HTTPS設定時にはSecure Cookieを使用します。

独自APIは管理者Session向けです。外部アプリ向けBearer API KeyやOpenAI互換APIはまだありません。Knowledge、OpenAPI Tool、Pluginの実行口は将来の拡張対象です。

参考仕様: [OpenAI Reasoning](https://developers.openai.com/api/docs/guides/reasoning)、[MCP](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports)、[Docker Security](https://docs.docker.com/engine/security/)。
