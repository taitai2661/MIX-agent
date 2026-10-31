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
Node.js 22 以上、pnpm 10.x、Rust 1.91 以上（Desktop 版を触る場合。CI の固定ツールチェーンと合わせる）、Docker & Docker Compose（Docker 版を触る場合）を用意します。

> [!TIP]
> Rust のビルド成果物（`apps/desktop/target`）は数 GB〜30 GB 超まで育ちます。
> ディスクを空けたいときは `cargo clean --manifest-path apps/desktop/Cargo.toml` を
> 実行してください。`apps/server/.venv` と各 `node_modules` も同様に再生成できます。

```sh
# 依存関係のインストール（リポジトリルート）
pnpm install
# Web / Desktop は独立した lockfile を持つため、各エディションも導入する
pnpm setup
```

### 2. Docker 版の開発・テスト
Docker 版を変更した場合は、以下のテストとビルド確認を実施してください：

```sh
# バックエンドの単体テスト（使い捨て PostgreSQL コンテナで migration 後に実行）
docker compose --profile test run --rm test

# カバレッジ込み（CI と同じ。現在の下限は 70%）
docker compose --profile test run --rm test \
  sh -c "cd /app && export COVERAGE_FILE=/tmp/.coverage \
    && alembic upgrade head \
    && pytest -c /app/pyproject.toml -p no:cacheprovider -q \
         --cov=mix_agent --cov-report=term-missing --cov-fail-under=70 /workspace/tests"

# 型チェック（apps/server で。warning の総数も予算で監視する）
cd apps/server && uv run python ../../scripts/check-pyright-budget.py

# Python の lint / 整形（リポジトリ全体。runners・deploy・scripts・tests も対象）
# パスはリポジトリルートから渡す（per-file-ignores の glob を効かせるため）
uv run --project apps/server ruff check apps/server runners deploy scripts tests experiments
uv run --project apps/server ruff format --check apps/server runners deploy scripts tests experiments

# Web フロントエンドの lint / 型チェック / 整形 / テスト（カバレッジ下限付き）とビルド
pnpm web:lint
pnpm --dir apps/web typecheck
pnpm web:format:check
pnpm web:test:coverage
pnpm web:build

# ブラウザのスモークテスト（API はモックするため DB 不要）
pnpm --dir apps/web test:e2e
# ビルド成果物に対して実行する場合（CI と同じ）
E2E_SERVER=preview pnpm --dir apps/web test:e2e
```

リポジトリ全体の機械的ゲートは次のとおりです。PR を出す前にローカルでも実行できます:

```sh
python3 scripts/check-secrets.py        # コミットされた資格情報の形を検出
python3 scripts/check-readme-parity.py  # README.md と README.en.md の構造一致
node scripts/check-version-consistency.mjs
python3 scripts/check-memory-parity.py
python3 scripts/check-error-i18n.py
python3 scripts/check-greeting-parity.py
python3 scripts/check-browser-parity.py
node scripts/check-release-notes.mjs --tag v0.5.4
```

API 仕様（FastAPI ルート）を変更した場合は、`scripts/export-openapi.py` を実行して `docs/openapi.json` を更新し、`pnpm generate:api` で TypeScript 型定義を再生成してください。新しい HTTP エラーを追加した場合は、`mix_agent/api/error_codes.py` と `apps/web/src/app/errorCodes.ts` の両方に登録してください（`python3 scripts/check-error-i18n.py` が検査します）。新規コードは `HTTPException` ではなく `APIError(status, code)` を使うと、日本語文言の変更で機械可読コードが変わらなくなります。

### 3. Desktop 版の開発・テスト
Desktop 版を変更した場合は、以下のチェックとテストを実施してください：

```sh
# UI の単体テストと型チェック
pnpm desktop:ui:test
pnpm desktop:ui:build

# Rust クレートのチェックとテスト（CI と同じ clippy / rustfmt）
cargo clippy --workspace --all-targets --manifest-path apps/desktop/Cargo.toml -- -D warnings
cargo fmt --all --manifest-path apps/desktop/Cargo.toml -- --check
cargo test --workspace --manifest-path apps/desktop/Cargo.toml

# （任意）デスクトップアプリのローカルビルド確認
pnpm desktop:build:dmg      # macOS の場合
pnpm desktop:build:windows  # Windows の場合
pnpm desktop:build          # Linux の場合（AppImage + deb）
```

### 4. ドキュメント修正時の注意
- `docs/` のファイルは**移動・改名しない**でください。CI・リリースワークフロー・parity 検査がパスを直接参照しています（`docs/README.md` の「配置制約」を参照）。
- エディションに関する記述を行う際は、**Desktop 版と Docker 版のどちらに関する内容か**を明確にしてください。
- 共通の機能（Provider 対応、モード仕様、Agent ループなど）を変更した場合は、双方の README やドキュメントへの影響を確認してください。`README.md` と `README.en.md` は見出し階層・コマンド・リンク先を揃える必要があります（`scripts/check-readme-parity.py`）。
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
