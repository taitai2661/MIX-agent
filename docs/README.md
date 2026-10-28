# ドキュメント索引

`docs/` 直下と `docs/desktop/` の配置は CI・リリースワークフロー・parity 検査から参照されているため、**ファイルの移動・改名はしない**こと。詳細は各節の「配置制約」を参照。

ドキュメントは役割で次の 3 種類に分かれる。

- **現行仕様** — 実装の正となる文書。挙動を変えたらこちらを更新する。
- **設計記録** — 実装済み機能の判断理由と経緯、および今後の計画（`*-plan.md`）。現行の挙動は現行仕様を参照する。
- **履歴** — リリースノートと `verification.md`。当時の記録として書き換えない。

## 全体・導入

| ドキュメント | 役割 | 内容 |
| :--- | :--- | :--- |
| [`architecture.md`](architecture.md) | 現行仕様 | プロジェクト全体の構成と Desktop / Docker の設計対比 |
| [`openapi.json`](openapi.json) | 現行仕様 | Docker 版 REST API の仕様（生成物。`scripts/export-openapi.py` で再生成） |
| [`operations.md`](operations.md) | 現行仕様 | Docker 版の運用・バックアップ・復元 |
| [`zimaos.md`](zimaos.md) | 現行仕様 | ZimaOS / CasaOS コミュニティストアでの導入・更新 |
| [`release-checklist.md`](release-checklist.md) | 現行仕様 | バージョン公開作業の確認手順 |
| [`verification.md`](verification.md) | 履歴 | 時点ごとの検証記録（新しい節を先頭に追記。履歴は書き換えない） |

## Memory（両エディション共通仕様）

| ドキュメント | 役割 | 内容 |
| :--- | :--- | :--- |
| [`memory-spec.md`](memory-spec.md) | 現行仕様 | Memory の共通契約（語彙・ライフサイクル・ランキング・tool 契約） |
| [`desktop/memory-spec.md`](desktop/memory-spec.md) | 現行仕様 | 上とバイト一致の複製（`scripts/check-memory-parity.py` が検査） |
| [`memory-roadmap.md`](memory-roadmap.md) | 設計記録 | Memory 強化ロードマップ（Phase 0〜6。完了済み。経緯の記録として保持） |

- `memory-spec.md` と `desktop/memory-spec.md` は**同一内容で置く**ことが CI の前提。片方だけ編集しないこと。
- ロードマップは完了済みフェーズの実施メモを含むため、新規仕様の置き場所には使わないこと。新規契約は `memory-spec.md` に追記する。

## Desktop 版（`desktop/`）

| ドキュメント | 役割 | 内容 |
| :--- | :--- | :--- |
| [`desktop/architecture.md`](desktop/architecture.md) | 現行仕様 | Desktop 版の内部設計（Rust クレート・セキュリティ・更新方法） |
| [`desktop/api-mapping.md`](desktop/api-mapping.md) | 現行仕様 | Docker 版 REST と Desktop 版 Tauri コマンドの対応表 |
| [`desktop/ui-conventions.md`](desktop/ui-conventions.md) | 現行仕様 | UI プリミティブと表記の統一基準 |
| [`desktop/update-signing.md`](desktop/update-signing.md) | 現行仕様 | アップデート署名鍵の生成・保管・ローテーション |
| [`desktop/coding-agent-plan.md`](desktop/coding-agent-plan.md) | 設計記録 | コーディングエージェント化（Phase 8.1〜8.4 / 10.0 完了、10.1 以降を計画） |
| [`desktop/local-runtime-plan.md`](desktop/local-runtime-plan.md) | 設計記録 | ローカル LLM 連携の設計と経緯（L1〜L5 実装済み。現行挙動は `architecture.md` の「Local runtime management」） |
| [`desktop/chromium-on-demand-plan.md`](desktop/chromium-on-demand-plan.md) | 設計記録 | Chromium を同梱から初回インストールへ変えた理由と代替案（現行手順は `architecture.md` の「Managed Chromium」） |

- Desktop 版の現行挙動は `desktop/architecture.md` に集約する。`*-plan.md` は決定の理由と経緯だけを残し、実装手順や現行仕様を重複して書かない。
- 計画書に「未決事項」を残したまま実装を完了させない。決めた内容は計画書へ追記し、現行仕様へ反映する。

## リリースノート（`release-notes-vX.Y.Z.md`）

公開済み履歴として変更しない。リリースワークフロー（`publish-store.yml`、`desktop-release.yml`）が `docs/release-notes-<tag>.md` のパスを直接参照するため、サブディレクトリへの移動は不可。

| ファイル | 備考 |
| :--- | :--- |
| `release-notes-v0.2.0.md` / `v0.2.2.md` / `v0.2.3.md` | Docker 版のみの時代 |
| `release-notes-v0.3.0.md` / `v0.3.5.md` | Memory Runtime・長時間 Agent ほか |
| `release-notes-v0.4.0.md` | 未公開のドラフト（冒頭に明記） |
| `release-notes-v0.5.0.md` | Desktop 版追加・macOS DMG 配布開始 |
| `release-notes-v0.5.1.md` | macOS 署名修正・Windows x64 追加 |
| `release-notes-v0.5.2.md` | アプリ内アップデート・Linux 追加 |
| `release-notes-v0.5.4.md` | Chromium 初回インストール・ローカルLLM・ブラウザ ライブ表示 |

## 配置制約（移動禁止の理由）

- `openapi.json` … `scripts/export-openapi.py`、`scripts/check-version-consistency.mjs`、`apps/web/package.json`（`generate:api`）、`ci.yml` が参照
- `memory-spec.md` × 2 … `scripts/check-memory-parity.py` が 2 パス固定で比較
- `memory-roadmap.md` … `memory-spec.md` の冒頭と `apps/server/mix_agent/db/models.py` が参照
- `verification.md` … `README.md`、`CONTRIBUTING.md`、リリースノート各版が参照
- `release-notes-*.md` … `publish-store.yml` と `desktop-release.yml` がタグからパスを組み立て
- `architecture.md` / `operations.md` / `zimaos.md` … `README.md` / `README.en.md` のドキュメント表が参照
