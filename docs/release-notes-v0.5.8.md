# MIX-agent v0.5.8

> **Desktop版はベータです。** 予期しない不具合やデータ損失の可能性があります。重要なデータは必ずバックアップしてください。

v0.5.8は、v0.5.7以降の機能追加と hardening をまとめたリリースです。Desktop版ではヘッドレス実行（CLI）とセーフモード、OS標準WebViewへのブラウザ移行、コーディングツールとUIの拡充を行いました。Docker / Web版では、認証・通信制御・会話ページングの強化と、SPAを `app` が配信する構成への整理を含みます。Desktop版は引き続きベータであり、macOSはApple Developer ID署名・公証なし（ad-hoc署名のみ）、Windowsインストーラーは未署名です。

## 主なハイライト

### Desktop版

- **ヘッドレス実行（CLI）**: インストール済みバイナリでウィンドウを開かずに Run Engine を実行できます。`MIX-agent Desktop run -C ~/src/app --json --yes "失敗しているテストを直して"`（`mix-agent run` エイリアス可）のように使い、標準入力からタスクを渡すこともできます。`--json` は Run イベントを NDJSON で出力し、末尾に `result` を1行出します。`--model` / `--provider` の指定、`--yes`（承認の自動化）、`--keep`（会話・Run の保持）に対応し、終了コードは `0` 成功 / `1` 失敗 / `2` 引数エラー / `130` キャンセルです。
- **`--doctor` と `mixd serve`**: `--doctor` はデータディレクトリの書き込み可否、DBの schema version と `quick_check`、秘密情報バックエンドの到達性を読み取り専用で確認します（`--json` で機械可読）。`mixd serve` はヘッドレスなHTTP/SSEブリッジで、トークン（`MIX_SERVE_TOKEN` / 0600の `serve.token`、既定では `--token` なし）、Origin／Host検査、承認の inbox・決定・回答・キャンセル・Runイベント・Scheduled Job のルートを提供します。
- **セーフモード**: `--safe-mode` で起動すると、MCP接続・ブラウザツール・スケジュール・Spotlight索引・ローカルランタイムの自動起動を無効化します。壊れたMCP設定や常駐プロセスで起動できなくなったときの復旧用で、起動中はヘッダーにバナーが表示されます。
- **ブラウザはOS標準WebViewへ移行**: WindowsはWebView2、macOSはWKWebView、LinuxはWebKitGTKのネイティブ子WebViewを使用します。Chrome for Testingの同梱・初回ダウンロード・固定ピン、`MIX_CHROMIUM_HOME` による手動配置はすべて廃止されました。ダウンロードやシステムChromeへのフォールバック手順は不要です。
- **Autoルーティングの選択理由とフィードバック**: `autoSelection.reason` を `auto_reason` としてメッセージ表示へ渡し、モデル名の隣の `Auto` バッジ（ツールチップ／読み上げ名）で理由を確認できます。Auto回答のメッセージに 👍／👎 を付けられるようになり、評価は経過時間を考慮した集計（`message_feedback`）に反映されます。共有回路遮断器、ローカルランタイムの起動待ちは未実装です。
- **ターミナルと承認の強化**: 右パネルの「ターミナル」から永続PTYシェルを直接操作でき、出力が実行中からストリーミング表示され、`Ctrl-C` で停止できます。設定 `sandbox_terminal` でシェルをOS機能により隔離できます（macOSは `sandbox-exec`、Linuxは `bubblewrap`／`firejail`、WindowsはJob Objectによるプロセスツリー隔離のみで、ファイルシステムの書き込み隔離は未対応）。`rm -rf /`・`curl … | sh`・`sudo`・`git push --force`・認証情報の読み出しなどの危険なコマンドは二段階承認となり、「常に許可」には保存できません。
- **コーディングツールとUIの拡充**: ユーザーへ質問する `ask_user` と `QuestionCard`、計画ツール、メッセージ評価（`POST /messages/{key}/feedback`）、実行中の追加指示（Steering Queueの表示・編集・削除）、Tool呼び出し履歴（Rust側で redact、最新2000件）、Skillのrevision一覧・復元、Skillの一意制約マイグレーションを追加しました。音声入力（Web Speech API）、Problemsパネル（LSP診断）、通知のクリックで対象へ移動、コマンドパレットの使用履歴、設定項目の検索アンカー、ANSI描画、UXイベント計測（ローカル完結）、デザイントークン／a11yゲート、コーディングエージェント評価ハーネスも整備しています。
- **監査ログとポリシー契約**: ハッシュチェーンで改ざんを検出する追記専用の `audit_log`（migration `0008`）、保持期間のアーカイブ・削除経路、`.mix/policy.toml` のポリシー契約（`docs/policy-spec.md`）を追加しました。起動時のチェーン検証はまだ配線されておらず、DBは平文です。
- **マルチウィンドウ**: 「ファイル ▸ 新しいウィンドウ」（`⌘⇧N` / `Ctrl+Shift+N`）、トレイ、コマンドパレットから2つ目のウィンドウを開けます。バックエンドは共有されるため、別々のスレッドを並べて比較できます。
- **DB暗号化の状態を明確化**: 既定は所有者のみ読み書き可能な平文SQLite（0600）です。`db.master` の生成経路はありますがSQLCipherは接続に配線されていないため、設定・診断画面は「このビルドでは未対応」と表示し、`db_encrypted` は常に `false` です。保存データの保護はOSのディスク暗号化（FileVault / BitLocker / LUKS）に依存します。

### Docker / Web版

- **SPA配信の整理**: Docker Compose の `app` サービスがビルド済みSPAを配信する構成になり、`web` コンテナを削除しました。
- **Providerプリセットを追加**: 52種類から53種類へ拡充しました。
- **認証・通信制御の強化**: 総当たり試行の回数をDB（`attempt_counters`、migration `0029_attempt_counters`）で管理し、再起動や複数ワーカーをまたいで維持します。リバースプロキシ配下では `MIX_TRUSTED_PROXIES`（`FORWARDED_ALLOW_IPS` も可）で信頼するプロキシを指定し、ログイン抑制と `/metrics` のアクセス判定に正しいクライアントIPを使います。OAuthのCIMDメタデータはHTTPSかつ同一オリジンに限定し、リダイレクトを追跡しないようにしました。承認時のリスク確認（確認なしの許可は422、高リスクの「常に許可」は拒否）、通信許可ドメインの厳格検証（IPリテラルは拒否）、`network_mode=none` の尊重、MCPごとの `source policy` がグローバル設定より優先される挙動を明文化しています。
- **会話データの取得とバックアップ**: メッセージとTool呼び出しは最新ページを上限付きで取得し、対応するRunも同じページに揃えました。承認時のリスクAck、プロジェクトドキュメント読み出しのブロッキングプールへの移動、バックアップの行数事前カウントと上限（256MB）早期拒否、バッチ読み出し、`MIX_KEY_VERSION` による暗号化キーのバージョン管理に対応しました。
- **コンテナ構成**: ホストのDocker Socketは直接マウントせず、隔離された `docker-socket-proxy` 経由でMCP管理コンテナのみがアクセスします。検証ログや計画書の文書を削除し、現行仕様へ集約しました。

## ChatGPT連携について（重要）

ChatGPT／Codex連携はOpenAIの公開APIではなく、Codex CLIが使う非公開バックエンドを利用する**非公式**機能です。仕様変更やブロックにより予告なく動作しなくなる可能性があります。利用は自己責任で行い、組織のポリシーを確認してください。既定では無効で、設定で明示的に有効化した場合だけ表示されます。

## Desktop版の更新方法

### v0.5.2以降

「設定 ▸ アップデート」で「更新を確認」を押し、表示された新版をダウンロードしてインストールします。

### v0.5.1以前から更新する場合

v0.5.1以前にはアップデータが入っていないため、手動でインストールしてください。v0.5.2以降のバージョンからアプリ内更新を利用できます。

### ブラウザについて

ブラウザ機能はOS標準のWebViewで動作します。WindowsはWebView2ランタイム、macOSはWKWebView、LinuxはWebKitGTKが必要です。Chrome for Testingのダウンロード・固定ピン・初回インストールは不要で、「設定 ▸ ブラウザ」からの導入操作や`<app-data>/chromium`への手動配置、`MIX_CHROMIUM_HOME`の指定は不要です（いずれも廃止済み）。

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
- Desktop版のSQLiteは既定で平文です。SQLCipherは未配線のため、OSのディスク暗号化を併用してください。
- Linux ARM64、macOS universal binary、Windows ARM64は配布対象外です。

## Docker / Web版

`v0.5.8`タグから従来どおりGHCRのAMD64イメージとZimaOSストア配布物を公開します。既存環境を更新する場合は、管理画面から暗号化バックアップを取得し、DB migration後の会話・設定・Provider情報を確認してください。`web` コンテナが不要になったため、独自に `web` サービスを運用していた場合は構成を見直してください。サーバー版は単一管理者が信頼できるLAN/VPN内で利用する前提です。インターネットへ直接公開しないでください。
