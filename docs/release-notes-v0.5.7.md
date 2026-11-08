# MIX-agent v0.5.7

> **Desktop版はベータです。** 予期しない不具合やデータ損失の可能性があります。重要なデータは必ずバックアップしてください。

v0.5.7は、v0.5.6以降の整理・保守リリースです。会話／記憶APIの分割、プロバイダー探索のモジュール分割、Desktopアプリ層の分割、サイドバーのスレッド整理、Autoルーティングの計測基盤（Phase 0〜1＋フォールバックの一部）をまとめています。Desktop版は引き続きベータであり、macOSはApple Developer ID署名・公証なし（ad-hoc署名のみ）、Windowsインストーラーは未署名です。

## 主なハイライト

### Desktop版

- **Autoルーティングの土台（Phase 0〜1＋Phase 2の一部）**: ターン単位の計測（TTFT / 所要時間 / 失敗種別）を `TurnPerformance` として記録し、`auto_model_stats` 集計テーブル（migration `0006_auto_stats.sql`）へUPSERTします。`ProviderErrorKind` による失敗分類（rate_limit / provider_5xx / timeout / auth / context_overflow等）をエンジン境界で付与し、`auto_priority`（balanced / speed / quality / cost）と `auto_fallback_attempts`（0〜2、既定1）設定を追加しました。最初の出力前の失敗に限り最大2件まで別モデルへ切り替えます。回路遮断器・ローカル起動待ち・選択理由UIは未着手です。
- **アプリ層の分割**: `app/src/lib.rs`（約2100行）を `adapters` / `app_run` / `providers` / `startup` / `state` / `tests` と `commands/conversations/` ディレクトリへ分割し、起動・状態・実行経路を見通せる形にしました。
- **プロバイダー探索の分割**: 単一ファイルの `discovery.rs`（約2200行）を `discovery/mod.rs`＋`scan` / `enrich` / `models` / `runtimes` / `install` / `urls` / `tests` に分割しました。挙動の変更はありません。
- **信頼性とエンジン計測**: `reliability.rs` の分類・クールダウン整理、`engine/execute_turn.rs` での計測点追加、`finish.rs` / `execute.rs` での運搬、`engine_loop` テストの更新を行いました。
- **サイドバーの整理**: スレッド一覧を `ThreadList` / `threadGroups` に切り出し、日時グループ表示とテスト（`Sidebar.test.tsx`、`ActivityPanel.test.tsx`）を追加しました。
- **設定UI**: `GeneralSettings` にAuto優先度の選択肢を追加し、i18n（en/ja）と設定バリデーションを揃えました。

### Docker / Web版

- **会話APIの整理**: `routes_conversations.py` から詳細系を `routes_conversation_detail.py` へ分割しました。一覧取得はPostgreSQL側でページングし、`limit` の既定を無制限（`None`）から上限500に変更しました。長時間運用でサイドバー読み込み時に全件を返す問題を防ぎます。停止Runに伴うブラウザ切断失敗は `LOGGER.debug` へ記録するのみとしました。
- **記憶APIの分割**: `memory/service.py`（約1400行）を `common` / `lifecycle` / `recall` / `links` に分割し、取得・保存・リンクの責務を分けました。`check-memory-parity.py` の対象に合わせ、Desktop/Webの仕様差分を検出できるようにしています。
- **Webサイドバー**: `AppSidebar` / `sidebarConfig` と `lib/conversations.ts`（＋単体テスト）を追加し、50件ずつのカーソルページングとFTS検索に合わせた取得へ寄せました。`e2e/conversations.spec.ts` を追加しています。
- **OpenAPI生成物の更新**: FastAPIバージョンを0.5.7へ上げ、`docs/openapi.json` を再生成しました（新設の詳細系エンドポイントを含む）。
- **運用・検証の追加**: `compose.yaml` の見直し、`scripts/check-async-db.py` の新設、マイグレーションのアップグレード試験（`test_migration_upgrade.py`）、エラー文言i18nチェックの拡張を行いました。
- **ストア配布物**: `scripts/render_zimaos_compose.py` に `v0.5.7` メタ（`update_at: 2026-11-08`）を追加し、`apps/MIX-agent/docker-compose.yml` を再生成しました。GHCRイメージタグはすべて `v0.5.7` です。

## ChatGPT連携について（重要）

ChatGPT／Codex連携はOpenAIの公開APIではなく、Codex CLIが使う非公開バックエンドを利用する**非公式**機能です。仕様変更やブロックにより予告なく動作しなくなる可能性があります。利用は自己責任で行い、組織のポリシーを確認してください。既定では無効で、設定で明示的に有効化した場合だけ表示されます。

## Desktop版の更新方法

### v0.5.2以降

「設定 ▸ アップデート」で「更新を確認」を押し、表示された新版をダウンロードしてインストールします。

### v0.5.1以前から更新する場合

v0.5.1以前にはアップデータが入っていないため、手動でインストールしてください。v0.5.2以降のバージョンからアプリ内更新を利用できます。

### Chromiumについて

ブラウザ機能の初回利用時は「設定 ▸ ブラウザ」からChrome for Testingを導入するか、システムのChromium / Chrome / Edgeへフォールバックしてください。

オフライン環境では、Chrome for Testingのアーカイブを `<app-data>/chromium` へ手動で展開するか、`MIX_CHROMIUM_HOME=/path/to/chromium` を指定してください。Linux arm64はChrome for Testingの配布対象外のため、システムブラウザのみ利用できます。

### インストーラー

- macOS: `MIX-agent-Desktop-Apple-Silicon.dmg`（arm64）／`MIX-agent-Desktop-Intel.dmg`（x86_64）
- Windows: `MIX-agent-Desktop-Windows-x64-Setup.exe`（NSIS）／`MIX-agent-Desktop-Windows-x64.msi`（WiX, en-US）
- Linux: `MIX-agent-Desktop-Linux-x64.AppImage`／`MIX-agent-Desktop-Linux-x64.deb`

ダウンロード後は `SHA256SUMS` と照合してください。

## ベータ版に関する注意

- **Desktop版はベータです。** 重要なデータは更新前にバックアップしてください。
- macOSアプリはDeveloper ID署名・公証を行っていません。初回起動時にGatekeeperの「このまま開く」確認が必要です。
- Windowsインストーラーは未署名です。SmartScreenの警告が出る場合は「詳細情報」→「実行」を選択してください。
- WindowsではAppData配下の `chrome.exe` に対してSmartScreen / Defenderが警告する場合があります。
- アップデータ署名は、Apple／Windowsのコード署名とは別の仕組みです。Gatekeeper／SmartScreenの警告は残ります。
- Linux ARM64、macOS universal binary、Windows ARM64は配布対象外です。

## Docker / Web版

`v0.5.7`タグから従来どおりGHCRのAMD64イメージとZimaOSストア配布物を公開します。既存環境を更新する場合は、管理画面から暗号化バックアップを取得し、DB migration後の会話・設定・Provider情報を確認してください。サーバー版は単一管理者が信頼できるLAN/VPN内で利用する前提です。インターネットへ直接公開しないでください。
