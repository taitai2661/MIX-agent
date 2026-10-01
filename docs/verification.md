# 検証記録

このファイルは時点ごとの検証記録です。**新の日付の節を先頭に足す**ことで
時系列が保たれます。節の記載は当時のまま残しますが、環境や仕様が変わった
箇所には `[追記]` を付けて明示してください。実 Provider・実 Docker・実機で
未実施の項目を、単体テストやビルド成功で置き換えて「動作確認済み」と
扱わないでください。

現在の日付順（新しい順）: 2026-10-01 → 2026-09-26 → 2026-09-23 → 2026-08-30 → 初期実装

## v0.3.5リリース候補の確認（2026-10-01）

確認した項目:

- 添付Dockerソースから、Memory Runtime、Projects、Todo、Skills Discovery／パッケージ、RunCheckpoint、Browser pause、Usage、PWAの実装を取り込み。
- Python compileall、JSON解析、YAML解析、`git diff --check`、`uv lock --check` が成功。
- WebのVitest 13ファイル58テストが成功。TypeScript／Vite production buildも成功。
- v0.3.5をPython、Web、MCP ClientInfo、OpenAPI、ZimaOS Compose、8種類のGHCRイメージ参照へ反映。
- ZimaOS Compose生成スクリプトをLinuxの大文字小文字に整合する`Apps/MIX-agent`出力へ修正し、v0.3.5のCompose生成・YAML解析が成功。

未確認の項目:

- SandboxにはDocker CLIがないため、PostgreSQLを含むCompose起動、migration適用、全イメージのビルド、Runner境界、実Provider／実MCPの受入確認は未実施。
- バックエンドpytestはテスト用DBホスト`postgres-test`への接続が必要で、Docker未導入のためDB接続前に失敗。単体テストが不合格だったという意味ではなく、実行基盤不足による未実施扱い。
- Web buildでは500KB超のchunk警告が出たが、build自体は成功。

## Agent 実行ループと自律性の強化（2026-09-26）

長時間作業向けに新しい状態・データ・APIを追加し、5テーマ（文脈圧縮、停滞検知、動的予算、中間チェックポイント、検証ループ厳密化）をまとめて導入しました。実装にはAlembic migration `0020_run_checkpoints.py` が必要で、`runs` テーブルの CHECK 制約に `budget_extension_pending` を追加し、新規 `run_checkpoints` テーブルを作成します。

確認した項目:

- バックエンドのPython: 21件のVitest相当（`test_tiered_summary`, `test_stagnation_and_verification`, `test_agent_completion`, `test_mode_policy`）が成功。`test_run_checkpoints` / `test_budget_extension_state` は PostgreSQL が必要で本環境では未実行。
- TypeScript / Vite build が成功。Vitest 11ファイル52テストが成功（`types.test.ts` に新イベントの parse テストを追加）。
- `ruff check` で `mix_agent/runs/{checkpoints,stagnation,verification,events}.py` と `mix_agent/context/tiered_summary.py` と `mix_agent/api/schemas.py` はすべて通過。`mix_agent/runs/engine.py` と `mix_agent/api/routes.py` の残存警告は既存の `except Exception: pass` パターンの踏襲によるもので新機能由来ではない。
- `apps/server/migrations/versions/0020_run_checkpoints.py` をマイグレーション一覧へ追加。`RunCheckpoint` モデル・`budget_extension_pending` 状態・`one_active_run` 部分Indexが新CHECK制約と整合。［追記］この時点では末尾の `0020` でしたが、以降は `0021_memory_role_taxonomy.py` が追加されています。
- 公開API: `POST /runs/{key}/resume` の `from_checkpoint` / `unknown_actions` 拡張、`POST /runs/{key}/budget-extension`、`GET /runs/{key}/checkpoints`、`GET /runs/{key}` の新フィールド（`budget_extension_request` / `budget_extensions_used` / `budget_extensions_max` / `stagnation` / `checkpoint_resumed_from` / `checkpoints`）。
- フロントエンドの `parseRunEvent` が新イベント `checkpoint_saved` / `budget_extension_requested` / `budget_extension_resolved` / `stagnation_detected` / `verification_required` / `phase_advanced` をすべて型安全に解釈。

未確認の項目:

- 実Provider・実Runner・実Dockerでの中間チェックポイント復元動作、予算延長フローのUX、停滞検知の実会話での精度検証。
- 階層的サマリの Tier 2 fallback が、巨大Tool出力の繰り返しで正しく発火するかどうかの負荷試験。
- Agentモード以外のモード（Chat / Thinking）で `RunView` の新フィールドが UI にどう露出するかの最終デザイン整合。

これは初期実装の検証記録であり、本番運用の認定ではありません。

## 現行コードとの差分と確認範囲（2026-09-23）

この節と 2026-08-30 の節の件数・画面確認は当時の記録です。現行コードにはKnowledgeのテキスト検索Tool、Skill、定期実行、Remote MCP OAuth、Context圧縮、一時モードが追加されています。実装の存在は実Provider・実Docker・実機での受入確認を意味しません。

会話のMarkdown書き出しは回答のみ・会話・Tool実行概要を選択できます。Tool概要には引数と結果本文を含めません。Context圧縮の要約と件数はRunに保存して画面へ表示します。要約失敗時は履歴を破棄せず停止します。Runの回答チェックは最終回答の形式と有無を判定し、内容の正確さまでは判定しません。一時モードは保存対象とTool許可状態を送信欄に表示します。

キーボード操作ではモデル選択のEscapeによる復帰、チェックボックスのEnter操作、IME確定中の誤送信を点検・修正しました。スクリーンリーダーとスマートフォン実機の受入確認は未実施です。

今回の確認: WebのTypeScript/Vite buildとVitest 22件、Contextの対象テスト14件、Composeの使い捨てPostgreSQLで書き出し・回答チェックの対象テスト4件が成功。画面の実操作とスクリーンリーダーでの確認は未実施です。一時モードの添付は選択時にアップロードされるため、送信せず画面を離れた場合の孤立添付を自動削除する処理は今後の課題です。

## Chat / Thinking拡張の確認（2026-08-30）

- Chatの自動思考、ThinkingのネイティブReasoning／通常推論フォールバック、プリセットの空リストを含むツール制限をテスト。
- OpenAI Responses / OpenRouter / AnthropicはHTTPモックと実SDKで送信設定・公開要約・ツール継続を確認。GeminiはSDK型を使うクライアントモックで確認。Claudeのadaptive / budget、Geminiのlevel / budgetの両方式を含む。
- Chat / Thinkingの承認待ち・再開・拒否、3 / 5ステップ上限、停止、ツール失敗時の安全なエラー返却をテスト。既存の認証・保存・バックアップ等の回帰テストも実施。
- Vitestは合計6件。モードの利用可否、明示的な不明Override、空のプリセット、要約表示と既存APIクライアントを確認。TypeScript / Vite build成功。
- 非対応ReasoningモデルのThinking通常推論フォールバック、初回Tool Callingプローブ、結果保存、手動再確認はHTTPモックの統合テストで確認。実Providerとブラウザでの受入確認は未実施。
- 390×844のviewportで思考要約・モード選択・入力欄を目視確認。実機キーボードは未確認。

今回の画面検証は `tests/fixtures/chat_preview.py` を使用。Compose profile `preview` が起動ごとに使い捨てPostgreSQLを作り、Providerはローカルの固定応答、ツール実行はモックに置き換える。既存データ、実AI、実ファイル操作、実コマンド、実Web検索は使用しない。実Provider各接続方式の課金・応答・思考品質と実Runnerの操作は未検証。

全体Lintには既存のFastAPI Depends / wildcard import等の指摘が残る。Python未定義名チェックと、新規の思考解決・回帰テストファイルのLintは成功。

［追記］当時の記録では「Git管理外のフォルダのため `git diff --check` は実行できない」としていましたが、このディレクトリは現在は Git 管理下にあり実行可能です。`CONTRIBUTING.md` に記載の手順どおり `git diff --check` を通してください。

## 実施した確認（初期実装）

- Python / pytest: 38件。認証・Origin・CSRF、Secret暗号化、承認停止と再開、拒否時の非実行、Memory履歴とScope、7種類のProviderのモデル取得（HTTPモック）、送信の冪等性、パストラバーサル・symlink、公開IP判定、バックアップ復元と失敗時の補償を含む。
- MCP: 公式SDKのテスト用stdio Serverを実際の子プロセスとして起動し、Tool取得・呼出しを確認。
- Vitest: 2件。APIクライアントのヘッダーとエラー処理。
- TypeScript型チェックとVite production build成功。Python未定義名チェック成功。
- `docker compose config --quiet` 成功。
- Codex内ブラウザで管理者作成、Setup、Memory保存、Provider追加、モデル取得、Chat送信、回答表示、再読込後の会話履歴を確認。
- チャットの390×844相当のviewportで入力欄と回答表示を目視確認。これは実機のソフトウェアキーボードの検証ではない。

画面検証では `tests/fixtures/fake_provider.py` の固定応答を使用。実AIの応答品質・課金・能力の検証ではない。画面検証と自動テストはCompose profileごとの使い捨てPostgreSQLを使い、本運用のvolumeとは分離している。テスト用アカウントを本運用へ移行しないこと。

## 未完了の受入確認

Dockerビルド中にホストの空き容量が約1.8GiBまで低下したため、ビルドを中止した。既存データやDockerキャッシュの削除は行っていない。最終コードの全イメージ再ビルドと実起動は未確認。容量を確保した環境で以下を実施する必要がある。

- 新規Dockerボリュームから、`.env`編集なしでSetupと実Providerへの最初のチャット。
- PostgreSQL上のMigration・排他制約・pg_trgm検索。
- Runnerの直接通信遮断、Proxy許可先の制限、DNS再解決・リダイレクト・metadata対策のコンテナ境界での確認。
- TerminalからDB、Provider Key、MCP Secret、Docker Socketへアクセスできないこと。
- Playwright Browser Tool、PDF抽出、Terminalキャンセル・子孫プロセス停止の実コンテナ確認。
- 承認待ちのDocker再起動、実行途中の強制終了、SSE再接続、Remote操作の結果不明表示。
- 空のDocker環境への完全バックアップ復元。テストはテスト用Runnerとの復元・補償であり、実ボリュームの耐障害性を証明しない。
- 実OpenAI / Anthropic / Gemini等の本文、公開Reasoning情報、Tool Call、画像添付、Provider固有情報の継続送信。
- 実Streamable HTTP MCP、Filesystemパッケージ取得、資格情報の送信先、Schema変更時の再承認。
- スマートフォン実機・キーボード操作・長いTool出力・切断時の再送・すべての設定画面。

## 現時点の制約

- 管理者1名、app 1プロセスを前提とする。共有Runnerは敵対するユーザーを隔離するSandboxではない。
- Providerのモデル一覧は全Providerのページングを網羅していない。取得できないモデルは手動登録する。
- Capabilityは不明を残す。未知のモデルのTool/Vision/Reasoningは管理者が確認してOverrideする。
- 日本語UIが初期実装されているが、翻訳文言の完全な分離と多言語化は未完了。
- バックアップはサイズ上限付き。空ディレクトリ・POSIX権限・BrowserのCookie・導入済みMCPパッケージ自体は完全再現しない。詳細は運用資料を参照。
- Secretは設定APIで再表示せず暗号化するが、外部MCPが結果本文に資格情報を含めた場合の自動検出・完全除去は保証しない。信頼できるServerのみ接続する。
- Knowledgeはテキストのチャンク検索に対応。外部同期や検索品質の実環境評価は未完了。MCP OAuthは実装済みですが、実Serverでの受入確認は別途必要です。画像生成・編集、Registry Store、OpenAI Compatible公開APIは未実装。

再実行コマンドはREADME参照。実Provider・Docker・実機で未実施の項目を、単体テストやビルド成功で置き換えて「動作確認済み」と扱わない。
