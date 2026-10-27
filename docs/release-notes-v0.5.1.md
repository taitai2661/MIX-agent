# MIX-agent v0.5.1

> **macOS版のインストール不能を修正し、Windows x64版を追加したDesktopアップデートです。**

v0.5.1は、公開済みv0.5.0のmacOS DMGでアプリを起動できなかった問題を修正し、Windows x64向けのNSIS／MSIインストーラーを追加したリリースです。Docker／Web版とZimaOS配布は従来どおり利用できます。

## 主なハイライト

- **macOSのインストール不能を修正**: `.app`バンドルにad-hoc署名を付与し、Apple SiliconでもダウンロードしたDMGからインストールして起動できるようにしました。v0.5.0のDMGはバンドル署名が欠けており、「壊れているため開けません」と表示されることがありました。
- **Windows x64版を追加**: `MIX-agent-Desktop-Windows-x64-Setup.exe`（NSIS）と`MIX-agent-Desktop-Windows-x64.msi`（WiX）を配布します。NSISは日本語／英語の言語選択とユーザー単位インストールに対応します。
- **Windowsターミナルを修正**: PowerShellフォールバック時の終了コードとcwdマーカー処理を修正し、永続PTYのテストをWindowsでも通るようにしました。
- **ショートカット表示をOS対応**: macOSでは`⌘`／`⇧`、Windowsでは`Ctrl`／`Shift`を表示します。キー入力は従来どおり`metaKey`と`ctrlKey`の両方を受け付けます。
- **配布前検査を強化**: macOSのリリースビルドで`codesign --verify --deep --strict`と、アプリ本体／同梱ChromiumのCPUアーキテクチャ一致を検査します。署名欠落や誤アーキテクチャの成果物は公開前に失敗します。

## Desktop版

### macOS

Apple Silicon（arm64）とIntel（x64）のDMGを配布します。

1. [GitHub Releases](https://github.com/taitai2661/MIX-agent/releases)からMacのアーキテクチャに合うDMGをダウンロードします。
2. `SHA256SUMS`とチェックサムを照合します。
3. DMGを開き、`MIX-agent Desktop`を`Applications`へドラッグします。
4. 初回起動時にGatekeeperの確認が表示された場合は、**システム設定 ▸ プライバシーとセキュリティ**で「このまま開く」を選択します。macOS 14以前は、アプリアイコンをControlキーを押しながらクリックして「開く」でも起動できます。

macOSアプリはApple Developer IDで署名せず、公証もしていません。Apple Siliconで起動できるようにad-hoc署名のみを付与しています。そのため初回起動時の確認は省略できません。

### Windows（x64）

Windows 10 1809以降またはWindows 11を対象に、次の2種類を配布します。

- `MIX-agent-Desktop-Windows-x64-Setup.exe`: NSIS。日本語／英語、ユーザー単位インストール。
- `MIX-agent-Desktop-Windows-x64.msi`: WiX（en-US）。組織配布向け。

WebView2が未導入の場合は、インストーラーのブートストラップが導入を試みます。インターネット接続が必要です。未署名のためSmartScreenが警告を表示する場合は、「詳細情報」→「実行」を選択してください。

ターミナルでPOSIXコマンドを使う場合は[Git for Windows](https://gitforwindows.org/)（Git Bash）を推奨します。未導入時はPowerShellへフォールバックします。

### 更新方法

v0.5.1では自動更新、アプリ内の更新確認、Updater署名アーティファクトの生成を無効にしています。新しい版はGitHub Releasesから手動でダウンロードしてインストールしてください。更新前に必要なデータのバックアップを取得してください。

## Docker / Web版

Docker／Web版の機能変更はありません。`v0.5.1`タグから従来どおりGHCRのAMD64イメージとZimaOSストア配布物を公開します。既存環境を更新する場合は、管理画面から暗号化バックアップを取得し、DB migration後の会話・設定・Provider情報を確認してください。

## 既知の制約

- macOSアプリはDeveloper ID署名・公証を行っていません。初回起動時にGatekeeperの確認が必要です。
- 修正版のIntel（x86_64）DMG、Windows 10／11実機でのインストール・初回起動・WebView2・Credential Manager・ターミナル・アンインストールは、このリリースノート作成時点では実機未確認です。GitHub Actionsのビルドと自動テストの結果は[`docs/verification.md`](verification.md)に記録します。
- Linux向けDesktop版、macOS universal binary、Windows ARM64、Linux ARM64は配布対象外です。
- サーバー版は単一管理者が信頼できるLAN/VPN内で利用する初期開発版です。インターネットへ直接公開しないでください。

## 検証について

自動テスト、Lint、UIビルド、Rust checks、macOS DMGの署名検証などの実施結果は[`docs/verification.md`](verification.md)に追記します。実機で未確認の項目を、ビルド成功や単体テストだけで確認済みとは扱いません。
