# MIX-agent v0.5.0

> **Desktop版を追加し、Docker／Web版と同じリポジトリ・GitHub Releaseから配布できるようにしたメジャーアップデートです。**

v0.5.0は、v0.3.5のセルフホスト版を土台に、ネイティブDesktop版、統合配布ワークフロー、モデル情報・Memory・Agent実行基盤の更新をまとめたリリースです。Docker版（FastAPI + PostgreSQL）とDesktop版（Tauri 2 + Rust + SQLite）は、同一リポジトリで管理し、DesktopアプリのバージョンとGitHub Releaseタグを`0.5.0`に統一しています。

## 主なハイライト

- **Desktop版を追加**: Tauri 2 + Rust + React 19で構成する、ローカル利用向けネイティブアプリを追加しました。Chat / Thinking / Agent、Provider、Tools、Browser、MCP、Memory、Knowledge、Skills、Projects、Schedulesなどを搭載し、OSトレイ・通知・deep link・OS Keychain連携に対応します。
- **macOS向けDesktop配布物**: v0.5.0ではApple Silicon（arm64）およびIntel（x64）Mac向け`.dmg`をGitHub Releaseから配布します。`SHA256SUMS`も添付し、ダウンロードしたファイルのチェックサムを照合できます。WindowsおよびLinux向けDesktop版は今回のリリース対象外です。
- **モデル情報の取得元を更新**: Context Window、価格、能力などのモデルメタデータを`models.dev`から取得します。取得に失敗した情報は未知として扱い、Providerから得た情報を優先します。
- **Memoryと長時間Agent実行を拡張**: Memoryの仕様を文書化し、関連・上書き・矛盾を表す`memory_links`、Projects、Runチェックポイント、予算延長、停滞検知、段階的なContext要約、回答検証ループを含めました。
- **配布経路を統合**: `v0.5.0`タグからDockerイメージ、ZimaOS用ストア配布物、Desktopインストーラーを同じGitHub Releaseへ集約するワークフローを用意しました。

## Desktop版

Desktop版は単一ユーザーのローカル利用を想定したアプリです。7つのRust crate（App、Core、Providers、Tools、Browser、MCP、Storage）とReact 19 UIで構成され、データはSQLiteに保存します。v0.5.0で配布するmacOS版はmacOS Keychainを利用します。WindowsおよびLinux向けDesktop版は今回のリリースには含まれません。

主な機能は次のとおりです。

- Chat / Thinking / Agentの実行、会話・Projects・Todoの管理
- OpenAI / Anthropic / GeminiなどのProviderとモデル選択、Capability Probe、Autoルーティング
- Tool Registryと権限ゲート、Browser Operator、Remote MCP、Skills、Knowledge、Memory
- Agentの実行予算・進捗表示、チェックポイントからの再開、停滞検知
- OSメニューからNew Chat／Settings／Backup／Diagnosticsを操作でき、OSトレイ・通知・`mix-agent-desktop://` deep link・バックアップ・Keychain連携にも対応
- UIの差分表示、スラッシュコマンド、`@`メンション、Tool実行のインライン表示

**更新について:** v0.5.0では自動更新、アプリ内の更新確認、Updater署名アーティファクトの生成を無効にしています。新しい版はGitHub Releasesから対応するmacOS用DMGを手動でダウンロードしてインストールしてください。公開するDMGには`SHA256SUMS`を添付します。

## Docker / Web版、Memory、Agentの変更

- **モデル情報**: `MIX_MODELS_DEV_URL`（既定値`https://models.dev/api.json`）で取得先を指定できます。旧`MIX_MODEL_INFO_URL`は非推奨エイリアスです。
- **Memory**: Memoryの役割・scope・lifecycle・evidence・conflict・revisionを仕様書にまとめました。`memory_links`により関連・上書き・矛盾の関係を表現し、仕様と実装のparityをCIで確認します。削除後も同一ownerのrevisionから、元のIDを保ったまま復元できます。
- **Agent実行**: Contextの予算可視化、段階的要約、耐久チェックポイント、任意のチェックポイントからのResume、予算延長フロー、停滞検知、検証ループを追加しました。
- **Desktop Terminal**: 永続PTYの完了・cwd marker同期とWindows Git Bash/MSYSのパス変換を修正しました。workspace外へ移動したshellのcwdを次のTool呼び出し前に戻し、workspace外へのパス操作を拒否します。
- **Projects / Todo / Skills**: Project文書をContextへ提供し、会話単位のTodo、Skill Discovery、`SKILL.md`検証、パッケージのインポート／エクスポートを追加しました。
- **Web UI / 運用**: Projects、Usage、Network、Diagnostics、Run Activity、Memory Mind Viewなどを追加・改善しました。利用量の履歴は本文や添付ではなく、モデル別のトークン数・価格など最小限の集計情報を扱います。
- **Migrations**: サーバー起動時にAlembicがDB migrationを適用します。今回含まれる主な変更は`0022_chatgpt_flat_memory.py`、`0024_memory_lifecycle.py`、`0025_memory_links.py`です（revision chainは`0022` → `0024` → `0025`で、`0023`は使用していません）。

## インストールとアップグレード

### Desktop版

[GitHub Releases](https://github.com/taitai2661/MIX-agent/releases)からApple SiliconまたはIntel Macに合う`.dmg`を取得してください。配布物と`SHA256SUMS`のチェックサムを確認してください。v0.5.0のmacOSアプリはApple Developer IDでコード署名せず、公証もしません。初回起動時にmacOS Gatekeeperの警告や起動制限が表示される場合があります。配布元とチェックサムを確認し、警告の意味を理解したうえで利用者自身の判断で起動してください。WindowsおよびLinux向けDesktop版はv0.5.0では配布しません。

### Docker版

従来のDocker Compose構成を利用できます。リリース後は対象バージョンのCompose設定またはリポジトリの`compose.yaml`を確認し、必要なイメージタグをそろえて更新してください。DB migrationは起動時に適用されます。

### ZimaOS

v2 Community Storeおよび旧クライアント向けアーカイブを従来の経路で公開します。手動Compose版とストア版ではデータ領域が異なる場合があり、既存データが自動的に引き継がれないことがあります。

## アップグレード前の注意

既存のサーバー版を更新する場合は、管理画面から暗号化バックアップを取得し、復元用パスフレーズを安全に保管してください。DB migration後、会話・設定・Provider情報が利用できることを確認してください。

## 既知の制約

- サーバー版は単一管理者が信頼できるLAN/VPN内で利用する初期開発版です。インターネットへ直接公開しないでください。
- Desktop版は初期リリースです。v0.5.0では自動更新を提供せず、更新はGitHub Releasesから手動で行ってください。macOSアプリ本体はApple Developer IDで署名せず、公証もしません。Gatekeeperが警告を表示したり、起動を制限したりする場合があります。配布元と`SHA256SUMS`を確認し、警告の意味を理解したうえで利用してください。
- v0.5.0で配布するDesktop版はApple Silicon / Intel macOSのみです。WindowsおよびLinux向けDesktop版は見送り、macOS universal binary、Windows ARM64、Linux ARM64も対象外です。
- Provider、MCP Server、Runner、Dockerイメージの実環境受入確認は利用環境に依存します。自動テスト・ビルド成功だけで実運用の安全性を保証するものではありません。
- Knowledgeはテキストのチャンク検索を中心とし、外部資料の自動同期や実環境での検索品質評価は今後の課題です。
- 画像生成・編集、Registry Store、外部向けOpenAI Compatible APIは未実装です。

## v0.3.5からの変更の流れ

- **v0.3.5**: Memory Runtime、Projects、長時間Agent向けチェックポイント／予算延長／停滞検知、Todo、Skills Discovery、Usage UI、Browser pauseを追加。
- **v0.5.0**: Desktop版を初めて同梱し、models.dev移行、Memory仕様と`memory_links`、Run Engine・配布ワークフローの更新を追加。

## 検証について

自動テスト、Lint、UI build、Rust checks、Docker Compose設定検証などの実施結果は、同梱の[`docs/verification.md`](verification.md)に追記します。実Provider・実Docker・各OS実機で未確認の項目は、確認済みとして扱いません。
