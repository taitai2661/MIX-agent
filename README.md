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

### プロジェクトフォルダの接続

作業対象のリポジトリを読み取り専用で `/project` にマウントすると、そのフォルダの `AGENTS.md`（無い場合は `CLAUDE.md`）が各Runの指示に自動で読み込まれ、`.mix/skills` → `.claude/skills` → `.agents/skills` のうち最初に存在したディレクトリの `SKILL.md` がSkillとして提供されます。いずれもDBには保存されず、ファイルを編集すれば次回のRunから反映されます。

```sh
MIX_PROJECT_DIR=/path/to/your/repo docker compose up -d
```

`MIX_PROJECT_DIR` を指定しない場合はComposeファイルのあるディレクトリがマウントされます。指示書もスキルも無い場合、何も追加されません。スケジュール実行にはプロジェクト文脈（指示書・資料）を渡していません（対話Runのみ）。

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

リリースページには、バージョンごとの[変更内容](docs/release-notes-v0.3.5.md)と
ZimaOSストアのアーカイブを掲載します。コンテナイメージはGitHub Container Registry
（GHCR）から取得します。初回リリースの公開作業には、GHCRパッケージをPublicに
設定してからストアとReleaseを配布する手順が含まれます。
公開手順は[リリースチェックリスト](docs/release-checklist.md)にまとめています。

## 実装内容

- Chat / Thinking / Agent、SSEストリーミング、会話履歴、添付
- プロジェクト（専用の指示・資料テキスト・会話の整理）と、Agentを使う調査モード（Web調査と出典つきレポート）
- 52種類のProviderプリセット（OpenAI、Anthropic、Gemini、主要互換API、国内・ローカル実行系）と、OpenAI互換／Anthropic／Gemini用カスタム接続
- 共通Tool Registry、Always Allow / Ask / Deny、永続化した承認
- Web / Files / Terminal / Browser / Memory、ユーザー向けPlan
- stdio / Streamable HTTP MCP、Filesystem導入テンプレート
- Agent編集、Memory履歴・復元、暗号化Secret
- 暗号化バックアップ、復元前退避、復元途中の再起動からの回復
- 運用診断（DB接続、Runner設定、滞留Run、直近の失敗・中断）
- Knowledgeのテキスト登録・検索Tool、Skillの登録・履歴・復元、定期実行
- Agent Skills（`SKILL.md`）の取り込み・書き出し（貼り付け／zip／サーバーディレクトリ）、同梱資料の読取Tool
- Remote MCPのOAuth認証フロー（対応Serverとの接続が必要）
- 会話のMarkdown書き出し（回答のみ／会話／Tool実行概要）

Knowledgeはテキストのチャンク検索を提供します。外部資料の自動同期や検索品質の実環境評価は今後の課題です。画像生成・編集、Registry Store、外部向けOpenAI Compatible APIは次段階です。Vision対応モデルへの画像添付・解析は含みます。

### Agent Skills

`name`と`description`を必須とする`SKILL.md`（YAML frontmatter + Markdown）を取り込めます。Skills画面で本文を貼り付けるか、`.zip`をアップロードするか、データボリュームの`/data/skills/<name>/SKILL.md`に置いて「サーバーディレクトリから取込」を実行します（`MIX_SKILLS_DIR`で別の置き場所に変更できます）。各Skillは`SKILL.md`または`.zip`として書き出せます。`scripts`・`references`・`assets`の同梱ファイルは保存され、AIは`skill_resource` Toolで読み取りますが、スクリプトは実行しません。`allowed-tools`は表示のみで、権限の自動許可は行いません。

プロジェクトの資料は現時点では貼り付けたテキストを各Runに渡します。プロジェクトに既存会話を移す場合は「会話を管理」から選択できます。プロジェクトを削除しても会話は残ります。調査モードには有効なWeb検索またはBrowser ToolとTool Calling対応モデルが必要です。出典の正確さは利用者が確認してください。

Contextの上限が近づくと、古い会話を要約してモデルへ渡します。チャット画面ではRunごとの圧縮要約と対象件数を確認できます。要約が生成できない場合は古い文脈を切り落とさず実行を止めます。Runの「回答チェック」はユーザー向け最終回答の有無を判定するもので、内容の正しさや依頼達成を保証しません。

### Chat / Thinking / Agent

- **Chat**: 対応が確認されたモデルでは、必要に応じて思考・検索・作成・実行を使います。思考やTool Callingが未対応・未確認でも通常会話は可能です。
- **Thinking**: 通常推論を使いながら必要に応じて検索・作成・実行を進めます。Reasoning対応モデルではProvider固有の思考機能も有効にしますが、非対応モデルでもThinkingは利用できます。毎回の検索や思考文の表示・長さを保証するものではありません。
- **Agent**: 従来の思考設定と、長い作業向けの実行ループを維持します。さらに、定期チェックポイント、停滞検知と自己修復ガイダンス、厳格な検証ループ、階層的な文脈圧縮、予算の動的調整を備えます。

標準では全モードに組み込みツールを提供します。プリセット選択時はそのツール一覧（空リストを含む）を優先し、MCPを自動追加しません。Ask / Deny・承認・Runner隔離はモードにかかわらず適用されます。上限はChatが20分・12ステップ・12 Tool Call、Thinkingが45分・24ステップ・24 Tool Call、Agentが90分・300ステップ・750 Tool Callです。長い作業はAgentを使ってください。

#### Agent の自律性

- **中間チェックポイント**: 10ステップごとに履歴・task_state・plan・直近Tool Callを `run_checkpoints` に保存。中断後の再開はチェックポイント単位で選択可能で、`POST /runs/{key}/resume` の `from_checkpoint` で任意の地点から再開できます。
- **停滞検知と自己修復**: 直近12 Tool Callから繰り返し失敗・読み取り停滞・Provider不安定を検知し、ステップ毎に修復ガイダンスを user-role 履歴へ注入。`stagnation_detected` イベントでUIにも表示。
- **厳格な検証ループ**: `update_plan` の `phase:<name>` 区切りごとに検証を要求。`delete_file` 後の `files_list` / `search_files`、`write_file` 上書き前の `read_file`、破壊的ターミナル操作後の check Tool を必須化。
- **階層的な文脈圧縮**: active サマリ（4000字まで）と archive rows（最大12件）を別管理。task_state（goal/plan/pending/important_facts/artifacts/open_questions）を head ブロックで優先保持。
- **予算の動的調整**: ステップまたはツール上限到達時にユーザーへ延長確認（既定2回まで）。`POST /runs/{key}/budget-extension` で `grant: true/false` を返却。確認なしで停止しないよう UX を強化。
- **結果不明操作の選択的処理**: 再開APIの `unknown_actions` で `executing` のまま残った各Tool Callを `retry` / `discard` / `mark_unknown` 単位で制御。

Chat / ThinkingのProvider固有思考制御はOpenAI Responses、OpenRouter、識別可能なClaude 3.7 / 4系とGemini 2.5 / 3系に対応しています。Ollama / LM Studio / 汎用互換APIと未識別モデルでは、その専用パラメーターを送らず通常推論でThinkingを実行します。Tool Callingが未確認のモデルを使うときは、Chat / Thinking / Agent のいずれも実行前に副作用のないダミー関数で自動確認します。結果は保存され、未確認・非対応時はツールなしで続行します。再実行は capability probe の鮮度ポリシー（未確定6時間、確定14日）に従い、Scheduler tick でも並行して走ります。モデル設定の「Tool Callingを再確認」でだけ手動再試行できます。手動Overrideは自動確認より優先します。

APIの `mode` は変更していません。`GET /models` の各レコードの `data.reasoning_control` は計算した制御方式またはnullで、`data.tool_probe` は保存済みのTool Calling確認結果です。`POST /models/{id}/verify-tools` は副作用のない再確認を実行します。Runには解決済みの思考方針とツール一覧を保存し、追加情報のない旧Runは従来動作を維持します。公開された思考要約だけを表示し、署名付きコンテキストはツール継続用に保持します。

### Run Events

`GET /runs/{key}/events` のSSEストリームは次の `kind` 値を流します（Engine→UIの表現契約）：

- 既存の `text` / `reasoning` / `model_started` / `model_selected` / `model_rerouted` / `message` / `status` / `tool_started` / `tool_result` / `plan` / `approval` / `browser_frame` / `context_summary`
- 新規 `checkpoint_saved` / `budget_extension_requested` / `budget_extension_resolved` / `stagnation_detected` / `verification_required` / `phase_advanced`

`run_checkpoints` のチェックポイントは `GET /runs/{key}/checkpoints` で一覧可能。`budget_extension_pending` 状態のRunに対する応答は `POST /runs/{key}/budget-extension` で `grant` と任意の予算上書きを受け付けます。

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

モデルのコンテキスト長・料金・能力などの補完データは [model-info](https://github.com/taitai2661/model-info) の静的JSONから取得します。既定の取得先はGitHub Pagesで、`MIX_MODEL_INFO_URL` に `https://` のアドレスまたはローカルのmodel-infoチェックアウト(その `v1/` を含むディレクトリ)を指定して差し替えられます。取得できない場合、当該モデルの補完データは未設定のまま扱われます。

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
