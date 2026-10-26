# MIX-agent v0.4.0

> Archived draft: v0.4.0 was not published. Use [`v0.5.0` release notes](release-notes-v0.5.0.md) for the release prepared from this source.

> **Desktop 版を同じリポジトリに統合し、モデルメタデータの取得元を models.dev へ移行したリリースです。**

v0.4.0は、v0.3.5で導入したMemory Runtime（役割分類・Evidence・Conflict）と長時間Agent実行基盤を土台に、Desktop 版（Tauri 2 + Rust）をモノレポへ統合し、リリース・ストア公開の自動化を成立させました。Docker 版（FastAPI + PostgreSQL）と Desktop 版（Rust + SQLite）は同一リポジトリで管理しますが、両者は共通の仕様書（`docs/memory-spec.md` / `docs/desktop/architecture.md`）で結ばれ、コードは共有しません。

既存データの migration は自動適用されます。**更新前に管理画面から暗号化バックアップを取得し、復元用パスフレーズを保管してください。**

## ハイライト

- **Desktop 版の統合と初配布**: Tauri 2 + Rust + React 19 のネイティブアプリ（`apps/desktop`）をモノレポへ取り込み。7つのRust crate とReact 19 UIで、Chat / Agent / Tool / Browser / MCP / Memory / Knowledge / Skills / Projects / Schedules を提供します。`.dmg` / `.msi` / `.exe` / `.deb` / `.AppImage` と `SHA256SUMS` を同じGitHub Releaseに添付します
- **モデルメタデータを models.dev へ移行**: 旧 `model-info` カタログを廃止し、`https://models.dev/api.json` 由来の Context Window・価格・能力を取得。`MIX_MODELS_DEV_URL` で差し替え可能で、`MIX_MODEL_INFO_URL` は非推奨のエイリアスとして残しています
- **Memory 仕様の明文化と `memory_links`**: 両エディション共通の仕様書 `docs/memory-spec.md` を追加し、Memory の語彙・ライフサイクル・ランキング・秘密情報の扱いを定義。関連・上書き・矛盾を表す `memory_links` を追加し、仕様と実装の乖離を CI で検出する parity ゲートを用意しました
- **Run Engine と API の整理**: Run の実行ループと定期実行・削除処理を `api/helpers/` へ切り出し、Run Engine 本体を見直し。会話・Run・期限切れデータの削除を `api/helpers/cleanup.py`、定期実行の消化を `api/helpers/scheduling.py` に集約しました
- **ストア公開の生成を一本化**: ZimaOS 用 Compose は常に `scripts/render_zimaos_compose.py` から生成します。タグごとのストアメタ（更新日・説明）をスクリプトの `RELEASE_META` に持ち、未登録のタグでは生成を失敗させます。手で編集して配布物がずれる経路を塞ぎました
- **Chromium の arch を明示**: 同梱 Chromium の取得を Chrome for Testing のプラットフォーム指定で固定し、Intel 版 / Apple Silicon 版が別 arch の Chromium を同梱しなくなりました

## 詳細な変更内容

### 1. Desktop 版（`apps/desktop`）

- **Cargo workspace**: `app`（Tauri 2 シェル）/ `core`（Run Engine・文脈・State）/ `providers`（OpenAI / Anthropic / Gemini の wire 実装）/ `tools`（Tool Registry + executor）/ `browser`（chromiumoxide backend）/ `mcp`（MCP client host）/ `storage`（SQLite + OS Keychain）の7 crate
- **React 19 UI**: `agents` / `browser` / `chat` / `files` / `knowledge` / `mcp` / `memory` / `onboarding` / `projects` / `providers` / `schedules` / `settings` / `shell` / `skills` / `system` / `tools` の各画面。Chat では Diff ビュー、スラッシュコマンド、`@` メンション、ツール実行のインライン表示に対応します
- **同梱 Chromium**: Chrome for Testing の zip を取得して同梱し、実行時はシステム Chromium へフォールバックしません。release ビルドは `MIX_REQUIRE_CHROMIUM=1` で fail-closed（browser-less 出荷を拒否）
- **モード別リミット**: Chat 12 ステップ / 20 分、Thinking 24 ステップ / 45 分、Agent 300 ステップ / 750 Tool Call / 90 分
- **OS 連携**: トレイ、メニュー、通知、deep link（`mix-agent-desktop://`）、バックアップ、Keychain 連携の Secret 保存
- **自動更新は未有効**: updater プラグインの基盤は用意されていますが、`plugins.updater.active = false` / `endpoints: []` のままなので、v0.4.0 の出荷版は更新を確認しません
- **版のライン**: Desktop 版は独立した版で、v0.4.0 の時点で `0.1.0` です。Docker 版と同じ Release に添付されますが、アプリ内の版番号は `0.1.0` のままです

### 2. Provider / モデル情報

- `providers/model_info.py` を廃止し、`providers/models_dev.py` を追加。取得先は `MIX_MODELS_DEV_URL`（既定 `https://models.dev/api.json`）で、ローカルに `api.json` を置いた場合はパスやファイル URL を指定できます
- 取得結果をモデル情報の evidence として保存し、Provider 自身の応答を優先したうえで不足分だけ補完する挙動を維持します
- 価格データの形が models.dev 基準（`cost`）に変わったため、`pricing.py` の正規化と `source` ラベル（`models_dev`）を対応付け
- 取得に失敗しても処理をブロックせず、当該モデルの補完データは未設定のまま扱います

### 3. Memory

- `docs/memory-spec.md` を追加し、Memory のデータモデル・語彙・ライフサイクル遷移・`memory_links`・検索とランキング・context への注入・Tool 契約・秘密情報の扱いを定義しました
- `memory_links`（仕様書 §4：関連・上書き・矛盾）を追加（migration `0025`）。4つのグラフ表（associations / features / conflicts / evidences）を1つの `memory_links` 表へ置き換えます
- 仕様書 §1 の8列（`lifecycle_state` / `superseded_by_id` / `disputed_by_id` / `confidence` / `salience` / `role` / `scope` / `source_kind`）を復元し、ライフサイクルの状態遷移（candidate / active / superseded / archived / expired）を明示的に扱います（migration `0024`）
- ChatGPT 風の平坦な Memory（`content` 列）に寄せる移行を挟み、その上の仕様書-compliant な表現へ戻し直しました（migration `0022` → `0024`）
- 仕様書と実装の比較を `scripts/check-memory-parity.py` で行い、CI の `memory-parity` ジョブで検査します

### 4. Run と API

- Run の実行ループを見直し、定期実行（Scheduled job）の実行と後始末を `api/helpers/scheduling.py` / `api/helpers/cleanup.py` へ切り出しました
- Run 予算・停滞検知・検証ループ・チェックポイント・文脈圧縮といった自律実行基盤は v0.3.5 から引き続き有効です

### 5. Web 版

- 空チャットの挨拶（時間帯・曜日・二十四節気・日本の祝日・最終活動・プロジェクト有無から文を合成）を追加。日本語原文と英訳を部品 ID で分離し、原文の言い回しを変えても英語表示が壊れません
- Memory の Trace ビューと関連（Associations）ビュー、Providers 画面へのテストを追加
- エラー表示・ローディング表示・トースト・初回案内（tour）のコンポーネントを追加

### 6. リリースとストア公開の自動化

- `desktop-release.yml`: macOS（aarch64 / x64）・Windows（msi / nsis）・Linux（deb / AppImage）の4ターゲットをビルドし、`SHA256SUMS` を生成して GitHub Release に添付します
- `validate-release.yml`: ストア用 Compose の生成漏れ（`git diff --exit-code`）、8種類の配布イメージのビルド、テストスイートをまとめて検証します
- `ci.yml`: Rust の3OSマトリクス（clippy `-D warnings` / `cargo test`）、Desktop UI の lint / typecheck / test / build、retrieval 品質ゲート（fixture suite の recall / MRR / noise）、memory parity ゲートを追加しました
- ストア用 Compose の生成は `scripts/render_zimaos_compose.py` が一元管理します。`update_at` とストア表示用の説明文はタグごとの `RELEASE_META` エントリで持ち、未登録のタグでは生成を失敗させます
- Chromium の取得先を matrix ごとに明示し、記録されたプラットフォームと対象 arch が一致することを `tauri build` 前に検証します

## データベース migration

起動時にAlembicが適用します。v0.4.0では以下を追加しています。

- `0022_chatgpt_flat_memory.py`: Memory を ChatGPT 風の平坦なリストへ移行（`content` 列を追加し、taxonomy 列と4つのグラフ表を削除）
- `0024_memory_lifecycle.py`: 仕様書 §1 の8列（ライフサイクル・役割・スコープ・確度など）を復元
- `0025_memory_links.py`: `memory_links`（仕様書 §4：関連・上書き・矛盾）

`0023` は使用していません（revision chain は `0022` → `0024` → `0025` で連続しています）。

## 過去バージョンからの変更の流れ

- **v0.2.0**: 初回の本格リリース。Chat / Thinking / Agent、50種類のProvider、Tool Registry、Knowledge、Skills、定期実行、Remote MCP OAuth、診断、暗号化バックアップ、Markdown書き出し、Web Pushを追加
- **v0.2.2**: バージョン情報、ZimaOSストアCompose、配布イメージ参照を統一
- **v0.2.3**: LAN内HTTPでのUUIDフォールバック、ストリーム切断後の再同期、同一添付ファイルの再選択、Memory関連表示を改善
- **v0.3.0**: Autoルーティング、Model Info、Capability Probe、Memoryトレース、Run予算可視化、Skillsパッケージ、Usage集計、設定画面刷新、PWAを追加
- **v0.3.5**: Memory Runtimeを役割分類・Evidence・Conflict・評価・ライフサイクルまで刷新。Projects、長時間Agent向けチェックポイント／予算延長／停滞検知、Todo、Skills Discovery、Usage UI、Browser pauseを追加
- **v0.4.0**: Desktop版をモノレポへ統合し初配布。モデルメタデータをmodels.devへ移行。Memory仕様書の明文化と`memory_links`の追加。Run EngineとAPIの整理。ストア公開の生成を一元化

## 配布物と更新

- **Docker 版**: `git pull --ff-only` のあと `docker compose up -d --build`。起動時にDB migrationが適用されます
- **Desktop 版**: GitHub Releasesから `.dmg`（Apple Silicon / Intel）、`.msi` / `.exe`、`.deb` / `.AppImage` を取得し、チェックサムは `SHA256SUMS` で確認できます
- **GitHub Container Registry**: `v0.4.0` タグで8種類のAMD64イメージを公開
- **ZimaOS Community Store**: v2ストアと旧クライアント向けv1アーカイブを従来の経路で公開
- **アプリケーション／MCP ClientInfo／Python／Web／OpenAPI／ZimaOS Compose のバージョンを `0.4.0` へ統一**

## 既知の制約

- v0.4.0時点でも、単一管理者が信頼できるLAN/VPN内で利用する初期開発版です。インターネットへ直接公開しないでください
- **macOS の Desktop 版は署名・公証されていません**。初回起動時に Gatekeeper の警告が出たら、アプリアイコンを「Control キーを押しながらクリック」→「開く」を選択するか、`xattr -cr "/Applications/MIX-agent Desktop.app"` を実行してください
- Desktop 版は α です。`aarch64-linux` / `universal-macos` / `win-arm64` は未対応で、自動更新も無効です
- 実Provider、実MCP Server、実Runner、Docker全イメージの受入確認は環境依存です。自動テスト・ビルド成功だけで実運用の安全性を保証しません
- ZimaOSストア版と手動Compose版ではデータ領域が異なる場合があり、既存データが自動的に引き継がれないことがあります
- Knowledgeはテキストのチャンク検索で、外部資料の自動同期と検索品質の実環境評価は今後の課題です
- 画像生成・編集、Registry Store、外部向けOpenAI Compatible APIは未実装です

## 検証状況

本リリースでは、Unit / Integration / Securityテスト、WebとDesktop UIのVitest、TypeScript／Vite build、Rustのclippy・test・fmt、ruff、生成物の再生成一致、Compose設定検証を実行します。実Provider・実Docker・実機で未実施の項目は、検証結果に明記します（`docs/verification.md` を参照）。
