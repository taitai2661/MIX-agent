# Desktop 版 macOS Spotlight 索引連携（計画）

> 状態: 実装中（Phase 0〜2 実装済み。Phase 3 の実機確認が残り）
> 対象: `apps/desktop`（**macOS のみ**）
> 最終更新: 2026-11-05
> 関連: [`architecture.md`](architecture.md) / [`../verification.md`](../verification.md)

---

## 0. 結論

macOS の **Core Spotlight**（`CSSearchableIndex`）に MIX-agent の
**会話・プロジェクト・ナレッジ**を登録し、システムの Spotlight 検索結果から
該当画面を直接開けるようにする。

- 索引はアプリの既定インデックス（`CSSearchableIndex.default()`）に
  `conversations` / `projects` / `knowledge` の 3 ドメインで書き込む。
- 検索結果のクリックは `CSSearchableItemActionType` の `NSUserActivity` で
  返る。Tauri/tao の既定デリゲートは browsing-web 以外を `false` で落とすため、
  `application:continueUserActivity:restorationHandler:` を差し替えて受ける。
- 設定で **メタデータ索引 / 本文索引 / 無効化** を選べる。既定は
  **メタデータのみ OFF、本文 OFF**（明示的に有効化したユーザーだけが対象）。
- `tauri dev` の非バンドル実行では Core Spotlight に登録できない。
  検証は `tauri build` で生成した `.app` を `/Applications` に置いて行う。
- 最初のゲートは **Phase 0 の実機スパイク**。デリゲート差し替えが
  将来の Tauri/tao 更新で壊れた場合は、機能を無効として扱い、計画をやり直す。

想定工数は Phase 0〜3 で **6〜9 日**。最初の実装 PR は Phase 0 + Phase 1
（会話タイトルの索引とクリック遷移）に絞る。

---

## 0.1 実装状況（2026-11-06）

- Phase 0〜2 を実装した。`app/src/spotlight/` にキュー・payload・Core Spotlight
  書き込み・クリック受信を追加し、`spotlight_items` / `spotlight_meta`
  （schema version 5）と設定タブを用意した。
- **Rust の Core Spotlight 書き込みは実機で確認済み**。`cargo build` した
  `spotlight_smoke` example を ad-hoc 署名した最小 `.app` に包み、
  `INDEX_OK` / `DELETE_OK` を確認した（macOS 27.2 / Apple Silicon）。
- `cargo test -p mix-agent-desktop`（94 unit + integration）、`cargo test -p mixd-storage`、
  `cargo clippy -p mix-agent-desktop --all-targets -- -D warnings`、
  `cargo fmt --all -- --check`、UI の i18n / useShellEvents テストが成功。
- **残り**: インストール版 `.app` での Spotlight UI 表示、検索結果クリックから
  NSUserActivity を受けて画面遷移する一連の確認。macOS 12 / 14 / Intel 実機、
  大量履歴での性能確認。`CSSearchableIndexDelegate` による OS 再索引要求は
  未実装（手動再構築で代替）。

---

## 1. 目的と非目標

### 目的

1. Spotlight で会話タイトル・プロジェクト名・ナレッジ名を検索できる。
2. 検索結果を選ぶと MIX-agent が前面に出て、該当スレッド / プロジェクト /
   ナレッジ画面が開く。アプリが終了していても起動して遷移する。
3. 会話の改名・削除・追加、プロジェクトやナレッジの変更が索引へ追従する。
4. 設定を OFF にしたら、アプリが登録した索引をすべて削除する。
5. 索引処理は起動・チャット・Run のクリティカルパスを塞がない。

### 非目標（この計画では扱わない）

- Windows Search / Linux（Tracker, recoll 等）への索引登録。
- ⌘K コマンドパレットの置き換え。パレットはアプリ内検索として残す。
- ワークスペース内のファイルの再索引。macOS の Spotlight が既に
  ファイルシステムを索引しているため、アプリから重複登録しない。
- Memory / Skill / Agent / スケジュールの索引。個人情報や設定値が
  システム検索に露出するため、Phase 1 の対象外とする。
- Spotlight の検索ランキングを Apple 製アプリ並みに制御すること。
- 会話単位の「Spotlight から除外」トグル（将来の拡張候補）。

---

## 2. 現状

- アプリ内検索は既にある。⌘K の `CommandPalette` が
  `search_conversations`（タイトル）と `search_messages`（FTS5）を呼ぶ。
  ただしこれは WebView 内だけで、macOS の Spotlight には何も出ない。
- DB は `conversations` / `projects` / `knowledge` / `messages` を持つ。
  いずれも `data` JSON に表示用の値を持つ（`storage/src/records.rs`）。
  - `ConversationData`: `title` / `mode` / `projectId` / `updatedAt`
  - `ProjectData`: `name` / `rootPath` / `instructions`
  - `KnowledgeData`: `docId` / `filename` / `chunkIndex` / `text`
- `mix-agent-desktop://` の deep link は **MCP OAuth のコールバック専用**として
  `app/src/deep_link.rs` が処理している。汎用の画面遷移には使っていない。
- ウィンドウを閉じてもトレイに残る（`on_window_event` で `hide`）。
  単一インスタンスプラグインで二重起動を防いでいる。
- アプリは ad-hoc 署名（`signingIdentity: "-"`）で、macOS 12.0 以上を対象とする。
  App Sandbox は有効化していない。
- Tauri が使う tao の `TaoAppDelegateParent` には
  `application:continueUserActivity:restorationHandler:` が既にあるが、
  `NSUserActivityTypeBrowsingWeb` 以外は `false` を返す
  （`tao-0.37.1/src/platform_impl/macos/app_delegate.rs`）。
  `application:willContinueUserActivityWithType:` も browsing-web 以外は
  `false` なので、そのままでは Core Spotlight のクリックを受け取れない。
- 現時点で Spotlight / CoreSpotlight / `NSUserActivity` を扱うコードはない。

---

## 3. 技術検証（2026-11-05 実施）

macOS 27.2 / Apple Silicon で、ad-hoc 署名・非サンドボックスの最小 `.app` を
作り、Core Spotlight の疎通を確認した。

| 確認 | 結果 |
| :--- | :--- |
| `CSSearchableItem` の登録（`indexSearchableItems`） | **成功**（エラーなし） |
| 同一 bundle id からの `CSSearchQuery` | **1 件ヒット** |
| 別 bundle id からの `CSSearchQuery` | 0 件（索引はアプリ単位で分離される） |
| ad-hoc 署名（Team ID なし） | 登録・検索とも成功 |
| App Sandbox なし | 登録・検索とも成功 |
| バンドル外の実行ファイル | bundle id を解決できず登録不可（`tauri dev` は不可） |

判明した制約:

1. **バンドルが必須**。`target/debug/mix-agent-desktop` を直接実行する
   `tauri dev` では `CFBundleIdentifier` がなく、Core Spotlight に登録できない。
   機能の動作確認は `tauri build --debug` で作った `.app`、またはリリース版で行う。
2. **索引はアプリの bundle id に閉じる**。別プロセスから OS 索引を一覧・検証
   できないため、設定画面の「索引済み件数」はアプリ側の管理テーブルを正とする。
3. **content type は具体的な型にする**。`UTType.text` は検索結果に表示される。
   汎用の `public.content` では表示されないという報告があるため使わない。
4. **クリックは `NSUserActivity` で返る**。
   `activityType == CSSearchableItemActionType`、
   `userInfo[CSSearchableItemActivityIdentifier]` に `uniqueIdentifier` が入る。
5. **tao のデリゲートを差し替える必要がある**。Tauri の `RunEvent::Opened` は
   URL 専用で、Core Spotlight の activity は流れてこない。
6. **`objc2-core-spotlight` 0.3.x で Rust から直接呼べる**。Swift ブリッジを
   ビルドに持ち込まずに済む。`CSSearchableIndex::isIndexingAvailable()` で
   実行時判定できる。macOS 12 でも `CSSearchableIndex` 自体は利用可能。

> この検証は「索引登録と同一 bundle id からの検索」まで。Spotlight UI 上での
> 表示と、検索結果クリック時のデリゲート呼び出しは Phase 0 で確認する。

---

## 4. 設計

### 4.1 全体像

```
SQLite (conversations / projects / knowledge)
   │  create/update/delete と Run 終了時に schedule
   ▼
SpotlightIndexer (app/src/spotlight/sync.rs)
   │  バッチ・デバウンス・再試行
   ▼
CSSearchableIndex.defaultSearchableIndex()
   │  domain: conversations | projects | knowledge
   ▼
macOS Spotlight
   │  ユーザーが結果を選択
   ▼
NSApplicationDelegate (差し替え)
   │  CSSearchableItemActivityIdentifier を解釈
   ▼
"spotlight://open" イベント + pending スロット
   ▼
useShellEvents → 会話 / プロジェクト / ナレッジ画面へ遷移
```

### 4.2 何を索引するか

| 種別 | uniqueIdentifier | domain | title | description / keywords | 本文（opt-in） |
| :--- | :--- | :--- | :--- | :--- | :--- |
| 会話 | `conversation:<id>` | `conversations` | スレッドタイトル（空なら最初のユーザー発言の要約） | 直近の assistant メッセージ先頭 200 文字、モード（チャット/思考/エージェント）、プロジェクト名 | user / assistant の本文のみ。ツール出力・添付・reasoning は含めない |
| プロジェクト | `project:<id>` | `projects` | プロジェクト名 | ルートフォルダ名、`rootPath`（本文 opt-in 時のみ）、指示の先頭 200 文字 | `instructions` と `sources` の先頭（上限あり） |
| ナレッジ | `knowledge:<docId>` | `knowledge` | ファイル名 | 先頭チャンクの先頭 200 文字、チャンク数 | 全チャンクを連結（上限あり） |

- `contentType` はすべて `public.text`（`UTType.text`）にする。
- `contentCreationDate` / `contentModificationDate` は会話の `created_at` /
  `updatedAt`、プロジェクトの `createdAt` / `updatedAt`、ナレッジの
  `created_at` を入れる。Spotlight の並び替えと日付フィルタに効く。
- `contentURL` は Phase 1 では設定しない。macOS がクリック時に URL を
  直接開こうとする可能性があり、activity 経由の遷移と競合するため。
  必要になったら Phase 0 の結果を見て、`mix-agent-desktop://` を入れるか
  独自 attribute にするかを決める。遷移は activity の identifier だけで行う。
- 本文索引の上限は **会話 8,000 文字 / ナレッジ 64,000 文字 / プロジェクト
  20,000 文字**。超えた分は切り捨て、`…` を付ける。
- 本文索引では `mixd_core::memory::is_sensitive` 相当の判定を通し、
  認証情報らしき行（API キー、トークン、パスワード）を除外する。
  タイトルと description は既に UI に出ている情報なので対象外。

### 4.3 クリック時の遷移

`app/src/spotlight/route.rs` で NSApplicationDelegate の 2 メソッドを差し替える。

- `application:willContinueUserActivityWithType:`
  `CSSearchableItemActionType` なら `true` を返す。
- `application:continueUserActivity:restorationHandler:`
  activity が Core Spotlight なら `userInfo` から identifier を取り出し、
  既存のデリゲート実装（browsing-web）へはフォールバックする。
  元の IMP は `method_setImplementation` の戻り値として保持し、関数ポインタで
  直接呼ぶ（`msg_send` で自分自身を呼ぶと再帰する）。
- 差し替えは Tauri の `setup` で行う。tao の
  `applicationDidFinishLaunching` から setup が呼ばれ、その後に macOS が
  activity を配送するため、コールドスタートでも間に合う。
- セレクタやクラスが想定と違う場合（将来の tao 更新）は **機能を無効**にし、
  `tracing::warn!` を 1 回だけ出す。無理に swizzle しない。
- 受けた identifier は次のように解決する。
  - `conversation:<id>` → DB から `mode` を引き、`SpotlightTarget::Conversation`
  - `project:<id>` → `SpotlightTarget::Project`
  - `knowledge:<docId>` → `SpotlightTarget::Knowledge`
  - 対象が既に削除されていれば索引から削除し、何もしない（stale item の掃除）。
- デリゲートコールバックは同期 API なので、identifier を pending に積んで
  すぐ `true` を返し、DB 解決と `spotlight://open` の emit は
  `tauri::async_runtime::spawn` したタスクで行う。DB 待ちで Spotlight の
  起動をブロックしない。
- ウィンドウを `show` / `unminimize` / `set_focus` してから
  `spotlight://open` を emit する。
- UI がまだ listen していないコールドスタートに備え、最後の target を
  `Mutex<Option<SpotlightTarget>>` に保持する。UI はマウント時に
  `spotlight_take_pending` を呼んで drain する（イベントと二重にならないよう
  take は 1 回だけ返す）。
- 既存の MCP OAuth 用 `mix-agent-desktop://` の処理は変更しない。

UI 側は `useShellEvents` に `spotlight://open` の listener を追加する。

- 会話: `surfaceForMode(mode)` → `setConversationId(id)` → `setView("chat")`
- プロジェクト: `setActiveProjectId(id)` → `setView("projects")`
- ナレッジ: `openSettings("knowledge")`

### 4.4 同期とライフサイクル

SQLite に管理テーブルを追加する（schema version 5）。

```sql
CREATE TABLE IF NOT EXISTS spotlight_items (
    item_id      TEXT PRIMARY KEY,   -- conversation:<id> など
    domain       TEXT NOT NULL,      -- conversations | projects | knowledge
    source_id    TEXT NOT NULL,      -- 元の会話 / プロジェクト / doc id
    content_hash TEXT NOT NULL,      -- 索引ペイロードの SHA-256
    state        TEXT NOT NULL DEFAULT 'pending', -- pending | indexed | deleted
    indexed_at   TEXT,
    last_error   TEXT
);
CREATE INDEX IF NOT EXISTS spotlight_items_state_idx
    ON spotlight_items(state, domain);

-- schema_version / last_full_sync など、ユーザー設定ではない内部状態。
CREATE TABLE IF NOT EXISTS spotlight_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
```

- **起動時**: 設定が ON なら、起動 5 秒後に `reconcile()` をバックグラウンド実行。
   - 会話 / プロジェクト / ナレッジを列挙し、`content_hash` を計算。
   - 既存行と一致すればスキップ、違えば `pending`、DB から消えた行は `deleted`。
   - `pending` を 50 件ずつ索引。`deleted` は identifier 指定で削除。
   - これがクラッシュ後の復旧経路を兼ねる。
- **実行時**: 変更系コマンドと Run 終了時に `schedule()` を呼ぶ。
   - 2 秒デバウンスでまとめ、`Notify` でワーカーを起こす。
   - 同じ item の連続更新は最後の 1 回だけ索引する。
- **再試行**: 失敗したら `last_error` を保存し、5 秒 → 30 秒 → 5 分の
  バックオフで再試行する。成功するとバックオフをリセットする。エラーは
  設定画面に表示し、失敗した行は pending のまま残す。
- **OFF にしたとき**: キューを止め、`deleteAllSearchableItems` を呼び、
  `spotlight_items` を空にする。
- **本文索引の ON/OFF 変更**: 全 item を `pending` に戻して再構築する。
- **手動再構築**: 3 ドメインを `deleteSearchableItemsWithDomainIdentifiers`
  で消してから全件を再登録する。
- **スキーマ更新**: DB の `PRAGMA user_version` と各 item の
  `content_hash` で再構築要否を判断する。専用の `schema_version` は持たない
  （テーブル作成は `0005_spotlight.sql` の `IF NOT EXISTS` で自己修復する）。

`schedule()` を呼ぶ箇所:

| 変更 | 呼び出し元 |
| :--- | :--- |
| 会話の作成 / 改名 / 削除 / fork / プロジェクト紐付け | `app/src/commands/conversations.rs` |
| メッセージ追加・編集・切り詰め（本文索引時） | `app/src/commands/conversations.rs` と Run 終了時 |
| Run の終了（タイトル自動生成・最終メッセージ反映） | `app/src/run_events.rs` の terminal status（`run_id` から `runs.conversation_id` を引く） |
| プロジェクトの作成 / 更新 / 削除 | `app/src/commands/projects.rs` |
| ナレッジの取り込み / ファイル取り込み / 削除 | `app/src/commands/knowledge.rs` |
| バックアップ復元後の次回起動 | `spotlight::start` の起動時 reconcile（復元は再起動前提） |
| 設定変更（有効化 / 本文 / OFF） | `app/src/commands/settings.rs` |

### 4.5 モジュール構成

```
apps/desktop/app/src/
├── spotlight/
│   ├── mod.rs      # 公開 API（schedule / reconcile / clear / status / install）
│   ├── index.rs    # CoreSpotlight の薄いラッパー（macOS のみ）
│   ├── payload.rs  # DB → 索引アイテムの純粋関数（テスト対象）
│   ├── sync.rs     # キュー、デバウンス、再試行、reconcile
│   ├── route.rs    # NSApplicationDelegate 差し替えと target 解決
│   └── noop.rs     # 非 macOS のスタブ
└── commands/
    └── spotlight.rs # Tauri コマンド（status / rebuild / clear / take_pending）
```

- `spotlight/mod.rs` は `#[cfg(target_os = "macos")]` で実装を切り替え、
  Windows / Linux では全 API が no-op になる。UI とコマンドは共通。
- `Cargo.toml` の macOS 限定依存:

```toml
[target.'cfg(target_os = "macos")'.dependencies]
objc2 = "0.6"
objc2-foundation = { version = "0.3", features = [
  "NSBundle", "NSUserActivity", "NSArray", "NSString", "NSError", "NSDate", "NSURL",
] }
objc2-core-spotlight = { version = "0.3", features = [
  "objc2-uniform-type-identifiers", # UTType.text
] }
objc2-uniform-type-identifiers = "0.3"
block2 = "0.6"
sha2 = "0.10" # content_hash（browser / mcp クレートと同じ版）
```

- `objc2-core-spotlight` の既定 feature で `CSSearchableIndex` /
  `CSSearchableItem` / `CSSearchableItemAttributeSet` / `block2` は有効。
  `UTType` を使うため `objc2-uniform-type-identifiers` だけ追加する。
- 既存の `tao` が使う objc2 とバージョンを揃える（Cargo.lock で確認）。
- `CSSearchableIndex` はスレッドセーフなので、tokio のワーカーから呼んでよい。
  `NSApplicationDelegate` の差し替えだけは main thread の `setup` で行う。

### 4.6 設定とプライバシー

`SettingsView` に次を追加する。

| キー | 既定 | 意味 |
| :--- | :--- | :--- |
| `spotlight_enabled` | `false` | macOS Spotlight への索引を有効にする |
| `spotlight_index_body` | `false` | 会話・ナレッジの本文も索引に含める |

- 既定を OFF にする理由: 会話タイトルにはプロジェクト名や顧客名が入り得る。
  システム全体の検索結果に出るため、明示的な opt-in にする。
- 設定画面は **macOS のみ**「Spotlight」タブを出す。非 macOS ではタブ自体を
  出さない（`os.platform()` で判定）。
- タブの内容:
  - 有効化トグル（説明: タイトルとプレビューが Spotlight に出る）
  - 本文も索引するトグル（警告: 会話本文が検索対象になる）
  - 状態: 索引済み件数 / 未処理件数 / 最終同期 / 最終エラー
  - 「今すぐ再構築」「索引から削除」ボタン
  - 注記: 索引はこの Mac のローカルにのみ保存され、アプリを削除しても
    残る場合があること
- 無効化時は `deleteAllSearchableItems` を必ず呼ぶ。失敗した場合は
  設定画面にエラーを残し、次回起動時に再試行する。
- 本文索引の対象外: 添付ファイル、画像、ツール出力、reasoning、
  Memory、システムプロンプト、API キー。
- 既存の `mixd_core::memory::is_sensitive` を使った行単位の除外を入れ、
  認証情報らしき文字列を含む行は索引しない。

### 4.7 Tauri コマンドと UI

追加コマンド（`app/src/commands/spotlight.rs`）:

| コマンド | 戻り値 | 用途 |
| :--- | :--- | :--- |
| `spotlight_status` | `SpotlightStatus` | 設定画面の表示 |
| `spotlight_rebuild` | `()` | 手動再構築 |
| `spotlight_clear` | `()` | 索引から削除 |
| `spotlight_take_pending` | `Option<SpotlightTarget>` | コールドスタートの遷移先 |

`SpotlightStatus` は `supported` / `reason` / `enabled` / `body_indexed` /
`indexed_count` / `pending_count` / `last_sync` / `last_error` を持つ。
`supported` は「macOS かつ bundle id が解決できるかつ
`CSSearchableIndex::isIndexingAvailable()` が true」で判定する。

UI 側の変更:

- `ui/src/features/settings/SpotlightSettings.tsx` を追加。
- `SettingsPane.tsx` のタブ一覧と本文に追加（macOS のみ）。
- `lib/ui.ts` の `SettingsTab` に `"spotlight"` を追加。
- `CommandPalette.tsx` の `SETTINGS_TARGETS` に「Spotlight」を追加。
- `lib/queries.ts` に `spotlightStatus` クエリ、`lib/api.ts` にコマンドラッパー。
- `useShellEvents.ts` に `spotlight://open` listener と起動時の pending drain。
- `lib/i18n/en.ts` に英語文言を追加（日本語はソース内の `t("…")`）。

---

## 5. 実装フェーズ

### Phase 0 — 足場と実機スパイク（0.5〜1 日）

- [ ] macOS 限定依存を追加し、`CSSearchableIndex::isIndexingAvailable()` と
      bundle id 判定を実装。
- [ ] `tauri build --debug` の `.app` で、開発用の固定アイテムを 1 件索引する
      スパイクを入れる（リリースビルドでは無効）。
- [ ] Spotlight UI にそのアイテムが出ることを確認。
- [ ] アイテムをクリックし、`application:continueUserActivity:` が呼ばれること、
      および差し替え後に identifier を取得できることを確認。
- [ ] 差し替えに失敗した場合の無効化パスを実装。
- [ ] 結果と制約を `docs/verification.md` に追記。

**ゲート**: ここでクリックを受け取れない場合、Phase 1 に進まず、
代替案（NSUserActivity のみ / App Intents / 別プロセス）を再検討する。

### Phase 1 — 会話の索引と遷移（2〜3 日）

- [ ] schema version 5 と `spotlight_items` / `spotlight_meta`。
- [ ] `payload.rs` で会話アイテムを構築（タイトル / プレビュー / モード /
      日付）。純粋関数としてテスト。
- [ ] `sync.rs` のキュー、デバウンス、バッチ、再試行。
- [ ] 起動時 reconcile。
- [ ] 会話の作成 / 改名 / 削除 / Run 終了の schedule。
- [ ] 設定（`spotlight_enabled`）と Spotlight タブ。
- [ ] クリック→ `spotlight://open` → 会話を開く。コールドスタートの pending。
- [ ] `spotlight_status` / `rebuild` / `clear`。

**受け入れ**: インストール版で有効化→30 秒以内にタイトル検索でヒット、
クリックで正しいスレッドが開く。改名・削除が 10 秒以内に反映。OFF で消える。

### Phase 2 — プロジェクト・ナレッジ・本文（2〜3 日）

- [ ] プロジェクトとナレッジの payload。
- [ ] プロジェクトの作成 / 更新 / 削除、ナレッジの取り込み / 削除の schedule。
- [ ] `spotlight_index_body`（既定 OFF）と再構築。
- [ ] 本文の上限・機密行の除外。
- [x] バックアップ復元後の次回起動 reconcile（復元は再起動前提）。
- [ ] ステータス表示の詳細化（件数 / 最終同期 / エラー）。
- [ ] 可能なら `CSSearchableIndexDelegate` を実装し、OS からの
      再索引要求に応答する（難しければ手動再構築で代替し、制約を明記）。

**受け入れ**: 会話・プロジェクト・ナレッジの 3 種が検索でき、本文 ON で
本文の語句でもヒットする。機密らしき行が索引に残らない。

### Phase 3 — 仕上げと検証（1〜2 日）

- [ ] `cargo test` / UI テスト / `cargo clippy` を通す。
- [ ] macOS 12 / 14 / 26 以降、Intel / Apple Silicon で手動確認。
- [ ] `docs/desktop/architecture.md` に現行仕様を追記。
- [ ] `docs/verification.md` に検証記録を追記。
- [ ] リリースノートと README の機能表を更新。
- [ ] 無効化・アンインストール時の索引削除手順を書く。

---

## 6. 受け入れ条件

1. インストール版の macOS で設定を有効化すると、会話タイトルが Spotlight の
   検索結果に出る。
2. 結果をクリックすると、アプリが前面に出て該当スレッドが開く。
   アプリ終了中でも起動して遷移する。
3. スレッドの改名・削除、プロジェクトの変更、ナレッジの追加・削除が
   索引に追従する。
4. 設定を OFF にすると、アプリが登録した索引がすべて消える。
5. 本文索引 OFF では、会話本文・ツール出力・添付が索引に含まれない。
6. 起動と Run の待ち時間に体感できる悪化がない（索引はバックグラウンド）。
7. Windows / Linux のビルドとテストに影響がない。
8. `tauri dev` では「この環境では利用できません」と表示され、壊れた
   索引やエラーを繰り返さない。

---

## 7. テスト計画

### 自動テスト

- `payload.rs` の単体テスト:
  - identifier の生成 / 解析（prefix ごとの往復）
  - タイトルが空のときのフォールバック
  - 本文の文字数上限と `…` の付与
  - 機密行の除外
  - content hash の安定性（同じ入力で同じ値）
- `sync.rs` のテスト:
  - `IndexSink` トレイトを差し替え、reconcile が「追加 / 更新 / 削除」を
    正しく分類する。
  - デバウンスで同じ item が 1 回になる。
  - 失敗時のバックオフと 3 回停止。
- `route.rs` のテスト:
  - identifier → `SpotlightTarget` の変換。
  - 存在しない対象のときの削除要求。
- UI:
  - `useShellEvents` の `spotlight://open` で surface / view / settings tab が
    正しく変わる。
  - Spotlight タブが macOS 以外で描画されない。
- CI では CoreSpotlight の実呼び出しを `#[ignore]` にし、通常の
  `cargo test` を Linux / Windows でも通す。

### 手動テスト（リリース前）

1. `pnpm desktop:build` で `.app` を作り `/Applications` に置く。
2. 設定 → Spotlight → 有効化。30 秒待つ。
3. Spotlight（⌘Space）でスレッドタイトルの一部を検索し、結果が出る。
4. 結果をクリック → 正しいスレッドが開く。
5. アプリを終了して同じ操作 → 起動して開く。
6. スレッドを改名 → 10 秒以内に結果のタイトルが変わる。
7. スレッドを削除 → 結果が消える。
8. 本文索引 ON → 本文の語句でヒット。OFF → ヒットしない。
9. 設定を OFF → Spotlight から結果が消える。
10. プロジェクト / ナレッジも同様に確認。
11. `log stream --predicate 'process == "corespotlightd"'` でエラーが
    連続していないことを確認。

---

## 8. リスクと対策

| リスク | 影響 | 対策 |
| :--- | :--- | :--- |
| `tauri dev` では動かない | 開発中に確認できない | bundle id を検出して「インストール版でのみ利用可能」と表示。`tauri build --debug` を検証手順に明記 |
| tao のデリゲート差し替えが Tauri 更新で壊れる | クリックが効かない | セレクタ存在チェック、失敗時は機能無効、Phase 0 のゲート、Tauri 更新時の確認項目に追加 |
| macOS のバージョン / 署名差で索引できない | 機能が出ない | macOS 12 / 14 / 26 と Intel / Apple Silicon で確認。`isIndexingAvailable` と bundle id で事前判定 |
| Spotlight 上の表示順が低い | 見つけにくい | まずは検索可能であることを優先。必要なら App Intents / NSUserActivity を Phase 4 で検討 |
| 機密情報がシステム検索に出る | プライバシー事故 | 既定 OFF、本文は別 opt-in、機密行の除外、OFF 時の全削除、設定画面の説明 |
| 大量の履歴で索引が重い | 起動・CPU | 50 件バッチ、起動 5 秒後、バックグラウンド、本文の上限、デバウンス |
| 索引の残骸がアンインストール後も残る | 情報残存 | 「索引から削除」を用意し、README / 設定に注記。アンインストーラでの削除は将来対応 |
| 別ユーザー / 複数インスタンス | 二重索引 | 単一インスタンスは既に有効。索引は macOS のユーザー単位 |
| バックアップ復元で ID が変わる | stale item | 復元後に全件 reconcile + 旧ドメイン削除 |

---

## 9. 参照

- 現行仕様: [`architecture.md`](architecture.md)
- 検証記録: [`../verification.md`](../verification.md)
- Apple: [CSSearchableIndex](https://developer.apple.com/documentation/corespotlight/cssearchableindex) /
  [CSSearchableItemAttributeSet](https://developer.apple.com/documentation/corespotlight/cssearchableitemattributeset) /
  [Adding your app's content to Spotlight](https://developer.apple.com/documentation/corespotlight/adding-your-app-s-content-to-spotlight)
- Rust bindings: [`objc2-core-spotlight`](https://docs.rs/objc2-core-spotlight/)
- tao のデリゲート実装: `tao-0.37.1/src/platform_impl/macos/app_delegate.rs`
- 実装上の既存コード: `app/src/deep_link.rs` / `app/src/tray.rs` /
  `app/src/commands/settings.rs` / `storage/src/records.rs`
