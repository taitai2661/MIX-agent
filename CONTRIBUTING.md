# Contributing to MIX-agent

MIX-agent への改善提案、バグ報告、ドキュメント修正を歓迎します。変更を提案する前に、既存の Issue と [`docs/`](docs/) の設計資料を確認し、目的と影響範囲が分かる形で説明してください。

---

## リポジトリの構成

本リポジトリは、以下の 2 つのエディションを単一のリポジトリで管理するモノレポ構成です：

1. **Desktop 版** (`apps/desktop`): Tauri 2 + Rust + React 19 で構成されたネイティブデスクトップアプリ
2. **Docker 版** (`apps/server`, `apps/web`): FastAPI (Python) + PostgreSQL + React 19 で構成されたセルフホストサーバー

両エディションは独立してビルド・テストが可能です。

---

## 開発とテストの流れ

### 1. 準備
Node.js 22 以上、pnpm 10.x、Rust 1.82 以上（Desktop 版を触る場合）、Docker & Docker Compose（Docker 版を触る場合）を用意します。

```sh
# 依存関係のインストール（リポジトリルート）
pnpm install
```

### 2. Docker 版の開発・テスト
Docker 版を変更した場合は、以下のテストとビルド確認を実施してください：

```sh
# バックエンドの単体テスト（使い捨て PostgreSQL コンテナで migration 後に実行）
docker compose --profile test run --rm test

# Web フロントエンドのテストとビルド
pnpm web:test
pnpm web:build
```

API 仕様（FastAPI ルート）を変更した場合は、`scripts/export-openapi.py` を実行して `docs/openapi.json` を更新し、`pnpm generate:api` で TypeScript 型定義を再生成してください。

### 3. Desktop 版の開発・テスト
Desktop 版を変更した場合は、以下のチェックとテストを実施してください：

```sh
# UI の単体テストと型チェック
pnpm desktop:ui:test
pnpm desktop:ui:build

# Rust クレートのチェックとテスト
cargo check --workspace --manifest-path apps/desktop/Cargo.toml
cargo test --workspace --manifest-path apps/desktop/Cargo.toml

# （任意）デスクトップアプリのローカルビルド確認
pnpm desktop:build:dmg # macOS の場合
```

### 4. ドキュメント修正時の注意
- エディションに関する記述を行う際は、**Desktop 版と Docker 版のどちらに関する内容か**を明確にしてください。
- 共通の機能（Provider 対応、モード仕様、Agent ループなど）を変更した場合は、双方の README やドキュメントへの影響を確認してください。
- 未検証の環境・Provider を「動作確認済み」と記載しないでください。検証結果は [`docs/verification.md`](docs/verification.md) に追記してください。

### 5. Pull Request の作成
- `git diff --check` で不要な空白・フォーマットエラーがないか確認します。
- PR の概要には、変更理由・対象エディション・検証結果・既知の制限事項を明記してください。

---

## セキュリティ報告

API Key、パスワード、認証トークンなどの秘密情報を Issue や Pull Request に貼り付けないでください。脆弱性等の報告は公開 Issue ではなく、リポジトリ管理者へ非公開の方法でご連絡ください。

---

## ライセンス

本リポジトリへのコントリビューションは、別途明示的な合意がない限り、リポジトリの [Apache License 2.0](LICENSE) に従って受理されます。
