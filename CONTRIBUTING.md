# Contributing to MIX-agent

MIX-agent への改善提案、バグ報告、ドキュメント修正を歓迎します。変更を提案する前に、既存の Issue と [`docs/`](docs/) の設計資料を確認し、目的と影響範囲が分かる形で説明してください。

## 開発の流れ

1. GitHub で Issue を作成するか、既存の Issue に対応するブランチを作成します。
2. 小さく目的を分け、動作変更にはテストまたは検証手順を追加します。
3. API 仕様を変更した場合は `docs/openapi.json` と生成型を更新します。
4. `docker compose --profile test run --rm test`、`cd apps/web && pnpm test && pnpm build` を実行します。
5. `git diff --check` で空白エラーを確認し、Pull Request では変更理由、検証結果、既知の制限を記載します。

## セキュリティ報告

API Key、パスワード、認証トークンなどの秘密情報を Issue や Pull Request に貼り付けないでください。再現可能な詳細を公開すると影響が広がるおそれがある場合は、公開 Issue ではなく、リポジトリ管理者へ非公開の方法で連絡してください。

## ライセンス

このプロジェクトへの意図的な Contribution は、別途明示的な合意がない限り、リポジトリの [`LICENSE`](LICENSE) に定める Apache License 2.0 の条件に従います。
