# MIX-agent v0.3.0

## 概要

v0.2.3以降の完成済みMIX-agentを、v0.3.0として正式に公開・配布します。

今回のリリース対象であるmainブランチでは、アプリケーションの実装ロジックやUIを変更していません。既存の完成済みコードをv0.3.0として識別できるよう、アプリケーションのバージョン情報、API仕様、コンテナ配布参照、ZimaOS配布メタデータを統一しました。

## 更新内容

### バージョン情報の統一

以下のバージョン参照を `0.3.0` に更新しています。

- FastAPIアプリケーションのバージョン
- MCP ClientInfoのバージョン
- Pythonパッケージのバージョン（`pyproject.toml` / `uv.lock`）
- Webパッケージのバージョン
- OpenAPI仕様の `info.version`
- ZimaOS用Composeのアプリバージョン

### コンテナ配布

既存のGitHub Actionsによる公開方式を継続し、Gitタグ `v0.3.0` を起点に、次の8種類のAMD64向けコンテナイメージをGHCRへ公開しました。

- `ghcr.io/taitai2661/mix-agent-init:v0.3.0`
- `ghcr.io/taitai2661/mix-agent-app:v0.3.0`
- `ghcr.io/taitai2661/mix-agent-execution:v0.3.0`
- `ghcr.io/taitai2661/mix-agent-browser-runner:v0.3.0`
- `ghcr.io/taitai2661/mix-agent-browser-provisioner:v0.3.0`
- `ghcr.io/taitai2661/mix-agent-mcp:v0.3.0`
- `ghcr.io/taitai2661/mix-agent-mcp-runtime:v0.3.0`
- `ghcr.io/taitai2661/mix-agent-egress:v0.3.0`

### ZimaOS Community Store

ZimaOS向けの既存配布経路をそのまま利用し、v0.3.0のイメージ参照に更新して公開しました。

- ZimaOS v2向けストアメタデータ
- 旧CasaOS/ZimaOSクライアント向けv1 `store/main.zip`
- `Apps/MIX-agent/docker-compose.yml` 内の全イメージタグ
- `x-casaos.version` とリリースタグの一致
- GitHub Releaseに添付されたストアアーカイブ

ZimaOSのストアURLは従来どおりです。

- v2: `https://cdn.jsdelivr.net/gh/taitai2661/MIX-agent@gh-pages/store.json`
- v1互換: `https://cdn.jsdelivr.net/gh/taitai2661/MIX-agent@gh-pages/store/main.zip`

## 変更していないもの

- アプリケーションの機能ロジック
- Web UI
- Dockerのビルド方式・構成
- 既存のGHCR公開方式
- ZimaOSストアの公開方式
- 依存関係の追加

## 更新前の注意

更新前にMIX-agentの管理画面から暗号化バックアップを取得し、復元用パスフレーズを保管してください。ZimaOSストア版と手動Compose版ではデータ領域が異なる場合があり、既存データが自動的に引き継がれないことがあります。

MIX-agentは信頼できるLAN/VPN内での単一管理者利用を前提としています。初期管理者の作成前にインターネットへ公開しないでください。MCP管理用コンテナはDocker socketを使用します。

## リリース参照

- Git tag: `v0.3.0`
- GitHub Release: **MIX-agent v0.3.0**
- コンテナレジストリ: GitHub Container Registry（GHCR）
