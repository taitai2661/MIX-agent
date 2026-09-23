# MIX-agent

個人・LAN/VPN内向けのセルフホストAIワークスペース。React + FastAPI + PostgreSQL。

**初期開発版です。インターネット公開・複数ユーザー・未知のコードを安全に実行する用途には使わないでください。**

## 導入方法

| 方法 | 対象 | 入口 |
| --- | --- | --- |
| Docker Compose | Dockerを自分で管理する人 | 下記の「Dockerで起動」 |
| ZimaOSコミュニティストア | AMD64のZimaOSを使う人 | [ZimaOSガイド](docs/zimaos.md) |

ZimaOSストアは配布イメージの公開とストア配布作業が完了したバージョンから利用できます。

## Dockerで起動

Docker Desktop、またはDocker EngineとCompose v2を用意します。初回ビルドとBrowser機能の導入に備えて、10GB以上のディスク空き容量を推奨します。

```sh
git clone https://github.com/taitai2661/MIX-agent.git
cd MIX-agent
docker compose up -d --build
```

[http://localhost:8080](http://localhost:8080) を開いて管理者を作成します。環境変数やAPI Keyの事前設定は不要です。

1. Providerを追加し、必要なAPI Keyを保存。保存後にモデル一覧を自動取得し、Context Windowを確認できるモデルをAuto候補へ追加します。
2. 必要に応じて「接続テスト」または「モデル取得」を再実行。
3. モデルの対応機能を確認。不明な機能は必要に応じて手動Override。
4. 既定モデル、Toolsの権限、Runnerの通信許可ドメインを設定。
5. 必要ならAgent・MCPを追加。

ローカルのOllama/LM Studioには、Provider設定でプライベート接続を明示的に許可します。Docker内のlocalhostはホストPCではありません。標準URLでは `host.docker.internal` を使用します。ホスト側サービスの待受設定も確認してください。

### 更新とバックアップ

更新前に管理画面から暗号化バックアップを取得し、復元に必要なパスワードを保管してください。
コードを取得して再ビルドすると、起動時にDB migrationが適用されます。

```sh
git pull --ff-only
docker compose up -d --build
```

状態はDockerボリュームに保存されます。`docker compose down` はコンテナを停止しますが、
`docker compose down -v` はボリュームも削除するため、更新時には実行しないでください。
手動Compose版からZimaOSストア版へ移る場合は、ボリュームが自動で引き継がれるとは限りません。
バックアップと復元の手順は[ZimaOSガイド](docs/zimaos.md)を確認してください。

### GitHub Releases

リリースページには、バージョンごとの[変更内容](docs/release-notes-v0.2.2.md)と
ZimaOSストアのアーカイブを掲載します。コンテナイメージはGitHub Container Registry
（GHCR）から取得します。初回リリースの公開作業には、GHCRパッケージをPublicに
設定してからストアとReleaseを配布する手順が含まれます。
公開手順は[リリースチェックリスト](docs/release-checklist.md)にまとめています。

## 実装内容

- Chat / Thinking / Agent、SSEストリーミング、会話履歴、添付
- 50種類のProviderプリセット（OpenAI、Anthropic、Gemini、主要互換API、国内・ローカル実行系）と、OpenAI互換／Anthropic／Gemini用カスタム接続
- 共通Tool Registry、Always Allow / Ask / Deny、永続化した承認
- Web / Files / Terminal / Browser / Memory、ユーザー向けPlan
- stdio / Streamable HTTP MCP、Filesystem導入テンプレート
- Agent編集、Memory履歴・復元、暗号化Secret
- 暗号化バックアップ、復元前退避、復元途中の再起動からの回復
- 運用診断（DB接続、Runner設定、滞留Run、直近の失敗・中断）
- Knowledgeのテキスト登録・検索Tool、Skillの登録・履歴・復元、定期実行
- Remote MCPのOAuth認証フロー（対応Serverとの接続が必要）
- 会話のMarkdown書き出し（回答のみ／会話／Tool実行概要）

Knowledgeはテキストのチャンク検索を提供します。外部資料の自動同期や検索品質の実環境評価は今後の課題です。画像生成・編集、Registry Store、外部向けOpenAI Compatible APIは次段階です。Vision対応モデルへの画像添付・解析は含みます。

Contextの上限が近づくと、古い会話を要約してモデルへ渡します。チャット画面ではRunごとの圧縮要約と対象件数を確認できます。要約が生成できない場合は古い文脈を切り落とさず実行を止めます。Runの「回答チェック」はユーザー向け最終回答の有無を判定するもので、内容の正しさや依頼達成を保証しません。

### Chat / Thinking / Agent

- **Chat**: 対応が確認されたモデルでは、必要に応じて思考・検索・作成・実行を使います。思考やTool Callingが未対応・未確認でも通常会話は可能です。
- **Thinking**: 通常推論を使いながら必要に応じて検索・作成・実行を進めます。Reasoning対応モデルではProvider固有の思考機能も有効にしますが、非対応モデルでもThinkingは利用できます。毎回の検索や思考文の表示・長さを保証するものではありません。
- **Agent**: 従来の思考設定と、長い作業向けの実行ループを維持します。

標準では全モードに組み込みツールを提供します。プリセット選択時はそのツール一覧（空リストを含む）を優先し、MCPを自動追加しません。Ask / Deny・承認・Runner隔離はモードにかかわらず適用されます。上限はChatが20分・12ステップ・12 Tool Call、Thinkingが45分・24ステップ・24 Tool Call、Agentが90分・300ステップ・750 Tool Callです。長い作業はAgentを使ってください。

Chat / ThinkingのProvider固有思考制御はOpenAI Responses、OpenRouter、識別可能なClaude 3.7 / 4系とGemini 2.5 / 3系に対応しています。Ollama / LM Studio / 汎用互換APIと未識別モデルでは、その専用パラメーターを送らず通常推論でThinkingを実行します。ThinkingでTool Callingが未確認の場合は、初回だけ副作用のないダミー関数を強制呼出しして互換性を確認します。結果は保存され、未確認・非対応時はツールなしで続行します。モデル設定の「Tool Callingを再確認」でだけ手動再試行できます。手動Overrideは自動確認より優先します。

APIの `mode` は変更していません。`GET /models` の各レコードの `data.reasoning_control` は計算した制御方式またはnullで、`data.tool_probe` は保存済みのTool Calling確認結果です。`POST /models/{id}/verify-tools` は副作用のない再確認を実行します。Runには解決済みの思考方針とツール一覧を保存し、追加情報のない旧Runは従来動作を維持します。公開された思考要約だけを表示し、署名付きコンテキストはツール継続用に保持します。

制御仕様の参照: [OpenAI](https://developers.openai.com/api/docs/guides/reasoning)、[Claude](https://platform.claude.com/docs/en/build-with-claude/adaptive-thinking)、[Gemini GenerateContent](https://ai.google.dev/gemini-api/docs/generate-content/thinking)、[OpenRouter](https://openrouter.ai/docs/guides/best-practices/reasoning-tokens)。実Providerでの確認範囲は検証記録を参照してください。

## 安全上の境界

- 標準では公開ポートはlocalhostの8080だけです。初期設定前のLAN公開は避けてください。
- Terminalは専用workspace全体を操作できます。サブフォルダ間の強い隔離はありません。
- Runnerはホスト・Docker Socket・Provider Key・DBパスワードをマウントしません。
- TerminalとMCPは別コンテナ。MCP Server同士は単一管理者の信頼領域を共有します。
- Runner通信は内部ネットワークとProxyを使用。許可ドメインが未設定なら公開ドメインを許可し、設定時は完全一致の許可リストとして扱います。private IPは拒否します。
- 任意のShellコードを許可した場合、「パッケージ導入だけを完全に禁止する」保証はありません。
- MCPパッケージ導入や通信許可先変更は管理者の操作です。AIからは設定を変更できません。
- Dockerの隔離設定は専用OS/VMによる敵対的コード隔離の代わりではありません。
- Providerに送るメッセージ・画像・Tool結果はProviderのデータ規約に従います。
- プリセットは接続方式と既定URLを設定するための一覧です。実際のモデル対応・料金・利用規約は各Providerで確認してください。URLが空のプリセットは、組織またはローカル環境のBase URLを入力して接続します。

## 開発

Node.js 22以上、pnpmを使用します。アプリケーションDBはPostgreSQL専用で、起動はDocker Compose経由です。

```sh
cd apps/web
pnpm install --frozen-lockfile
pnpm build
cd ../..
docker compose up -d --build
```

DBを含むアプリ起動はCompose内で完結します。ホストShellを使うRunnerフォールバックはありません。

```sh
# リポジトリルートから。使い捨てPostgreSQLでmigration後に実行します。
docker compose --profile test run --rm test
cd apps/web
pnpm test
pnpm build
```

API仕様は `docs/openapi.json`、生成TypeScript型は `apps/web/src/generated/api.ts`。更新は `scripts/export-openapi.py` と `pnpm generate:api` で行います。

詳しくは [アーキテクチャ](docs/architecture.md)、[運用](docs/operations.md)、[検証状況](docs/verification.md) を参照してください。
