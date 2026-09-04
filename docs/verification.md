# 検証記録

2026-08-30。これは初期実装の検証記録であり、本番運用の認定ではありません。

## Chat / Thinking拡張の確認（2026-08-30追記）

- Chatの自動思考、ThinkingのネイティブReasoning／通常推論フォールバック、プリセットの空リストを含むツール制限をテスト。
- OpenAI Responses / OpenRouter / AnthropicはHTTPモックと実SDKで送信設定・公開要約・ツール継続を確認。GeminiはSDK型を使うクライアントモックで確認。Claudeのadaptive / budget、Geminiのlevel / budgetの両方式を含む。
- Chat / Thinkingの承認待ち・再開・拒否、3 / 5ステップ上限、停止、ツール失敗時の安全なエラー返却をテスト。既存の認証・保存・バックアップ等の回帰テストも実施。
- Vitestは合計6件。モードの利用可否、明示的な不明Override、空のプリセット、要約表示と既存APIクライアントを確認。TypeScript / Vite build成功。
- 非対応ReasoningモデルのThinking通常推論フォールバック、初回Tool Callingプローブ、結果保存、手動再確認はHTTPモックの統合テストで確認。実Providerとブラウザでの受入確認は未実施。
- 390×844のviewportで思考要約・モード選択・入力欄を目視確認。実機キーボードは未確認。

今回の画面検証は `tests/fixtures/chat_preview.py` を使用。Compose profile `preview` が起動ごとに使い捨てPostgreSQLを作り、Providerはローカルの固定応答、ツール実行はモックに置き換える。既存データ、実AI、実ファイル操作、実コマンド、実Web検索は使用しない。実Provider各接続方式の課金・応答・思考品質と実Runnerの操作は未検証。

全体Lintには既存のFastAPI Depends / wildcard import等の指摘が残る。Python未定義名チェックと、新規の思考解決・回帰テストファイルのLintは成功。Git管理外のフォルダのため `git diff --check` は実行できない。

## 実施した確認

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
- Knowledge / RAG、画像生成・編集、MCP OAuth、Registry Store、OpenAI Compatible公開APIは未実装。

再実行コマンドはREADME参照。実Provider・Docker・実機で未実施の項目を、単体テストやビルド成功で置き換えて「動作確認済み」と扱わない。
