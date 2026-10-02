# MIX-agent v0.3.5

> **Memory Runtime刷新と、長時間Agent作業・Projects・Skills・運用性をまとめて強化するメジャー機能リリースです。**

v0.3.5は、v0.3.0で導入したAutoルーティング／Model Info／Memoryトレース／利用量集計を土台に、以降の完成済み実装を正式なリリースとしてまとめたものです。既存のMemoryデータはmigrationで新しい分類へ移行され、従来のMemory APIとツールも互換レイヤーを通じて継続利用できます。

## ハイライト

- **Memory機能を一新**: Fact / Preference / Goal / Decision / Failure / Experienceの役割分類、scope、ライフサイクル、確度・顕著性・強度、Evidence、Conflict、Revisionを導入。
- **Memory Runtime**: `recall` / `remember` / `forget` を中心に、重複・矛盾の評価、関連Memoryの再活性化、Decision / Failure優先の想起、Task後の非同期Memory形成を追加。
- **Memory UI刷新**: 役割別のMind View、Evidence・関連・競合・置換履歴の詳細表示、競合解決、検索デバッグ、明示的なRememberフォームを追加。
- **Projects**: 会話をProject単位で整理し、Project文書とProjectコンテキストをAgentへ供給。
- **長時間Agent作業**: 動的予算延長、停滞検知、段階的Context要約、耐久チェックポイント、任意のチェックポイントからのResume、回答検証ループを追加。
- **Tasks / Todo**: 会話内Todoの作成・更新・完了と、Run状態・イベント・UI表示を統合。
- **Skillsパッケージ**: `SKILL.md` の検証・解析・レンダリング、zip入出力、ホストディレクトリからのDiscovery、API経由のインポート／エクスポートを追加。
- **Model / Provider運用**: Capability Probe、Model Info、品質ロール、価格情報、Autoルーティングを整理し、未知の能力は安全に`unknown`として扱う設計を強化。
- **Usage / Statistics**: 本文を保存せず、モデルごとのトークン数・価格・期間集計を表示。
- **Projects・PWA・設定画面**: Projects画面、PWAアイコン／manifest、Network・Usage・Diagnostics・設定ナビゲーションを追加・刷新。
- **ブラウザと実行状態**: Browser pause、Runイベントの型安全な表示、ストリーム切断後の再同期、途中状態の永続化を改善。

## 詳細な変更内容

### 1. Memory Runtimeとデータモデル

- Memoryの役割を `fact`、`preference`、`goal`、`decision`、`failure`、`experience` に分類。
- `user`、`project`、`task`、`session` のscopeと、candidate / active / superseded / archived / expiredのライフサイクルを導入。
- Agent / user / importedなどのsource種別、human / system / unverifiedのverification、task紐付けを追加。
- Memory Evidence（根拠・参照・要約・取得時刻）とMemory Conflict（競合ペア・理由・解決状態）を追加。
- 重複時のreinforce、矛盾時のconflict生成、置換時のrevision、想起時のco-activationを実装。
- 旧来のassociative Memory行を新しい語彙へバックフィルし、既存データを破壊的に削除せず移行。
- Memory Toolは生のCRUDではなくRuntimeを経由し、既存の`memory_search` / `memory_add`等も互換維持。

### 2. Agent実行ループとContext

- Contextをsystem / task state / summary / recent conversation / memory / skills / knowledge / toolsへ分割し、予算を可視化。
- 大きなTool出力や長い会話をTier 1 / Tier 2の段階的要約で扱い、要約失敗時は履歴を黙って破棄せず停止。
- RunCheckpointを保存し、長時間Runを途中状態から再開可能にしました。
- 予算枯渇時の`budget_extension_pending`、利用可能な延長回数、承認・拒否フローを追加。
- 進捗が停滞したRunを検出し、検証・再試行・回答チェックの状態を保存。
- Browser pause、Unknown action、SSE再接続後のメッセージ／Tool履歴再同期を改善。

### 3. Projects、会話、Todo

- Projectの作成・編集・アーカイブと、会話のProject紐付けを追加。
- Project文書を登録・検索し、会話のContextへ関連文書を挿入。
- Todoの作成・更新・完了・並び替えと、Agent Loopのイベント表示を追加。
- 会話一覧の検索・整理・アーカイブ・ごみ箱運用を強化。

### 4. SkillsとMCP

- SkillのDiscovery、`SKILL.md` metadata検証、zipパッケージ化・展開・インポート・エクスポートを追加。
- Skill履歴・復元とProject／Contextへの統合を整理。
- MCP Registry、OAuth、Filesystemテンプレート、権限確認を従来どおり利用できます。
- Tool Registryのfingerprint、call scope、rule scope、Allow / Ask / Deny判定を堅牢化。

### 5. Provider、Model、利用量

- Tool Calling / Vision / Reasoningの副作用のないCapability Probeを追加。
- Model Info静的カタログからContext Window、価格、能力、マルチモーダル情報を取得し、失敗時は未知として継続。
- 役割別Model選択、品質ロール、Reliability learningによる再試行・Autoルーティングを改善。
- 1回の呼び出しについて本文・添付・Tool結果を保存せず、トークン数と価格だけを90日保持。
- SettingsのUsage／Statisticsで期間別・モデル別の利用状況を確認可能。

### 6. UI、PWA、運用

- Memory Mind View、Projects、Run Budget、Run Activity、Todo Pane、Network、Usage、Diagnosticsを追加。
- Settings／Overview／Provider／Model／Setup／Chatの導線と状態表示を刷新。
- PWA manifest、favicon、各サイズのアイコン、テーマ・アクセント・Toggle・Select Menuを追加。
- 暗号化バックアップ、復元前退避、復元途中の再起動回復、DB／Runner／Run診断を継続。

## データベース migration

起動時にAlembicが適用します。v0.3.5では以下を追加しています。

- `0017_projects.py`: Projectテーブル
- `0018_browser_pause.py`: Browser pause状態とRun排他Index
- `0019_model_usage.py`: プライバシー最小限のModel Usage履歴
- `0020_run_checkpoints.py`: 予算延長状態とRunCheckpoint
- `0021_memory_role_taxonomy.py`: Memory役割分類、Evidence、Conflict、旧データのバックフィル

**更新前に管理画面から暗号化バックアップを取得し、復元用パスフレーズを保管してください。**

## 過去バージョンからの変更の流れ

- **v0.2.0**: 初回の本格リリース。Chat / Thinking / Agent、50種類のProvider、Tool Registry、Knowledge、Skills、定期実行、Remote MCP OAuth、診断、暗号化バックアップ、Markdown書き出し、Web Pushを追加。
- **v0.2.2**: バージョン情報、ZimaOSストアCompose、配布イメージ参照を統一。アプリ本体の機能変更はなし。
- **v0.2.3**: LAN内HTTPでのUUIDフォールバック、ストリーム切断後の再同期、同一添付ファイルの再選択、Memory関連表示を改善。
- **v0.3.0**: Autoルーティング、Model Info、Capability Probe、Memoryトレース、Run予算可視化、Skillsパッケージ、Usage集計、設定画面刷新、PWA、安定性・性能改善を追加。
- **v0.3.5**: v0.3.0の基盤を拡張し、Memory Runtimeを役割分類・Evidence・Conflict・評価・ライフサイクルまで刷新。Projects、長時間Agent向けチェックポイント／予算延長／停滞検知／段階的要約、Todo、Skills Discovery、Usage UI、Browser pauseを追加。

## 配布物と更新

- Docker Compose: `docker compose up -d --build`
- ZimaOS Community Store: v2ストアと旧クライアント向けv1アーカイブを従来の経路で公開。
- GitHub Container Registry: `v0.3.5`タグで8種類のAMD64イメージを公開。
- アプリケーション／MCP ClientInfo／Python／Web／OpenAPI／ZimaOS Composeのバージョンを`0.3.5`へ統一。

## 既知の制約

- v0.3.5時点でも、単一管理者が信頼できるLAN/VPN内で利用する初期開発版です。インターネットへ直接公開しないでください。
- 実Provider、実MCP Server、実Runner、Docker全イメージの受入確認は環境依存です。自動テスト・TypeScript build成功だけで実運用の安全性を保証しません。
- Knowledgeはテキストのチャンク検索で、外部資料の自動同期と検索品質の実環境評価は今後の課題です。
- 画像生成・編集、Registry Store、外部向けOpenAI Compatible APIは未実装です。

## 検証状況

本リリースでは、既存のUnit / Integration / Securityテスト、WebのVitest、TypeScript／Vite build、Compose設定検証を実行します。実Provider・実Docker・実機で未実施の項目は、検証結果に明記します。
