# ドキュメント索引

`docs/` 直下と `docs/desktop/` の配置は CI・リリースワークフロー・parity 検査から参照されているため、**ファイルの移動・改名はしない**こと。

## 読む順

1. [`../README.md`](../README.md) — セットアップと全体像
2. [`architecture.md`](architecture.md) — モノレポ構成と Docker 版アーキテクチャ
3. [`desktop/architecture.md`](desktop/architecture.md)、[`memory-spec.md`](memory-spec.md)
4. [`../CONTRIBUTING.md`](../CONTRIBUTING.md)、[`operations.md`](operations.md)

## 一覧

| ドキュメント | 内容 |
| :--- | :--- |
| [`architecture.md`](architecture.md) | プロジェクト全体の構成と Desktop / Docker の設計対比 |
| [`agent-skills.md`](agent-skills.md) | Agent Skills の対応範囲・探索パス・検証 |
| [`openapi.json`](openapi.json) | Docker 版 REST API の仕様（生成物。`scripts/export-openapi.py` で再生成） |
| [`operations.md`](operations.md) | Docker 版の運用・バックアップ・復元 |
| [`zimaos.md`](zimaos.md) | ZimaOS / CasaOS コミュニティストアでの導入・更新 |
| [`release-checklist.md`](release-checklist.md) | バージョン公開作業の確認手順 |
| [`policy-spec.md`](policy-spec.md) | Desktop 版の監査ログ・`.mix/policy.toml` ポリシー契約 |
| [`memory-spec.md`](memory-spec.md) | Memory の共通契約 |
| [`desktop/memory-spec.md`](desktop/memory-spec.md) | 上とバイト一致の複製（`scripts/check-memory-parity.py` が検査） |
| [`desktop/architecture.md`](desktop/architecture.md) | Desktop 版の内部設計・セキュリティ・更新方法 |
| [`desktop/api-mapping.md`](desktop/api-mapping.md) | Docker 版 REST と Desktop 版 Tauri コマンドの対応表 |
| [`desktop/ui-conventions.md`](desktop/ui-conventions.md) | UI プリミティブと表記の統一基準 |
| [`desktop/update-signing.md`](desktop/update-signing.md) | アップデート署名鍵の生成・保管・ローテーション |
| [`desktop/coding-eval.md`](desktop/coding-eval.md) | コーディングエージェント評価ハーネス |
| [`desktop/ux-baseline.md`](desktop/ux-baseline.md) | Desktop UI の UX ベースライン |
| [`release-notes-v0.5.8.md`](release-notes-v0.5.8.md) | 最新リリースノート |

- `memory-spec.md` と `desktop/memory-spec.md` は**同一内容で置く**ことが CI の前提。片方だけ編集しないこと。
- リリースワークフロー（`desktop-release.yml`）が `docs/release-notes-<tag>.md` のパスを直接参照するため、サブディレクトリへの移動は不可。

## 配置制約（移動禁止の理由）

- `openapi.json` … `scripts/export-openapi.py`、`scripts/check-version-consistency.mjs`、`ci.yml` が参照
- `memory-spec.md` × 2 … `scripts/check-memory-parity.py` が 2 パス固定で比較
- `release-notes-*.md` … `desktop-release.yml` がタグからパスを組み立て
- `architecture.md` / `operations.md` / `zimaos.md` / `agent-skills.md` … `README.md` / `README.en.md` のドキュメント表が参照
