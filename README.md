<div align="center">

# MIX-agent

**個人・LAN/VPN 内向けのセルフホスト AI ワークスペース**

[![CI](https://github.com/taitai2661/MIX-agent/actions/workflows/ci.yml/badge.svg)](https://github.com/taitai2661/MIX-agent/actions/workflows/ci.yml)
[![Release](https://github.com/taitai2661/MIX-agent/actions/workflows/release.yml/badge.svg)](https://github.com/taitai2661/MIX-agent/actions/workflows/release.yml)
[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)
[![Docker](https://img.shields.io/badge/docker--compose-2496ED?logo=docker&logoColor=white)](compose.yaml)

日本語 | [English](README.en.md)

</div>

---

MIX-agent は、自分のマシンまたは信頼できるネットワーク（LAN/VPN）の中で AI を使うための、セルフホスト AI ワークスペースです。React + FastAPI + PostgreSQL で構成され、Docker Compose で起動します。チャット・長時間の Agent 実行・ブラウザ操作・MCP・ローカルファイル作業を、自分で管理する環境で完結させられます。

> **ステータス**: 初期開発版です。インターネット公開・複数ユーザー・未知のコードを安全に実行する用途には使わないでください。

## 特徴

- **Chat / Thinking / Agent の 3 モード**: SSE ストリーミング、会話履歴、添付に対応。Agent モードはチェックポイント・停滞検知・検証ループ・予算の動的調整などの自律実行基盤を備えます
- **幅広い Provider 対応**: OpenAI / Anthropic / Gemini ほか主要互換 API・国内・ローカル実行系を含む 52 種のプリセットと、カスタム接続。Capability Probe による Tool Calling の自動確認付き
- **Tool Registry と権限管理**: Web / Files / Terminal / Browser / Memory などの組み込みツールを Always Allow / Ask / Deny で制御。承認は永続化されます
- **MCP 対応**: stdio / Streamable HTTP、Remote MCP の OAuth 認証フロー、Filesystem 導入テンプレート
- **プロジェクト / Knowledge / Skills**: プロジェクト単位の指示・資料・会話整理、テキストのチャンク検索、`SKILL.md`（Agent Skills）の取り込み・書き出し
- **運用と安全**: 暗号化バックアップ・復元、運用診断、Runner の隔離と通信許可ドメイン制御（[安全上の境界](#安全上の境界)を参照）

## エディション構成

本リポジトリは 2 つのエディションを同一リポジトリで管理しています：

| エディション | 技術スタック | 特徴・用途 | 導入・起動方法 |
| --- | --- | --- | --- |
| **Desktop 版** (`apps/desktop`) | Tauri 2 + Rust + React | 個人ローカル専用、ネイティブ UI、高速・省リソース | **推奨**: Releases から `.dmg` 等をインストール<br>開発時: `pnpm desktop:dev` |
| **Docker 版** (`apps/server`, `apps/web`) | FastAPI + PostgreSQL + React | セルフホスト、LAN/VPN 共有、長時間バックグラウンド実行 | `docker compose up -d --build` |

## 導入方法

### Desktop 版で起動（ローカルネイティブアプリ・推奨）

#### 1. インストーラーから導入（推奨）

[GitHub Releases](https://github.com/taitai2661/MIX-agent/releases) からお使いの OS に合わせたインストーラーをダウンロードします：

- **macOS**:
  - Apple Silicon (M1/M2/M3/M4): `mix-agent-desktop_*_aarch64.dmg`
  - Intel Mac: `mix-agent-desktop_*_x64.dmg`
  - ダウンロードした `.dmg` を開き、`MIX-agent Desktop` を `Applications` フォルダへドラッグ＆ドロップしてインストールします。
  - **v0.4.0 の macOS ビルドは署名・公証されていません**（公開環境に Apple Developer の証明書が設定されていないため）。初回起動時に Gatekeeper の警告が表示されます。アプリアイコンを「Control キーを押しながらクリック」→「開く」を選択するか、ターミナルで `xattr -cr "/Applications/MIX-agent Desktop.app"` を実行してください。
- **Windows**: `*.msi` または `*.exe` インストーラー
- **Linux**: `*.deb` または `*.AppImage` パッケージ

#### 2. ソースコードから起動・ビルド

Rust (1.82+)、Node.js (22+)、pnpm を用意します。

```sh
# 依存関係のインストール
pnpm install

# デスクトップ版の起動 (Tauri Dev)
pnpm desktop:dev

# macOS 向け .dmg パッケージのローカルビルド
pnpm desktop:build:dmg
```

1. Provider を追加し、必要な API Key を保存。保存後にモデル一覧を自動取得し、Context Window を確認できるモデルを Auto 候補へ追加します。
2. 必要に応じて「接続テスト」または「モデル取得」を再実行。
3. モデルの対応機能を確認。不明な機能は必要に応じて手動 Override。
4. 既定モデル、Tools の権限、Runner の通信許可ドメインを設定。
5. 必要なら Agent・MCP を追加。

ローカルの Ollama / LM Studio には、Provider 設定でプライベート接続を明示的に許可します。

### Docker で起動（セルフホスト・サーバー版）

Docker Desktop、または Docker Engine と Compose v2 を用意します。
リポジトリを clone して直下で起動するだけで、**追加の設定ファイル作成なしで即座にインストール・起動**できます。

```sh
git clone https://github.com/taitai2661/MIX-agent.git
cd MIX-agent
docker compose up -d --build
```

[http://localhost:8080](http://localhost:8080) をブラウザで開いて管理者を作成します。環境変数や API Key の事前設定は不要です。Docker 内の localhost はホスト PC ではありません。ローカルの Ollama / LM Studio 等に接続する標準 URL では `host.docker.internal` を使用します。

### プロジェクトフォルダの接続

作業対象のリポジトリを読み取り専用で `/project` にマウントすると、そのフォルダの `AGENTS.md`（無い場合は `CLAUDE.md`）が各 Run の指示に自動で読み込まれ、`.mix/skills` → `.claude/skills` → `.agents/skills` のうち最初に存在したディレクトリの `SKILL.md` が Skill として提供されます。いずれも DB には保存されず、ファイルを編集すれば次回の Run から反映されます。

```sh
MIX_PROJECT_DIR=/path/to/your/repo docker compose up -d
```

`MIX_PROJECT_DIR` を指定しない場合は Compose ファイルのあるディレクトリがマウントされます。指示書もスキルも無い場合、何も追加されません。スケジュール実行にはプロジェクト文脈（指示書・資料）を渡していません（対話 Run のみ）。

### 更新とバックアップ

更新前に管理画面から暗号化バックアップを取得し、復元に必要なパスワードを保管してください。コードを取得して再ビルドすると、起動時に DB migration が適用されます。

```sh
git pull --ff-only
docker compose up -d --build
```

状態は Docker ボリュームに保存されます。`docker compose down` はコンテナを停止しますが、`docker compose down -v` はボリュームも削除するため、更新時には実行しないでください。手動 Compose 版から ZimaOS ストア版へ移る場合は、ボリュームが自動で引き継がれるとは限りません。バックアップと復元の手順は [ZimaOS ガイド](docs/zimaos.md) を確認してください。

### リリース

[GitHub Releases](https://github.com/taitai2661/MIX-agent/releases) にて、バージョンごとの変更内容とともに各配布物を公開しています：
- **Desktop 版インストーラー**: macOS 用 `.dmg`（Apple Silicon / Intel）、Windows 用 `.msi` / `.exe`、Linux 用 `.deb` / `.AppImage`、およびチェックサム（`SHA256SUMS`）
- **Docker コンテナイメージ**: GitHub Container Registry（GHCR）より配布
- **ZimaOS / CasaOS ストアアーカイブ**: `mix-agent-zimaos-store-*.tar.gz`

Desktop 版の版は独立したラインで、v0.4.0 の時点で `0.1.0` です。Docker 版と同じ Release に添付されますが、アプリ内の版番号は `0.1.0` のままです。

最新版は [v0.4.0](docs/release-notes-v0.4.0.md) です。公開手順は[リリースチェックリスト](docs/release-checklist.md)にまとめています。

## 実装内容

- Chat / Thinking / Agent、SSE ストリーミング、会話履歴、添付
- プロジェクト（専用の指示・資料テキスト・会話の整理）と、Agent を使う調査モード（Web 調査と出典つきレポート）
- 52 種類の Provider プリセット（OpenAI、Anthropic、Gemini、主要互換 API、国内・ローカル実行系）と、OpenAI 互換／Anthropic／Gemini 用カスタム接続
- 共通 Tool Registry、Always Allow / Ask / Deny、永続化した承認
- Web / Files / Terminal / Browser / Memory、ユーザー向け Plan
- stdio / Streamable HTTP MCP、Filesystem 導入テンプレート
- Agent 編集、Memory 履歴・復元、暗号化 Secret
- 暗号化バックアップ、復元前退避、復元途中の再起動からの回復
- 運用診断（DB 接続、Runner 設定、滞留 Run、直近の失敗・中断）
- Knowledge のテキスト登録・検索 Tool、Skill の登録・履歴・復元、定期実行
- Agent Skills（`SKILL.md`）の取り込み・書き出し（貼り付け／zip／サーバーディレクトリ）、同梱資料の読取 Tool
- Remote MCP の OAuth 認証フロー（対応 Server との接続が必要）
- 会話の Markdown 書き出し（回答のみ／会話／Tool 実行概要）

Knowledge はテキストのチャンク検索を提供します。外部資料の自動同期や検索品質の実環境評価は今後の課題です。画像生成・編集、Registry Store、外部向け OpenAI Compatible API は次段階です。Vision 対応モデルへの画像添付・解析は含みます。

### Agent Skills

`name` と `description` を必須とする `SKILL.md`（YAML frontmatter + Markdown）を取り込めます。Skills 画面で本文を貼り付けるか、`.zip` をアップロードするか、データボリュームの `/data/skills/<name>/SKILL.md` に置いて「サーバーディレクトリから取込」を実行します（`MIX_SKILLS_DIR` で別の置き場所に変更できます）。各 Skill は `SKILL.md` または `.zip` として書き出せます。`scripts`・`references`・`assets` の同梱ファイルは保存され、AI は `skill_resource` Tool で読み取りますが、スクリプトは実行しません。`allowed-tools` は表示のみで、権限の自動許可は行いません。

プロジェクトの資料は現時点では貼り付けたテキストを各 Run に渡します。プロジェクトに既存会話を移す場合は「会話を管理」から選択できます。プロジェクトを削除しても会話は残ります。調査モードには有効な Web 検索または Browser Tool と Tool Calling 対応モデルが必要です。出典の正確さは利用者が確認してください。

Context の上限が近づくと、古い会話を要約してモデルへ渡します。チャット画面では Run ごとの圧縮要約と対象件数を確認できます。要約が生成できない場合は古い文脈を切り落とさず実行を止めます。Run の「回答チェック」はユーザー向け最終回答の有無を判定するもので、内容の正しさや依頼達成を保証しません。

### Chat / Thinking / Agent

- **Chat**: 対応が確認されたモデルでは、必要に応じて思考・検索・作成・実行を使います。思考や Tool Calling が未対応・未確認でも通常会話は可能です。
- **Thinking**: 通常推論を使いながら必要に応じて検索・作成・実行を進めます。Reasoning 対応モデルでは Provider 固有の思考機能も有効にしますが、非対応モデルでも Thinking は利用できます。毎回の検索や思考文の表示・長さを保証するものではありません。
- **Agent**: 従来の思考設定と、長い作業向けの実行ループを維持します。さらに、定期チェックポイント、停滞検知と自己修復ガイダンス、厳格な検証ループ、階層的な文脈圧縮、予算の動的調整を備えます。

標準では全モードに組み込みツールを提供します。プリセット選択時はそのツール一覧（空リストを含む）を優先し、MCP を自動追加しません。Ask / Deny・承認・Runner 隔離はモードにかかわらず適用されます。上限は Chat が 20 分・12 ステップ・12 Tool Call、Thinking が 45 分・24 ステップ・24 Tool Call、Agent が 90 分・300 ステップ・750 Tool Call です。長い作業は Agent を使ってください。

#### Agent の自律性

- **中間チェックポイント**: 10 ステップごとに履歴・task_state・plan・直近 Tool Call を `run_checkpoints` に保存。中断後の再開はチェックポイント単位で選択可能で、`POST /runs/{key}/resume` の `from_checkpoint` で任意の地点から再開できます。
- **停滞検知と自己修復**: 直近 12 Tool Call から繰り返し失敗・読み取り停滞・Provider 不安定を検知し、ステップ毎に修復ガイダンスを user-role 履歴へ注入。`stagnation_detected` イベントで UI にも表示。
- **厳格な検証ループ**: `update_plan` の `phase:<name>` 区切りごとに検証を要求。`delete_file` 後の `files_list` / `search_files`、`write_file` 上書き前の `read_file`、破壊的ターミナル操作後の check Tool を必須化。
- **階層的な文脈圧縮**: active サマリ（4000 字まで）と archive rows（最大 12 件）を別管理。task_state（goal/plan/pending/important_facts/artifacts/open_questions）を head ブロックで優先保持。
- **予算の動的調整**: ステップまたはツール上限到達時にユーザーへ延長確認（既定 2 回まで）。`POST /runs/{key}/budget-extension` で `grant: true/false` を返却。確認なしで停止しないよう UX を強化。
- **結果不明操作の選択的処理**: 再開 API の `unknown_actions` で `executing` のまま残った各 Tool Call を `retry` / `discard` / `mark_unknown` 単位で制御。

Chat / Thinking の Provider 固有思考制御は OpenAI Responses、OpenRouter、識別可能な Claude 3.7 / 4 系と Gemini 2.5 / 3 系に対応しています。Ollama / LM Studio / 汎用互換 API と未識別モデルでは、その専用パラメーターを送らず通常推論で Thinking を実行します。Tool Calling が未確認のモデルを使うときは、Chat / Thinking / Agent のいずれも実行前に副作用のないダミー関数で自動確認します。結果は保存され、未確認・非対応時はツールなしで続行します。再実行は capability probe の鮮度ポリシー（未確定 6 時間、確定 14 日）に従い、Scheduler tick でも並行して走ります。モデル設定の「Tool Calling を再確認」でだけ手動再試行できます。手動 Override は自動確認より優先します。

API の `mode` は変更していません。`GET /models` の各レコードの `data.reasoning_control` は計算した制御方式または null で、`data.tool_probe` は保存済みの Tool Calling 確認結果です。`POST /models/{id}/verify-tools` は副作用のない再確認を実行します。Run には解決済みの思考方針とツール一覧を保存し、追加情報のない旧 Run は従来動作を維持します。公開された思考要約だけを表示し、署名付きコンテキストはツール継続用に保持します。

### Run Events

`GET /runs/{key}/events` の SSE ストリームは次の `kind` 値を流します（Engine→UI の表現契約）：

- 既存の `text` / `reasoning` / `model_started` / `model_selected` / `model_rerouted` / `message` / `status` / `tool_started` / `tool_result` / `plan` / `approval` / `browser_frame` / `context_summary`
- 新規 `checkpoint_saved` / `budget_extension_requested` / `budget_extension_resolved` / `stagnation_detected` / `verification_required` / `phase_advanced`

`run_checkpoints` のチェックポイントは `GET /runs/{key}/checkpoints` で一覧可能。`budget_extension_pending` 状態の Run に対する応答は `POST /runs/{key}/budget-extension` で `grant` と任意の予算上書きを受け付けます。

制御仕様の参照: [OpenAI](https://developers.openai.com/api/docs/guides/reasoning)、[Claude](https://platform.claude.com/docs/en/build-with-claude/adaptive-thinking)、[Gemini GenerateContent](https://ai.google.dev/gemini-api/docs/generate-content/thinking)、[OpenRouter](https://openrouter.ai/docs/guides/best-practices/reasoning-tokens)。実 Provider での確認範囲は[検証状況](docs/verification.md)を参照してください。

## 主なドキュメント

| ドキュメント | 内容 |
| --- | --- |
| [アーキテクチャ](docs/architecture.md) | サービス構成と設計 |
| [運用](docs/operations.md) | 運用手順 |
| [検証状況](docs/verification.md) | 実環境での確認範囲 |
| [OpenAPI 仕様](docs/openapi.json) | API 仕様（生成 TypeScript 型は `apps/web/src/generated/api.ts`） |
| [ZimaOS ガイド](docs/zimaos.md) | ZimaOS コミュニティストアでの導入 |
| [コントリビューション](CONTRIBUTING.md) | 開発手順と Pull Request の流れ |

## 安全上の境界

- 標準では公開ポートは localhost の 8080 だけです。初期設定前の LAN 公開は避けてください。
- Terminal は専用 workspace 全体を操作できます。サブフォルダ間の強い隔離はありません。
- Runner はホスト・Docker Socket・Provider Key・DB パスワードをマウントしません。
- Terminal と MCP は別コンテナ。MCP Server 同士は単一管理者の信頼領域を共有します。
- Runner 通信は内部ネットワークと Proxy を使用。許可ドメインが未設定なら公開ドメインを許可し、設定時は完全一致の許可リストとして扱います。private IP は拒否します。
- 任意の Shell コードを許可した場合、「パッケージ導入だけを完全に禁止する」保証はありません。
- MCP パッケージ導入や通信許可先変更は管理者の操作です。AI からは設定を変更できません。
- Docker の隔離設定は専用 OS/VM による敵対的コード隔離の代わりではありません。
- Provider に送るメッセージ・画像・Tool 結果は Provider のデータ規約に従います。
- プリセットは接続方式と既定 URL を設定するための一覧です。実際のモデル対応・料金・利用規約は各 Provider で確認してください。URL が空のプリセットは、組織またはローカル環境の Base URL を入力して接続します。

## 開発

Node.js 22 以上、pnpm を使用します。アプリケーション DB は PostgreSQL 専用で、起動は Docker Compose 経由です。

モデルのコンテキスト長・料金・能力などの補完データは [models.dev](https://models.dev) の `api.json` から取得します。既定の取得先は `https://models.dev/api.json` で、`MIX_MODELS_DEV_URL` に別の URL またはローカルの `api.json` へのパスを指定して差し替えられます。取得できない場合、当該モデルの補完データは未設定のまま扱われます。旧 `MIX_MODEL_INFO_URL` は非推奨です。

```sh
# フロントエンドのビルド
cd apps/web
pnpm install --frozen-lockfile
pnpm build
cd ../..
docker compose up -d --build

# テスト（リポジトリルートから。使い捨て PostgreSQL で migration 後に実行）
docker compose --profile test run --rm test
cd apps/web
pnpm test
pnpm build
```

DB を含むアプリ起動は Compose 内で完結します。ホスト Shell を使う Runner フォールバックはありません。API 仕様は `docs/openapi.json`、生成 TypeScript 型は `apps/web/src/generated/api.ts`。更新は `scripts/export-openapi.py` と `pnpm generate:api` で行います。

詳細は [CONTRIBUTING.md](CONTRIBUTING.md)、[アーキテクチャ](docs/architecture.md)、[運用](docs/operations.md)、[検証状況](docs/verification.md) を参照してください。

## ライセンス

本プロジェクトは [Apache License 2.0](LICENSE) で提供されます。詳細は [NOTICE](NOTICE) を参照してください。コントリビューションはすべて同じ Apache License 2.0 のもとで受理されます。
