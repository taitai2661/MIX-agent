# MIX-agent v0.5.6

> **Desktop版はベータです。** 予期しない不具合やデータ損失の可能性があります。重要なデータは必ずバックアップしてください。

v0.5.6は、Desktop版・Docker／Web版に共通する機能リリースです。ChatGPT／Codex使用枠の非公式連携、言語サーバーによる診断、Agent Skillsの同梱ファイル対応、ログイン時起動、Git連携のRun取り消し、会話の全文検索とページング、UIの高速化を追加しました。Desktop版は引き続きベータであり、macOSはApple Developer ID署名・公証なし（ad-hoc署名のみ）、Windowsインストーラーは未署名です。

## 主なハイライト

### Desktop版

- **ChatGPT／Codex使用枠の非公式連携**: 設定 ▸ ChatGPT からデバイスコードでログインし、サブスクリプションの使用枠（5時間／週次）を確認できます。通常のProvider一覧には表示されず、接続は1つだけです。非公式・自己責任の機能として明示的に有効化した場合のみ利用できます。
- **言語サーバー（LSP）ツール**: `lsp` ツールが `diagnostics`（型エラー・lint）と `definition`（宣言へジャンプ）を提供します。`rust-analyzer` / `typescript-language-server` / `pyright-langserver`・`pylsp` / `gopls` をPATHから自動検出し、未導入でも他の機能は動作します。`MIX_LSP_*` で実行ファイルを指定できます。
- **Agent Skillsの拡張**: `license` / `compatibility` / `metadata` / `allowed-tools` を保持したまま `SKILL.md` を入出力できます。`.mix/skills` / `.claude/skills` / `.agents/skills` をプロジェクトとホームから探索し、同梱の `scripts/` / `references/` / `assets/` は `skill_resource` で必要時に読み取ります（スクリプトは自動実行しません）。配布zipの展開サイズ上限と暗号化エントリ拒否も追加しました。
- **ログイン時起動とトレイ常駐**: 設定 ▸ 一般の「ログイン時に起動」でOSの自動起動に登録します（macOSはLaunchAgent、WindowsはRunキー、LinuxはXDG autostart）。起動時は `--hidden` でトレイに常駐し、スケジュールと実行中のRunを継続します。
- **Git連携のRun取り消し**: Run開始時の作業ツリー（未コミット変更を含む）を `refs/mix-agent/runs/<id>` に固定し、「このRunの変更を取り消す」で `terminal` が作成・変更したファイルも実行前へ戻せるようになりました。Git管理外のフォルダでは従来どおり `edit` / `files` の変更だけが対象です。
- **会話のページングと全文検索**: サイドバーは50件ずつのカーソルページングで読み込み、SQLite FTS5（trigram）でメッセージ・ナレッジ・記憶を検索します。長いトランスクリプトは可視領域だけを描画します。
- **ヘッドレス診断**: インストール済みバイナリで `--doctor` / `--doctor --json` / `--version` / `--help` を利用できます。データディレクトリ、DBのschema versionと `quick_check`、秘密情報バックエンドを読み取り専用で確認します。
- **秘密情報の保存方式**: API Keyや認証情報はAES-256-GCMで暗号化してローカルに保存します。マスター鍵はWindowsではCredential Manager、macOS / Linuxでは所有者のみ読める0600の`.master_seed`に保管します。macOSのad-hoc署名ビルドではKeychainのACLが再ビルドごとに一致せず許可ダイアログが出続けるため、既定ではKeychainを読みません（旧版のKeychain seedは初回起動時に一度だけ`.master_seed`へ移行）。`MIX_SECRET_BACKEND=keychain-seed`でKeychain方式に切り替えられます。
- **パフォーマンス改善**: 起動時のバックアップ／整合性チェックを必要時のみに限定し、ブラウザのアイドル終了を5分に短縮、UIの描画と検索を軽量化しました。

### Docker / Web版

- **ChatGPT／Codex連携**: Provider設定で非公式連携を有効化すると、デバイスコードのログイン、使用枠の取得、ログアウトをWeb UIから行えます。トークンは既存の暗号化Secretストアだけに保存し、応答やログには出しません。
- **Provider更新の修正**: `PATCH /providers/{id}` が部分更新として動作し、名前だけの変更でBase URL・追加設定・LAN接続許可・レート制限が消えなくなりました。
- **Agent Skillsの強化**: 仕様に沿ったメタデータ保持、探索パスの拡張、同梱ファイルの読み取り、zip-bomb対策をDesktop版と共通化しました。
- **検索とEmbeddingの改善**: 記憶・ナレッジの候補取得をFTS5で絞り込み、ランキングの品質ゲート（recall / MRR / noise）を両エディションで共通のfixtureに対して実行します。
- **APIレスポンスの一貫性**: ChatGPT Providerの `has_secret_id` を常に返すようになり、英語UIのエラーコード対応表とOpenAPI生成物を更新しました。

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

`v0.5.6`タグから従来どおりGHCRのAMD64イメージとZimaOSストア配布物を公開します。既存環境を更新する場合は、管理画面から暗号化バックアップを取得し、DB migration後の会話・設定・Provider情報を確認してください。サーバー版は単一管理者が信頼できるLAN/VPN内で利用する前提です。インターネットへ直接公開しないでください。
