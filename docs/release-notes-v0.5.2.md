# MIX-agent v0.5.2

> **Desktop版はベータです。** 予期しない不具合やデータ損失の可能性があります。重要なデータは必ずバックアップしてください。

v0.5.2は、Desktop版に署名検証付きのアプリ内アップデートを追加し、Linux x64（AppImage / deb）の配布を開始するリリースです。Desktop版は引き続きベータであり、macOSはApple Developer ID署名・公証なし（ad-hoc署名のみ）、Windowsインストーラーは未署名です。Docker／Web版の機能変更はありません。

## 主なハイライト

- **アプリ内アップデート**: 「設定 ▸ アップデート」から新バージョンの確認・ダウンロード・インストール・再起動ができます。起動時の自動確認（既定オン）と、ヘッダーの更新バッジを追加しました。
- **署名検証を必須化**: アップデータはビルド時に署名され、アプリは埋め込み公開鍵でダウンロードしたペイロードを検証します。署名が一致しない更新はインストールされません。
- **`latest.json` を公開**: GitHub Releasesに、macOS（Apple Silicon / Intel）、Windows x64、Linux x64の署名済みアップデータと更新マニフェストを添付します。
- **Linux x64版を追加**: `AppImage` と `deb` を配布します。AppImageはアプリ内更新に対応します。debはパッケージ管理下のため、GitHub Releasesから手動で更新してください。
- **配布前検査を強化**: 4プラットフォーム分のアップデータと `.sig` が揃わない場合、`latest.json` の生成を失敗させます。

## Desktop版の更新方法

### v0.5.2以降

「設定 ▸ アップデート」で「更新を確認」を押し、表示された新版をダウンロードしてインストールします。macOS / Linuxでは再起動後に適用されます。Windowsではインストーラー実行時にアプリが終了します。

### v0.5.1以前から更新する場合

v0.5.1以前にはアップデータが入っていないため、**v0.5.2だけは手動でインストール**してください。v0.5.2以降のバージョンからアプリ内更新を利用できます。

### インストーラー

- macOS: `MIX-agent-Desktop-Apple-Silicon.dmg`（arm64）／`MIX-agent-Desktop-Intel.dmg`（x86_64）
- Windows: `MIX-agent-Desktop-Windows-x64-Setup.exe`（NSIS）／`MIX-agent-Desktop-Windows-x64.msi`（WiX, en-US）
- Linux: `MIX-agent-Desktop-Linux-x64.AppImage`／`MIX-agent-Desktop-Linux-x64.deb`

ダウンロード後は `SHA256SUMS` と照合してください。

## ベータ版に関する注意

- **Desktop版はベータです。** 重要なデータは更新前にバックアップしてください。予期しない不具合、設定やデータの不整合が発生する可能性があります。
- macOSアプリはDeveloper ID署名・公証を行っていません。初回起動時にGatekeeperの「このまま開く」確認が必要です。
- Windowsインストーラーは未署名です。SmartScreenの警告が出る場合は「詳細情報」→「実行」を選択してください。
- アップデータ署名は、Apple／Windowsのコード署名とは別の仕組みです。Gatekeeper／SmartScreenの警告は残ります。
- Linux ARM64、macOS universal binary、Windows ARM64は配布対象外です。

## Docker / Web版

Docker／Web版の機能変更はありません。`v0.5.2`タグから従来どおりGHCRのAMD64イメージとZimaOSストア配布物を公開します。既存環境を更新する場合は、管理画面から暗号化バックアップを取得し、DB migration後の会話・設定・Provider情報を確認してください。サーバー版は単一管理者が信頼できるLAN/VPN内で利用する前提です。インターネットへ直接公開しないでください。
