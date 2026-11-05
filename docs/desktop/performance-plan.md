# MIX-agent Desktop — 軽量化計画（2026-11-04）

## 目的

デスクトップ版（`apps/desktop/`）の **起動・描画・実行時処理・常駐リソース** を軽くする。
対象は Tauri シェル（Rust）と React UI の両方。Docker 版（`apps/server/`, `apps/web/`）は対象外。

この計画は「すでに入っている最適化をやり直さない」ことを前提にする。現状のコードには
以下が実装済みで、**触らない（退行させない）**。

| 実装済み | 場所 | 効果 |
| :--- | :--- | :--- |
| Transcript の仮想スクロール（40件超） | `ui/src/features/chat/Transcript.tsx` | 長い履歴でも DOM を抑える |
| スクロールの rAF バッチ | 同上 | ストリーミング中の同期レイアウトを回避 |
| Run イベントの 60ms バッチ | `ui/src/lib/runStore.ts` | トークンごとの再描画を最大16回/秒に |
| トークンイベントの DB 書き込みバッチ（250ms / 64件） | `core/src/runs/engine/event_bus.rs` | SQLite INSERT をボトルネックにしない |
| 会話・メッセージのカーソルページング | `app/src/commands/conversations.rs` | 全件ロードを回避 |
| FTS5（メッセージ・知識） | `storage/src/migrations/0002_search.sql` | 全件スキャンを索引に |
| ペインのコード分割・chunk 分割 | `ui/src/App.tsx`, `ui/vite.config.ts` | 初期ロードを軽量化 |
| Chromium のアイドル終了（現在5分） | `browser/src/operator.rs` | 数百MBの回収 |
| dev プロファイルの debuginfo 縮小 | `Cargo.toml` | `target/` の肥大化抑制 |
| 埋め込みの起動5秒後バックフィル | `app/src/lib.rs` | 起動パスから分離 |
| 記憶抽出の 30 秒ティック / 最大4並列 | `app/src/scheduler.rs`, `core/src/memory_jobs.rs` | 応答パスから分離 |

> 方針: **まず測る → 効く順に直す → 数値で退行を防ぐ**。時間（レイテンシ）は CI の
> ゲートにしない（マシン依存）。品質（検索の recall / MRR）は既存の共有 fixture で守る。

## 実装状況（2026-11-04）

| Phase | 状態 | 実装 |
| :--- | :--- | :--- |
| 0 計測 | 完了 | `target: "startup"` の起動スパン、`shell-ready` / `transcript-ready` の mark |
| 1 起動 | 完了 | バックアップはマイグレーション時のみ + 日次バックグラウンド、`quick_check` は7日ごと、MCP 接続の非同期化と 3 秒バリア、設定1回読み、worktree/Chromium 清掃の背景化 |
| 2 UI | 完了 | Sidebar は `busyConversationIds`、ChatPane は run/status/approval のみ購読、timeline は `LiveBlock` に閉じる、Markdown 120ms、ポーリング可視連動、`git status -unormal`（Changes は `-uall`）、Onboarding/Palette/Shortcut の lazy 化、一覧の `content-visibility` |
| 3 検索/実行 | 完了 | `memory_search` FTS（候補 2000、短語/CJK は全件フォールバック）、Run イベントの IPC 50ms バッチ、埋め込みバックフィルのアイドル待ち、記憶抽出の前景優先（並列1） |
| 4 常駐 | 完了 | Chromium 5分、LSP 10分アイドル + 5分 sweep、ログ14日、`run_events` 180日、`PRAGMA optimize` / passive checkpoint、`opt-level = 3` |
| 5 退行防止 | 完了 | `scripts/check-desktop-bundle.mjs` を CI に追加（初期 500kB / 合計 1.5MB） |

**実装時の判断**

- リプレイのバッチ化は、実際に使われている **ライブの `run_event` IPC** を
  50ms/64 件でまとめる形にした（`run_event_batch`）。`replay_run_events` も
  500 件ずつ同じイベント名で送る。
- Sidebar の仮想化は、ページングが 50 件単位で `content-visibility` でも十分だったため
  後者を採用した。数百件を常時表示するようになったら `@tanstack/react-virtual` へ移す。
- 数値の基準（起動時間・p95）はこの環境では未取得。Phase 0 のログと
  `cargo run --release -p mixd-core --bin retrieval-bench -- --size 10000` で取得し、
  次の変更時に比較する。fixture スイートだけは 2026-11-04 に debug で再確認し、
  `recall@5 = 1.0000` / `recall@8 = 1.0000` / `MRR = 1.0000` / `noise = 0.4167`
  （FTS 候補絞り込み後も不変）。

---

## 現状の負荷源（コードから確認できたもの）

| # | 負荷源 | 場所 | 影響 |
| :--- | :--- | :--- | :--- |
| 1 | **起動のたびに DB 全体を `VACUUM INTO`** | `storage/src/db.rs:359` `backup_before_open` | DB サイズに比例。長期利用者ほど起動が遅い |
| 2 | **起動のたびに `PRAGMA quick_check`** | `storage/src/db.rs:74` | DB サイズに比例 |
| 3 | **`build_app_state` を `block_on` で完了まで待つ** | `app/src/lib.rs:1669` 付近 | MCP 接続・Keychain 読み出しが初回描画を待たせる |
| 4 | **MCP の `connect_all` が起動パス上** | `app/src/lib.rs:1059` | 外部サーバー次第で数百ms〜数秒 |
| 5 | **設定を起動中に3回読む** | `app/src/lib.rs:1051,1082,1125` | 小さめだが無駄 |
| 6 | **Sidebar が `byConversation` 全体を購読** | `ui/src/features/shell/Sidebar.tsx:171` | ストリーミング中、毎フラッシュで Sidebar 全体が再描画 |
| 7 | **ChatPane が live timeline を購読し Transcript まで再描画** | `ui/src/features/chat/ChatPane.tsx:121` | 60ms ごとに大きいツリーを再調整 |
| 8 | **ストリーミング中に Markdown を毎回再パース** | `ui/src/features/chat/LiveBlock.tsx:250` | 長い回答ほど終盤が重い |
| 9 | **`git status -uall` を 15 秒ごと** | `app/src/commands/workspace.rs:209` | 未追跡ファイルが多いと毎回全列挙。さらに `rev-parse` も毎回 |
| 10 | **記憶検索が毎 Run 全行走査 + JSON パース** | `core/src/retrieval.rs:702` `score_rows` | 件数に比例。1万件で数十ms/回 |
| 11 | **Run リプレイが最大5万イベントを1件ずつ IPC** | `app/src/run_events.rs:38` | 長い Run を開くと詰まる |
| 12 | **`run_events` / `messages` の保持期間がない** | 全体 | DB が単調増加し、1・2・10 が年々悪化 |
| 13 | **release が `opt-level = "s"`（サイズ優先）** | `Cargo.toml:73` | スコアリング等の CPU 処理が遅め |
| 14 | **Git status / browser status / approvals のポーリングが可視状態を見ない** | `queries.ts`, `useBrowserStatus.ts` | 背面ウィンドウでも処理が続く |
| 15 | **Sidebar の会話一覧が非仮想化** | `Sidebar.tsx` | 数百スレッドでスクロール・検索が重い |

---

## Phase 0 — 計測基盤（1〜2日）

**これを最初にやる。** どの改善が効いたかを判断できないと、後の作業が勘になる。

1. **起動スパンの計測**
   - `Storage::open` の内訳（backup / quick_check / schema / data migrations）を
     `tracing::info!(target: "startup", ...)` で個別に出す。
   - `build_app_state` の主要ブロック（browser / MCP / providers / registry / grants）に
     `Instant::now()` のスパンを付ける。
   - `setup` 開始 → ウィンドウ表示 → 最初の `list_messages` 完了、を同じ `startup` ターゲットで記録。
   - 出力先は既存の `mix-agent.log`（`app/src/lib.rs:1601`）。UI の Diagnostics ペインに
     「前回起動の内訳」を出す（新しいテーブルは不要。最後のスパンをメモリに保持）。

2. **UI の計測**
   - `performance.mark("shell-ready")` / `"transcript-ready"` を `App.tsx` / `ChatPane.tsx` に追加。
   - ストリーミング中の再描画回数は React Profiler（開発時のみ）で確認。
   - `useRunEvents` のフラッシュ回数と適用イベント数を開発ビルドでカウント。

3. **検索ベンチの基準値**
   - 既存の `cargo run -p mixd-core --bin retrieval-bench -- --size 10000 --mode lexical`
     と `--mode hybrid` を **release** で記録（debug の数字は使わない）。
   - fixture スイート（`--suite fixture`）の recall / MRR を退行防止の基準にする。

4. **目標値（初期値。測ってから更新する）**

| 指標 | 目標 | 備考 |
| :--- | :--- | :--- |
| コールド起動 → シェル表示 | 1.5秒以内（M系 Mac） | DB 100MB 時 |
| 起動 → 最初のメッセージ一覧 | 2.0秒以内 | 上に同じ |
| 記憶検索 p95（1万件, release） | 20ms 以内 | lexical / hybrid とも |
| ストリーミング中の Sidebar 再描画 | 状態変化時のみ | 16回/秒をやめる |
| アイドル CPU | 1% 未満 | Chromium 停止時 |
| 常駐 RSS（Chromium 除く） | 250MB 未満 | 目安 |

---

## Phase 1 — 起動時間（P0・3〜5日）

### 1.1 起動時バックアップを毎回やめる

`storage/src/db.rs` の `backup_before_open` は毎回 `VACUUM INTO` する。これは
**DB サイズに比例する起動コスト**で、履歴が増えるほど悪化する。

- **マイグレーションが必要なときだけ** pre-open スナップショットを取る
  （`PRAGMA user_version < SCHEMA_VERSION` または `schema_is_current()` が false）。
- 通常起動時は、最後のスナップショットから **24時間以上経っていれば**、
  ウィンドウ表示後にバックグラウンドで取る。
- 最終バックアップ時刻は `<data_dir>/backup-state.json` などに保存する
  （DB の中に持つと、壊れた DB から読めない）。
- `prune_backups` は現状維持（保持数で切る）。

**期待**: DB 100MB で数百ms、1GB で数秒の削減。クラッシュ復旧の安全性は
「マイグレーション前は必ず取る」ことで維持する。

### 1.2 `quick_check` を毎回やめる

- 終了時に「クリーン終了」フラグを書き、次回起動でフラグがあるときはスキップ。
- フラグがない（前回が強制終了）とき、または前回チェックから7日経過で実行。
- `PRAGMA quick_check(1)` にして、最初のエラーで止める（全ページ走査を避ける）。

### 1.3 `build_app_state` を初回描画のクリティカルパスから外す

現状は `setup` で `block_on(build_app_state(...))` が終わるまでウィンドウが出ない
（`app/src/lib.rs:1741` 付近）。以下を分割する。

- **同期で残す**: `Storage::open`、`SecretStore`、最小限の設定読み出し、
  Tool Registry の構築（ネットワーク不要）。
- **非同期へ移す**: MCP の `hydrate_and_connect` / `collect_tool_specs`、
  `cleanup_stale_worktrees`、`prune_orphan_models`、`reconcile_stale_runs`、
  `seed_project_grants`。
- 非同期側は `tokio::sync::watch` か `OnceCell` で「準備完了」を公開し、
  **最初の Run だけ**がそれを待つ（UI は待たない）。MCP 未接続の間は
  MCP ツールが広告されないだけで、通常のチャットは開始できる。
- `connect_all` はサーバーごとに **3〜5秒のタイムアウト**を付け、可能なら並列化する。

### 1.4 設定読み出しを1回にまとめる

`read_settings` が browser / registry / sandbox で3回呼ばれている。
1回読んで `AppSettings` を使い回す。

**Phase 1 の受け入れ条件**
- コールド起動の `startup` ログで、`backup` が通常起動時に 0ms、
  `mcp_connect` が初回描画後に完了している。
- DB 100MB 相当の fixture で、起動時間が Phase 0 の基準から 30% 以上短縮。

---

## Phase 2 — UI 描画（P0/P1・3〜5日）

### 2.1 サブスクリプション境界を直す（最優先）

- **Sidebar**: `useRunEvents((s) => s.byConversation)` はフラッシュごとに新しい
  オブジェクトを返すため、ストリーミング中ずっと再描画される。
  `runStore` に **`runningConversationIds: string[]`**（または
  `Record<string, boolean>`）を持ち、`status` が変わったときだけ更新する。
  Sidebar はそれだけを購読する。
- **ChatPane → Transcript**: live timeline の購読を `LiveBlock` 側へ移す。
  `ChatPane` は `runId` と `isBusy` だけを購読し、`Transcript` には
  `liveRunId` と「live があるか」だけを渡す。スクロール追従に必要な
  文字数は `liveLength` のようなプリミティブで購読する。
- これにより、トークン到着ごとの再描画が「LiveBlock の中」に閉じる。

### 2.2 ストリーミング中の Markdown 再パースを抑える

`LiveBlock` は 60ms ごとに `Markdown` へ蓄積テキスト全体を渡す。`memo` は
テキストが変わるたびに無効化されるため、長い回答の終盤ほど1回のパースが重い。

- まず **ストリーミング中の更新間隔を 100〜150ms** に落とす（イベント自体は 60ms のまま、
  描画だけ間引く）。
- 効果が足りなければ、**完了したブロックを安定化**して部分ごとに memo 化する
  （コードブロック・段落単位）。最終トークンで全体を1回だけ整形する。
- 計測は React Profiler で「1フラッシュあたりの Markdown パース時間」を見る。

### 2.3 ポーリングを可視状態に連動

- `useGitStatus`: `refetchIntervalInBackground: false` を明示し、
  `document.visibilityState === "hidden"` と「FileTree / Changes ペインが非表示」の間は止める。
- `git status` の **`-uall` → `-unormal`**（未追跡ディレクトリを1行に畳む）。
  変更ペインが開いているときだけ `-uall` に切り替える。
- `rev-parse --abbrev-ref HEAD` は毎回呼ばず、branch をキャッシュして
  `HEAD` の変化（`.git/HEAD` の mtime）で無効化する。
- `useBrowserStatus` の 1 秒ポーリングは、ブラウザパネルが開いている間だけにする
  （既にイベントがあるのでポーリングはフォールバック）。
- `usePendingApprovals` の 4 秒は可視時のみ。

### 2.4 初期バンドルを削る

現状の初期 chunk は約 482kB（非圧縮）。以下を `lazy` 化する。

- `Onboarding`（初回のみ）
- `CommandPalette`（ショートカットで開くまで不要）
- `ShortcutHelp`（同上）
- `SurfaceSwitcher` / `WindowControls` はシェルに必要なので、依存する
  Radix コンポーネントだけを分割できないか検討。

`vite.config.ts` の `chunkSizeWarningLimit: 600` は警告のまま。**初期 chunk 500kB を
超えたら CI で落とす**（時間ではなくサイズは安定した指標）。

### 2.5 長い一覧

- Sidebar の会話一覧は非仮想化。ページは 50 件ずつだが、`load more` で数百件になる。
  `@tanstack/react-virtual` を導入するか、各行に `content-visibility: auto` を付ける。
- Memory / Knowledge の一覧も同じ方針（件数が多い画面から）。

**Phase 2 の受け入れ条件**
- ストリーミング中、Sidebar の再描画が「status 変化時のみ」になる（Profiler で確認）。
- 長い回答（1万文字）のストリーミングで、フレーム落ちが体感されない。
- 初期 chunk が 500kB 以下、`pnpm desktop:ui:build` が通る。

---

## Phase 3 — Run / 検索処理（P1・5〜8日）

### 3.1 記憶検索の全件スキャンを索引化

`core/src/retrieval.rs` の `score_rows` は `memories` を全件ストリームし、
各行の JSON をパースして `terms.score_parts` を計算する。1万件で数十ms/回、
10万件でさらに比例して増える。

- `knowledge_search` と同じ要領で **`memory_search` FTS5**（title / content）を追加し、
  候補を上位 N 件（例: 2000）に絞る。スコアラーは既存の `rank_score` のまま。
- 3文字未満・CJK の短いクエリは FTS が効かないので、**従来の全件スキャンへフォールバック**。
- `pinned` と直近更新行は候補に必ず含める（仕様 §2 の取りこぼし防止）。
- 共有 fixture（`--suite fixture`）で recall / MRR が不変であることを確認。
  合成コーパス（`--size 10000`）で p95 の改善を記録。

**注意**: 候補の絞り込みはランキングの意味を変えうる。必ず fixture とベンチを
セットで回し、`must_not_match` が増えないことを確認する。

### 3.2 Run リプレイのバッチ化

`app/src/run_events.rs` は最大 5万イベントを1件ずつ `app.emit` する。
長い Run を開くと IPC と React 側の処理が一度に走る。

- リプレイを **500件程度のチャンク**にまとめ、`run_event_batch` として送る。
- UI 側は `reduceRunEvents` で一括適用し、`setState` を1回にする。
- もしくは Rust 側で `RunTimeline` 相当まで畳んでから送る（契約の重複を避けるなら
  既存 reducer を共有できるチャンク方式が安全）。

### 3.3 バックグラウンド処理を前景と分離

- **埋め込みバックフィル**（起動5秒後）: 前景 Run が動いている間は開始を遅らせ、
  バッチ間に `yield` を入れる。設定で「アイドル時のみ」を既定にする。
- **記憶抽出**: 現在は最大4並列・1件90秒。前景 Run がアクティブな間は
  並列数を 1〜2 に落とす優先度付きセマフォを入れる。
- **スケジュール実行**: 既に別ループ。変更不要。

### 3.4 ツール出力の上限を徹底

- `terminal` / `grep` / `glob` / `browser` の結果は、DB 保存・IPC 送信の前に
  既存の上限で切る。巨大な base64 画像や HTML をそのまま IPC に載せない。
- `context.rs` の `TOOL_RESULT_CHAR_BUDGET`（60k）は維持。超えた古い結果は
  ポインタに置換する既存動作を、保存時にも適用する。

**Phase 3 の受け入れ条件**
- 1万件の記憶で release の p95 が 20ms 以下。
- fixture の recall@5 / recall@8 / MRR が 1.0000 のまま。
- 1万イベントの Run を開いても UI が 200ms 以内に操作可能。

---

## Phase 4 — 常駐リソースと長期運用（P1/P2・3〜5日）

### 4.1 Chromium

- アイドル終了を **15分 → 5分**（設定で変更可能に）。
- ブラウザパネルを閉じ、かつそのワークスペースを使う Run がなければ即終了する。
- スクリーキャストはパネル可視時のみ。非表示中はポーリングも止める
  （`useLiveFrame.ts` は既に `visibilitychange` を見ているので、パネル開閉も条件に加える）。

### 4.2 LSP / ターミナル

- LSP はワークスペースごとに1プロセス。**10分アイドルで停止**し、次のクエリで再起動。
- ターミナルは既存の PTY eviction を維持。アイドル時間を設定可能にする。

### 4.3 ログ

- `tracing_appender::rolling::daily` は日次で増え続ける。**保持日数（例: 14日）と
  合計サイズ上限**を追加し、起動時に古いファイルを削除する。
- Diagnostics ペインに「ログを削除」を置く。

### 4.4 DB の保持期間

`run_events` と `messages` は削除されず、DB が単調増加する。これは Phase 1 の
バックアップ・`quick_check`・検索スキャンを年々重くする根本原因。

- **完了済み Run の `run_events`**: 90日より古いものは、`status` / 最終テキスト /
  ツール結果の要約だけを残して圧縮する（または削除）。UI のリプレイが
  「古い Run は要約のみ」を表示できるようにする。
- **`messages`**: ユーザーが明示的に削除しない限り消さない（製品仕様）。
  代わりに `VACUUM` は自動で走らせず、設定の「データベースを最適化」で実行。
- 起動時ではなく、**アイドル時に `PRAGMA optimize`** を実行する。

### 4.5 release プロファイルの再検討

現在は `opt-level = "s"`（サイズ優先）。検索スコアリングや JSON 処理は CPU 依存。

- `opt-level = 3`（または `mixd-core` / `mixd-providers` だけ 3）で
  `retrieval-bench` と起動時間を比較する。
- バイナリサイズと更新ペイロードの増加（`opt-level="s"` 比）を測り、
  10% 以内なら速度優先を採用。
- `lto = "thin"` は維持。`codegen-units = 1` も維持。

---

## Phase 5 — 退行防止（運用）

1. **計測の定例化**: release ビルドで四半期ごとに
   `retrieval-bench`（fixture + 1万件）と起動スパンを記録し、この文書に追記する。
2. **CI は時間で落とさない**: 既存方針どおり、品質 fixture のみゲート。
   バンドルサイズは時間と違って安定するので、初期 chunk の上限だけゲートにする。
3. **PR チェックリスト**（エンジン / UI ストリーミング / DB スキーマを触る場合）:
   - 全件スキャンを追加していないか
   - 新しいポーリングを追加していないか（イベントで代替できないか）
   - 新しい `listen` / `setInterval` を追加したら解除もセットか
   - 長いリストを非仮想化で描いていないか
   - 起動パスにネットワーク I/O を足していないか

---

## 実行順と見積もり

| 順 | Phase | 内容 | 目安 | 期待効果 |
| :--- | :--- | :--- | :--- | :--- |
| 1 | 0 | 計測基盤 | 1〜2日 | 判断材料 |
| 2 | 1 | 起動（backup / quick_check / MCP分離） | 3〜5日 | 起動30〜50%短縮 |
| 3 | 2.1 | サブスクリプション境界 | 1〜2日 | ストリーミング中のCPU削減 |
| 4 | 2.3 | ポーリング・git status | 1日 | アイドル/背面の負荷削減 |
| 5 | 2.2, 2.4 | Markdown・バンドル | 2日 | 描画と初回ロード |
| 6 | 3.1 | 記憶検索の索引化 | 3〜5日 | 件数増加に対する耐性 |
| 7 | 3.2, 3.3 | リプレイ・バックグラウンド | 2〜3日 | 長い Run と前景の応答性 |
| 8 | 4 | 常駐・保持期間・release | 3〜5日 | 長期運用とメモリ |

**最初の1週間（Quick Wins）**: Phase 0 + 1.1 + 1.2 + 2.1 + 2.3。
ここだけで「起動が遅い」「ストリーミングが重い」の体感の大半に効く見込み。

---

## リスクと対策

| リスク | 対策 |
| :--- | :--- |
| バックアップ頻度を下げて復旧不能になる | マイグレーション前は必ず取得。日次スナップショットは維持 |
| MCP を非同期化して最初の Run でツールが欠ける | 最初の Run だけ準備完了を待つ。UI には「接続中」を表示 |
| 検索候補の絞り込みで順位が変わる | 共有 fixture をゲートにし、ベンチで p95 と recall を同時確認 |
| `-unormal` で未追跡ファイルの表示が変わる | 変更ペイン表示時のみ `-uall` に切り替える |
| `opt-level=3` で配布サイズが増える | 10% 以内を条件に採用。超えるならホットクレートのみ |
| リプレイのバッチ化で契約が二重化する | 既存 `applyRunEvent` を共有し、チャンクを一括適用する形にする |

---

## やらないこと

- CI にレイテンシの閾値を入れる（マシン依存でフレークする）。
- 効果を測らずに「とりあえず `memo` を増やす」「`useMemo` を増やす」。
- 仕様（`docs/memory-spec.md`, `docs/desktop/architecture.md`）のランキング式や
  イベント契約を、性能目的で変える。
- Docker 版とのパリティを壊す最適化。
