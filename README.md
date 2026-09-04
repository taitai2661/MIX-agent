# MIX-agent

React と FastAPI、PostgreSQL で構成された、個人または信頼できる LAN / VPN 内で利用するセルフホスト AI ワークスペースです。Chat、Thinking、Agent、会話履歴、添付ファイル、Tool、MCP、Memory、Provider 管理などを一つの画面から扱えます。

> **開発段階に関する注意**
> MIX-agent は初期開発版です。COdexで突貫工事で作られているので多分バグ多いです。セキュリティも保証しませんし、できませんので、全て自己責任でお願いします。

## まず動かす

### 必要なもの

| 項目 | 必要条件 |
| --- | --- |
| Docker | Docker Desktop または Docker Engine |
| Compose | Docker Compose v2（`docker compose` コマンド） |
| 開発時の Node.js | Node.js 22 以上 |
| 開発時のパッケージマネージャー | pnpm |
| ディスク | 初回ビルドで Playwright 用ブラウザを取得するため、10 GB 以上を推奨 |

### Docker Compose で起動する

リポジトリのルートで次を実行します。

```sh
git clone https://github.com/taitai2661/MIX-agent.git
cd MIX-agent
docker compose up -d --build
```

起動後、ブラウザで [http://localhost:8080](http://localhost:8080) を開き、初回画面から管理者を作成します。通常のローカル起動では、環境変数や API Key を事前に設定する必要はありません。

停止する場合は次を実行します。

```sh
docker compose down
```

データを含むボリュームまで削除する場合は、内容を確認してから次を実行してください。削除したデータは復元できません。

```sh
docker compose down -v
```

### 初回設定

管理画面で Provider を追加し、必要な API Key を保存します。保存後にモデル一覧を取得し、利用する既定モデル、Tools の権限、Runner の通信許可ドメインを設定してください。必要に応じて Agent や MCP も追加できます。

ローカルの Ollama や LM Studio を Docker 内から利用する場合、Docker コンテナの `localhost` はホスト PC を指しません。標準的には `host.docker.internal` を使い、Provider 設定でプライベート接続を明示的に許可します。ホスト側サービスの待受アドレスとファイアウォール設定も確認してください。

## 主な機能

| 分野 | 内容 |
| --- | --- |
| 会話 | Chat / Thinking / Agent、SSE ストリーミング、会話履歴、添付ファイル |
| Provider | OpenAI、Anthropic、Gemini、主要な互換 API、国内・ローカル実行系のプリセット、およびカスタム接続 |
| Tools | 共通 Tool Registry、Always Allow / Ask / Deny、永続化された承認 |
| 実行 | Web、Files、Terminal、Browser、Memory、ユーザー向け Plan |
| MCP | stdio / Streamable HTTP MCP、Filesystem 導入テンプレート |
| 管理 | Agent 編集、Memory 履歴・復元、暗号化 Secret、暗号化バックアップ |

Knowledge / RAG、画像生成・編集、MCP OAuth、Registry Store、外部向け OpenAI Compatible API は次段階です。Vision 対応モデルへの画像添付・解析は初期版に含まれます。

## 開発者向けセットアップ

アプリケーション DB は PostgreSQL 専用で、DB を含むバックエンドの起動は Docker Compose 内で完結します。フロントエンドの依存関係をインストールしてビルドするには、次を実行します。

```sh
cd apps/web
pnpm install --frozen-lockfile
pnpm build
cd ../..
docker compose up -d --build
```

フロントエンド単体のテストと、型生成のコマンドは次のとおりです。

```sh
cd apps/web
pnpm test
pnpm build
pnpm generate:api
```

バックエンドを含む検証は、リポジトリルートから使い捨て PostgreSQL コンテナを使って実行します。

```sh
docker compose --profile test run --rm test
```

API 仕様は [`docs/openapi.json`](docs/openapi.json)、生成済み TypeScript 型は [`apps/web/src/generated/api.ts`](apps/web/src/generated/api.ts) にあります。API 仕様を変更したときは、[`scripts/export-openapi.py`](scripts/export-openapi.py) と `pnpm generate:api` を使って関連ファイルを更新してください。

## ディレクトリ構成

```text
apps/server/       FastAPI バックエンド、DB モデル、Migration
apps/web/          React + Vite フロントエンド
packages/          共有パッケージ
runners/           Browser / Execution / MCP Runner
compose.yaml       ローカル開発・検証用 Compose 定義
deploy/            コンテナイメージおよびデプロイ関連設定
docs/              アーキテクチャ、運用、検証、OpenAPI
scripts/           API 型生成などの補助スクリプト
tests/             バックエンド共通テスト設定
```

設計の詳細は [アーキテクチャ](docs/architecture.md)、運用時の確認事項は [運用ガイド](docs/operations.md)、実装済み機能の検証範囲は [検証状況](docs/verification.md) を参照してください。

## 安全上の境界

標準設定で公開されるポートは localhost の 8080 だけです。初期設定前に LAN へ公開しないでください。Terminal は専用 workspace 全体を操作し、サブフォルダ間の強い隔離は行いません。Runner はホスト、Docker Socket、Provider Key、DB パスワードをマウントしませんが、Docker の隔離は専用 OS / VM による敵対的コード隔離の代替ではありません。

任意の Shell コードを許可した場合、パッケージ導入を完全に禁止できる保証はありません。MCP パッケージの導入や通信許可先の変更は管理者の操作として扱われ、AI から設定を変更できない設計です。Provider に送信するメッセージ、画像、Tool 結果は、各 Provider のデータ規約と利用規約に従います。API Key や `.env` ファイルを Git にコミットしないでください。

## GitHub へ公開する手順

このリポジトリを自分の GitHub リポジトリへ公開する場合は、まず内容を確認します。

```sh
git status
git diff --stat
git diff --check
```

次に、ライセンスとドキュメントを含めてコミットし、GitHub のリモートへ push します。

```sh
git add .
git commit -m "Prepare project for Apache-2.0 release"
git branch -M main
git push -u origin main
```

GitHub のリポジトリ画面で、`LICENSE` が **Apache License 2.0** として認識され、README が表示されることを確認してください。公開前には、次の項目を必ず確認します。

| 確認項目 | 確認内容 |
| --- | --- |
| 秘密情報 | API Key、パスワード、トークン、個人データ、`.env`、ローカル DB が含まれていない |
| ライセンス | [`LICENSE`](LICENSE) と [`NOTICE`](NOTICE) が含まれている |
| 再現性 | クリーンな環境で `docker compose up -d --build` が実行できる |
| 検証 | `docker compose --profile test run --rm test`、`pnpm test`、`pnpm build` が通る |
| 公開範囲 | 初期開発版であることと安全上の制限が README に明記されている |

## ライセンス

Copyright 2026 MIX-agent contributors.

本ソフトウェアは [Apache License 2.0](LICENSE) の下で公開します。ライセンスの本文については [`LICENSE`](LICENSE)、追加の著作権・帰属表示については [`NOTICE`](NOTICE) を確認してください。依存ライブラリには別個のライセンスが適用される場合があります。

## References

[1]: https://www.apache.org/licenses/LICENSE-2.0 "Apache License 2.0"
[2]: https://choosealicense.com/licenses/apache-2.0/ "Apache 2.0 License overview"
