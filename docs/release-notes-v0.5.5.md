# MIX-agent v0.5.5

> **Desktop版はベータです。** 予期しない不具合やデータ損失の可能性があります。重要なデータは必ずバックアップしてください。

v0.5.5は、Desktop版・Docker／Web版に共通の安定化リリースです。ブラウザ通信の境界強化、ツール出力の秘密情報マスク、バックアップと復元の改善、実行ログの上限対応、API誤差形式の統一など、セキュリティと運用性を中心に改善しました。Happy Halloween! 🎃 Desktop版は引き続きベータであり、macOSはApple Developer ID署名・公証なし（ad-hoc署名のみ）、Windowsインストーラーは未署名です。

## 主なハイライト

### Desktop版

- **ファイルアクセスの信頼境界を明確化**: 未許可のパスへの読み書き・保存はネイティブダイアログで確認するようになりました。プロジェクトルートは永続許可、その場限りの許可は12時間有効です。
- **ブラウザを常時ヘッドレス化＋ライブ表示を改善**: ライブ画面はCDPスクリーンキャストによるストリーミング表示になり、タブ切り替え・進む／戻る・停止に対応しました。
- **ブラウザ通信の境界を強化**: CDPレベルで全リクエストの宛先を解決し、プライベート／loopback／link-local／クラウドmetadataへの通信とDNSリバインドを拒否します。
- **ツール出力の秘密情報をマスク**: `API_KEY`・`token`・`secret`・`password`形式の代入や既知の鍵プレフィックス（`sk-`、`ghp-`、`AKIA`、JWTなど）、実行中の環境変数の値を `[redacted]` に置き換えます。
- **編集検証の修正**: バッチ編集の検証台帳が対象ごとに正しく作動し、成功した編集だけが検証ゲートを有効化します。`files.read` の行番号プレフィックスがアンカーから除去されます。
- **実行ログの上限対応**: 5万件を超える実行ログは先頭＋末尾を残して省略し、`replay_truncated` として省略件数をUIに表示します。
- **ファイル選択・確認ダイアログの非同期化**: ダイアログ表示中に関連のない操作が止まらなくなりました。
- **バックアップのパスワード方針**: 新規バックアップは12文字以上のパスワードが必須になりました。復元時は従来のパスワードも受け付けます。
- **プラットフォーム別UX**: ショートカット表記や更新案内がOSごとに切り替わります。`database_encryption=true` は未同梱（SQLCipherなし）のため明示的に拒否されます。

### Docker / Web版

- **API誤差形式を統一**: エラー応答は `{detail, code}` 形式に統一され、英語UIとコード対応表がCIで検査されます。
- **監視とログ**: Prometheus形式の `/metrics`（トークンまたはloopback／プライベートのみ）とJSON構造化ログ（`MIX_LOG_FORMAT=json`、`X-Request-ID`）を追加しました。Composeのヘルスチェックは依存のない `/live` を使います。
- **Egress方針を明示化**: `{mode: public_web|restricted|none, allowed_domains}` 形式になり、設定更新・復元・起動時のポリシー再構成が安全になりました。
- **プロキシ対応**: `MIX_TRUSTED_PROXIES` で信頼したプロキシからの `X-Forwarded-For` のみ採用し、レート制限と `/metrics` の到達判定が正しく動きます。
- **会話タイトルの修正**: 「新しいチャット」への改名が未編集扱いにならず、添付のみの初回メッセージに `添付について` と付きます。
- **明示の記憶（覚えて／忘れて）の分離**: 記憶の確定はRun確定後のベストエフォートで行い、応答済みメッセージが500エラーになりません。
- **スキル配布のzip-bomb対策**: 申告・実測の展開サイズ上限（16 MiB）と暗号化エントリ拒否を追加しました。
- **バックアップ復元の堅牢化**: 復旧用パスフレーズを暗号化保存し、中断ジャーナルの旧形式にも対応します。
- **DBインデックスの追加**: セッション・監査・実行ログ・メッセージの複合インデックスで一覧系の応答を改善しました（migration `0027` / `0028`）。
- **Web UIの改善**: エラー表示の統一、タイムアウト修正、ESLint／Prettierゲート、Playwright E2E（smoke＋chat SSE）を追加しました。

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

`v0.5.5`タグから従来どおりGHCRのAMD64イメージとZimaOSストア配布物を公開します。既存環境を更新する場合は、管理画面から暗号化バックアップを取得し、DB migration後の会話・設定・Provider情報を確認してください。サーバー版は単一管理者が信頼できるLAN/VPN内で利用する前提です。インターネットへ直接公開しないでください。
