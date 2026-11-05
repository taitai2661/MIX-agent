# ドキュメント索引

`docs/` 直下と `docs/desktop/` の配置は CI・リリースワークフロー・parity 検査から参照されているため、**ファイルの移動・改名はしない**こと。詳細は「配置制約」を参照。

ドキュメントは役割で次の 4 種類に分かれる。

- **現行仕様** — 実装の正となる文書。挙動を変えたらこちらを更新する。
- **設計記録** — 実装済み機能の判断理由・経緯・代替案（`*-plan.md` を含む）。現行の挙動は必ず現行仕様を参照する。
- **計画** — 未着手または進行中の施策。フェーズ・受け入れ条件・計測方法を置き、実装が進んだら状態を更新する。
- **履歴** — リリースノートと `verification.md`。当時の記録として書き換えない。

## 読む順（はじめて触る場合）

1. [`../README.md`](../README.md) — セットアップと全体像
2. [`architecture.md`](architecture.md) — モノレポ構成と Docker 版アーキテクチャ
3. 対象エディションの仕様 — [`desktop/architecture.md`](desktop/architecture.md)、[`memory-spec.md`](memory-spec.md)
4. 開発・運用手順 — [`../CONTRIBUTING.md`](../CONTRIBUTING.md)、[`operations.md`](operations.md)
5. 経緯と検証 — [`verification.md`](verification.md)、[`release-notes-v0.5.6.md`](release-notes-v0.5.6.md)

## 全体・導入

| ドキュメント | 役割 | 内容 |
| :--- | :--- | :--- |
| [`architecture.md`](architecture.md) | 現行仕様 | プロジェクト全体の構成と Desktop / Docker の設計対比 |
| [`agent-skills.md`](agent-skills.md) | 現行仕様 | Agent Skills（`SKILL.md`）の対応範囲・探索パス・同梱ファイル・検証 |
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
| [`desktop/coding-eval.md`](desktop/coding-eval.md) | 現行仕様 | コーディングエージェント評価ハーネス（scripted / live、採点、シナリオ追加） |
| [`desktop/coding-agent-plan.md`](desktop/coding-agent-plan.md) | 設計記録 | コーディングエージェント化（Phase 8.1〜8.4 / 10.0 完了、10.1 以降を計画） |
| [`desktop/local-runtime-plan.md`](desktop/local-runtime-plan.md) | 設計記録 | ローカル LLM 連携の設計と経緯（L1〜L5 実装済み。現行挙動は `desktop/architecture.md` の「Local runtime management」） |
| [`desktop/chromium-on-demand-plan.md`](desktop/chromium-on-demand-plan.md) | 設計記録 | Chromium を同梱から初回インストールへ変えた理由と代替案（現行手順は `desktop/architecture.md` の「Managed Chromium」） |
| [`desktop/performance-plan.md`](desktop/performance-plan.md) | 計画 | 起動・描画・実行時処理・常駐リソースの軽量化（Phase 0〜5、計測と受け入れ条件） |
| [`desktop/agent-speed-plan.md`](desktop/agent-speed-plan.md) | 計画 | エージェント 1 ターンの高速化（コンテキスト予算・キャッシュ・検証段階化・ルーティング。品質ゲート付き） |

- Desktop 版の現行挙動は `desktop/architecture.md` に集約する。`*-plan.md` は決定の理由と経緯だけを残し、実装手順や現行仕様を重複して書かない。
- 計画書に「未決事項」を残したまま実装を完了させない。決めた内容は計画書へ追記し、現行仕様へ反映する。

## 戦略・計画

| ドキュメント | 役割 | 内容 |
| :--- | :--- | :--- |
| [`dsh-competitive-plan.md`](dsh-competitive-plan.md) | 計画 | DeepSeek Harness 対抗戦略と 6 ヶ月ロードマップ（Phase 0〜4、判断ゲート、KPI） |
| [`codex-quota-plan.md`](codex-quota-plan.md) | 設計記録 | ChatGPT/Codex 使用枠の非公式連携（Desktop/Docker、OAuth・クォータ・リスク） |

## リリースノート（`release-notes-vX.Y.Z.md`）

公開済み履歴として変更しない。リリースワークフロー（`publish-store.yml`、`desktop-release.yml`）が `docs/release-notes-<tag>.md` のパスを直接参照するため、サブディレクトリへの移動は不可。

| ファイル | 備考 |
| :--- | :--- |
| [`release-notes-v0.2.0.md`](release-notes-v0.2.0.md) | Docker 版のみの時代（初回のバージョン付きリリース） |
| [`release-notes-v0.2.2.md`](release-notes-v0.2.2.md) | Docker 版のパッチ更新 |
| [`release-notes-v0.2.3.md`](release-notes-v0.2.3.md) | Docker 版のパッチ更新 |
| [`release-notes-v0.3.0.md`](release-notes-v0.3.0.md) | Auto ルーティング・Model Info・Memory トレースほか |
| [`release-notes-v0.3.5.md`](release-notes-v0.3.5.md) | Memory Runtime・長時間 Agent・Projects・Skills ほか |
| [`release-notes-v0.4.0.md`](release-notes-v0.4.0.md) | 未公開のドラフト（アーカイブ。冒頭に明記） |
| [`release-notes-v0.5.0.md`](release-notes-v0.5.0.md) | Desktop 版追加・macOS DMG 配布開始 |
| [`release-notes-v0.5.1.md`](release-notes-v0.5.1.md) | macOS 署名修正・Windows x64 追加 |
| [`release-notes-v0.5.2.md`](release-notes-v0.5.2.md) | アプリ内アップデート・Linux 追加 |
| [`release-notes-v0.5.4.md`](release-notes-v0.5.4.md) | Chromium 初回インストール・ローカルLLM・ブラウザ ライブ表示 |
| [`release-notes-v0.5.5.md`](release-notes-v0.5.5.md) | 安定化（ブラウザ境界・秘密マスク・復元改善・実行ログ上限） |
| [`release-notes-v0.5.6.md`](release-notes-v0.5.6.md) | ChatGPT連携・LSP・Skills拡張・自動起動・Run取り消し・検索とページング |

`v0.5.3` は発行していません（`v0.5.2` の直後に `v0.5.4` を公開）。欠番は意図的で、`v0.5.3` のリリースノートは存在しません。

## ドキュメントを追加・更新するとき

- 新しい文書はこの索引の表に追加し、**役割**（現行仕様 / 設計記録 / 計画 / 履歴）を明記する。
- 相対リンクを追加したら `python3 scripts/check-doc-links.py` を通す。
- 現行仕様を変えたら、`*-plan.md` ではなく現行仕様を更新する。計画書には決定の理由と経緯だけを残す。
- `README.md` / `README.en.md` のドキュメント表に載る文書は、両方の表を同時に更新する（`scripts/check-readme-parity.py`）。
- `memory-spec.md` を変えたら `desktop/memory-spec.md` も同じ内容にする（`scripts/check-memory-parity.py`）。
- リリースノートは公開済みの履歴として書き換えない。`verification.md` は新しい節を先頭に追記する。

## 配置制約（移動禁止の理由）

- `openapi.json` … `scripts/export-openapi.py`、`scripts/check-version-consistency.mjs`、`apps/web/package.json`（`generate:api`）、`ci.yml` が参照
- `memory-spec.md` × 2 … `scripts/check-memory-parity.py` が 2 パス固定で比較
- `memory-roadmap.md` … `memory-spec.md` の冒頭と `apps/server/mix_agent/db/models.py` が参照
- `verification.md` … `README.md`、`CONTRIBUTING.md`、リリースノート各版が参照
- `release-notes-*.md` … `publish-store.yml` と `desktop-release.yml` がタグからパスを組み立て
- `architecture.md` / `operations.md` / `zimaos.md` / `agent-skills.md` … `README.md` / `README.en.md` の本文またはドキュメント表が参照
