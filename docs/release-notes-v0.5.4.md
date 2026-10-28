# MIX-agent v0.5.4

> **Desktop版はベータです。** 予期しない不具合やデータ損失の可能性があります。重要なデータは必ずバックアップしてください。

v0.5.4は、Desktop版のChromiumを同梱から初回インストール方式へ変更し、ローカルLLMランタイム管理とブラウザ自動操作のライブ表示を追加するリリースです。Docker／Web版はブラウザ操作の拡張とUIの改善を含みます。Desktop版は引き続きベータであり、macOSはApple Developer ID署名・公証なし（ad-hoc署名のみ）、Windowsインストーラーは未署名です。

## 主なハイライト

### Desktop版

- **Chromiumを初回インストール方式へ変更**: 配布物にブラウザ本体を含めなくなりました。macOS arm64のローカルビルドでは `.app` が約35.6 MiB（従来は展開後約569 MiB）になり、ダウンロードと更新が軽くなります。
- **設定 ▸ ブラウザから導入・更新・削除**: 進捗表示、キャンセル、再試行に対応しました。Chrome for Testing 156.0.8078.73を固定し、SHA-256とサイズを検証してから `<app-data>/chromium` へ展開します。検証後のアトミックな入れ替え、パストラバーサル・危険なsymlinkの拒否、インストール後の `--version` 起動確認を行います。
- **システムブラウザへのフォールバック**: 未導入でもシステムのChromium / Chrome / Edgeがあればブラウザ機能を利用できます。探索順は `MIX_CHROMIUM_HOME` → `<app-data>/chromium` → 旧バンドル → システムブラウザ → 無効です。
- **ローカルLLMランタイム管理**: Ollama / LM Studioの検出、起動・停止、モデルの取得・読み込み・削除、送信時の自動起動を追加しました。LAN上のランタイムを自動起動したり、コンテキスト長を勝手に制限したりしないよう、loopbackとLANを区別します。
- **ブラウザ操作の拡張とライブ表示**: `snapshot` + `ref` による要素指定、タブ操作、履歴、hover / press / select / check / extract / wait_for などを追加しました。設定画面のライブ画面からクリック・キー入力・キー送信ができます。
- **ブラウザ操作の境界を強化**: `navigate` / `new_tab` は `http` / `https` / `about:blank` のみ許可し、`file:` / `data:` / `javascript:` を拒否します。localhostなどへのアクセスは設定で明示的にONにした場合だけ許可し、クラウドmetadata（169.254.0.0/16）とlink-localはONでも拒否します。IPv4-mapped IPv6も同じ判定に落とします。
- **スケジュールとプロジェクトの改善**: スケジュール一覧にプロンプト・モード・モデル・次回実行時刻を表示し、削除せずに編集できるようにしました。ローカルモデルのコンテキスト長は設定した上限で制限します。

### Docker / Web版

- **ブラウザ操作を追加**: `browser_hover` / `browser_press` / `browser_select` / `browser_check` / `browser_reload` / `browser_back` / `browser_forward` / `browser_close` をPlaywrightランナーに追加しました。Desktop版とDocker版のアクション語彙は `packages/tool_contracts/browser.json` で共通化し、CIで不一致を検出します。
- **Chromium導入の通知**: Web版でバックグラウンド導入の開始・完了・失敗をトーストで通知します。
- **UI改善**: 実行アクティビティ、スケジュール、プロジェクト、MCP、Memory画面の表示とローディング／空状態を改善しました。

## Desktop版の更新方法

### v0.5.2以降

「設定 ▸ アップデート」で「更新を確認」を押し、表示された新版をダウンロードしてインストールします。

### v0.5.1以前から更新する場合

v0.5.1以前にはアップデータが入っていないため、手動でインストールしてください。v0.5.2以降のバージョンからアプリ内更新を利用できます。

### Chromiumの再導入について

v0.5.4ではブラウザを同梱しません。v0.5.2以前からアップデートした場合、同梱Chromiumは新しいアプリに引き継がれないため、初回のブラウザ利用時に「設定 ▸ ブラウザ」から導入するか、システムのChromium / Chrome / Edgeへフォールバックしてください。

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

`v0.5.4`タグから従来どおりGHCRのAMD64イメージとZimaOSストア配布物を公開します。既存環境を更新する場合は、管理画面から暗号化バックアップを取得し、DB migration後の会話・設定・Provider情報を確認してください。サーバー版は単一管理者が信頼できるLAN/VPN内で利用する前提です。インターネットへ直接公開しないでください。
