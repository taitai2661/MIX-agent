# MIX-agent Desktop — Architecture（Phase 6 完了時点）

## 目的

`docker/MIX-agent/` 版の中核機能（Provider Adapter / Run Engine / Tool Registry / Run イベントの `kind` 契約 / Capability 3 状態 / Checkpoint / Stagnation / Verification / Tiered Summary / Budget Extension / 暗号化 Secret）を **Tauri 2 + Rust + React 19** のネイティブアプリへ再実装する。`docker/` との **コード共有ゼロ**、仕様書のみで契約を結ぶ。

## ディレクトリ

```
MIX-agent/
├── apps/
│   ├── desktop/                # Desktop 版（Cargo + pnpm workspace）
│   │   ├── app/         Tauri 2 shell + #[tauri::command] surface
│   │   ├── core/        Run Engine / state / events / context
│   │   ├── providers/   OpenAI / Anthropic / Gemini / OpenAI-Compatible / Ollama / LM Studio
│   │   ├── tools/       Tool Registry + executor
│   │   ├── browser/     Browser Operator + chromiumoxide backend
│   │   ├── mcp/         MCP client host
│   │   ├── storage/     SQLite + Keychain
│   │   ├── scripts/     fetch-chromium.mjs
│   │   └── ui/          React 19 + Vite + Tailwind (Desktop独自UI)
│   ├── server/                 # Docker 版バックエンド (FastAPI + Python)
│   └── web/                    # Docker 版フロントエンド (React 19 + Vite)
├── docs/                       # ドキュメント (desktop/ 含む)
├── scripts/                    # パリティチェック、アイコン生成など
└── compose.yaml                # Docker Compose 設定
```

## 信頼境界

```
┌──────────────────────────────────────────────────────────────┐
│ Tauri App (tokio runtime 上の一つのバイナリ)                 │
│                                                              │
│  WebView ─ invoke('cmd') ─► #[tauri::command]   ◄── emit()   │
│  (WKWebView / WebView2 / WebKitGTK)                          │
│                                                              │
│  mixd-core (Run Engine, events, context)                     │
│  mixd-providers (Provider adapters)                          │
│  mixd-tools (Tool Registry, executor)                        │
│  mixd-browser (Browser Operator, chromiumoxide backend)      │
│  mixd-mcp (rmcp host, OAuth PKCE)                            │
│  mixd-storage (sqlx+SQLite WAL, keyring-rs)                  │
│                                                              │
│  └─ 子プロセスとして実 Chromium を起動（Browser Operator）    │
└──────────────────────────────────────────────────────────────┘
```

DB は同一プロセスの tokio runtime 上で in-proc に開かれます（Docker 版の
PostgreSQL コンテナに相当する分離はありません）。ただし Browser Operator は
実 Chromium を**子プロセスとして**起動するため、単一の OS プロセスではありません。

WebView からは `fs` / `shell` / `http` プラグインの権限を `app/capabilities/default.toml`
で一切付与していません（明示的な deny ではなく不許可）。すべての操作が
`#[tauri::command]` 経由となり、Rust が信頼の単一源泉です。

## Docker 版との整合点

| Docker 版 | Desktop 版 | 共有方針 |
| --- | --- | --- |
| `routes.py` の URL 契約 | `#[tauri::command]` 引数・戻り JSON | 形の対応関係を維持（対応表は [`api-mapping.md`](api-mapping.md)） |
| Run イベントの `kind` 値一覧 | `mixd_core::runs::engine::RunEvent` | 共通分は同じ `kind` 名。差分は下述 |
| Capability 3 状態 | `mixd_core::routing::CapabilityState` | 仕様踏襲 |
| Checkpoint / Stagnation / Verification | `core::runs::{checkpoint_store,stagnation,verification}` | 仕様踏襲 |
| Tiered Summary | `core::context` | 仕様踏襲 |
| 暗号化 Secret (AES-GCM + AAD) | `storage::secrets` | 仕様踏襲 |
| pg_trgm / JSONB Index | SQLite `json_extract` + FTS5 | 再実装 |
| マルチユーザー / 管理者作成 | 単一ユーザー前提 | 削除 |
| Docker container 群 | tokio runtime 上の in-proc タスク | 置換 |

### Run イベントの `kind` 差分

`app/src/run_events.rs` は Docker 版が SSE で流すのと同じ JSON 形状を、
Tauri の `run_event` というイベント名で emit します。
ただし `kind` の一覧は **完全一致ではありません**。

共通（両エディションが使う）: `text` / `reasoning` / `message` / `status` / `error` /
`model_started` / `model_selected` / `tool_started` / `tool_result` /
`context_summary` / `checkpoint_saved` / `budget_extension_requested` /
`budget_extension_resolved` / `stagnation_detected` / `verification_required`

Desktop のみ: `tool_denied` / `tool_approval_requested` / `tool_approval_resolved` /
`destructive_tool_called` / `verification_failed` / `todo_list` / `awaiting_approval`

Docker のみ: `model_rerouted` / `plan` / `approval` / `browser_frame` / `phase_advanced`

UI 側の解釈は `ui/src/lib/runEvents.ts` の `applyRunEvent()` が持ちます。
`default` 分岐で未知の `kind` は黙って無視されるので、Desktop 側で
`kind` を増やすときは同じ switch にケースを追加してください。

## Browser Operator

```
Agent (mixd-core::runs::engine)
   │
   ▼ call_tool("browser", ...)
Browser Tool (mixd-tools)
   │
   ▼ trait BrowserTool
Browser Tool Interface (mixd-browser::interface)
   │
   ▼
Browser Operator (mixd-browser::operator)   ← Workspace Profile 保持
   │  navigate / click / type / scroll /
   │  dom / screenshot / evaluate / wait_for
   ▼
   Backend: enum BrowserBackend  ← 差し替え可能
          Chromium          = 同梱 Chromium を起動
          SystemChromium    = システム Chromium にフォールバック
          Disabled          = 実行ファイルが見つからない
   │
   ▼
   Bundled Chromium (~370MB/プラットフォーム) or system Chromium
```

`mixd-core` は `chromiumoxide` に依存しません。backend の差し替えだけで
Browser 実装の差し替えが可能で、`BrowserBackend` は 3 状態
（`Chromium` / `SystemChromium` / `Disabled`）を持ちます。

## セキュリティ

- **OS Keychain**: `keyring-rs` で Provider Key / DB 暗号化鍵 / MCP token / OAuth refresh token を用途別 AAD（`"{purpose}|{name}"`）で保存。
- **DB 暗号化設定 (Codex 互換)**: 標準は **OFF（平文 SQLite）** で動作し、所有者のみ読み書き可（0600）のアクセス権で保護されます（Codex parity）。設定（GeneralSettings）から保存時の暗号化を **ON / OFF** 切り替え可能で、ON に設定された場合は OS Keychain にマスター暗号化鍵（`db.master`）が生成・保管されます。診断画面（Diagnostics）でも暗号化状態が確認できます。
- **Egress allowlist + private IP deny**: `tools::network` が reqwest 実行前に検査。allowlist が空なら公開ホストのみ許可し、private IP は常に拒否します。保存済みルールは `*.example.com` のようなワイルドカードに対応します。
- **WebView CSP**: `app/tauri.conf.json` の `app.security.csp` で `connect-src 'self' ipc: http://ipc.localhost` に限定。
- **window 設定**: `app.macosPrivateApi = true` は `titleBarStyle: Overlay` +
  `hiddenTitle` + `trafficLightPosition` が macOS の private API を使うための要件です
  （無効化するとカスタムタイトルバーが正しく描画されません）。`Cargo.toml` の
  `tauri` 依存の `macos-private-api` feature と対になっています。
- **OS プロセス sandbox**: **未実装**。macOS `sandbox-exec` / Windows Job Object / Linux landlock + bubblewrap のいずれもコードに存在しません。`terminal` tool はユーザーの権限で任意のプロセスを起動でき、`browser` tool も外部 Chromium を無制限に操作できます。敵対的なコードを安全に実行する用途には使わないでください。

## Bundled Chromium

Desktop 版はブラウザを同梱する。取得は `scripts/fetch-chromium.mjs` が
Chrome for Testing の公開エンドポイントから行い、`app/resources/chromium/` に
次のレイアウトで展開して `.gitignore` 対象（1プラットフォーム約 370 MB）になります。

| OS | パス（第一候補） | フォールバック候補 |
| --- | --- | --- |
| macOS | `chromium/chrome-mac/Chromium.app/Contents/MacOS/Chromium` | `chrome-mac/Google Chrome for Testing.app/…`、`chromium/Chromium.app/…` |
| Windows | `chromium/chrome-win/chrome.exe` | `chromium/chrome.exe` |
| Linux | `chromium/chrome-linux/chrome` | `chromium/chrome` |

`tauri.conf.json` の `bundle.resources` は **map 形式**（source → target）で
`"resources/chromium": "chromium"` を宣言し、macOS バンドルでは
`Contents/Resources/chromium` 配下に配置されます。

探索は `mixd_browser::profile` が行います。

- `find_bundled_chromium()` が上の候補を順に `is_file()` で探します。
- 基準ディレクトリの選定は `resolve_resource_dir()` が担当します。
  パッケージ済みアプリでは `app.path().resource_dir()` を優先します
  （`app_data` から推測しません）。一方 debug ビルド（`cargo` / `tauri dev`）では
  `resource_dir()` が `target/debug` を指し、`tauri-build` がミラーした Chromium は
  macOS framework のシンボリックリンクを欠いた不完全なツリーになるため、
  `app/resources` の pristine な実体を優先します。`env!("CARGO_MANIFEST_DIR")` は
  `cfg` で release から切り離されるため、ローカルパスが配布バイナリに焼き込まれません。

同梱が無い場合は system Chromium（macOS / Windows / Linux の既知パス）に
フォールバックし、それも無ければ Browser 機能を `disabled` として報告します。

```bash
node scripts/fetch-chromium.mjs              # 安定版を取得
MIX_SKIP_CHROMIUM=1 cargo check              # 取得をスキップ
MIX_CHROMIUM_VERSION=154.0.8037.57 node scripts/fetch-chromium.mjs  # 固定
```

`pnpm tauri build` の `beforeBuildCommand` が UI ビルド後に自動で取得します。
macOS 配布時は同梱の `Chromium.app` も署名対象になります。signing / notarization /
staple の各手順は `desktop-release.yml` に実装済みです（署名証明書のインポート →
ビルド時署名 → notarization + staple）。いずれも secrets（`APPLE_CERTIFICATE` /
`APPLE_SIGNING_IDENTITY` / `APPLE_ID` / `APPLE_APP_SPECIFIC_PASSWORD` /
`APPLE_TEAM_ID`）が無い場合はスキップされ、無署名ビルドとして警告付きで継続します。
配布に署名・公証が必要な場合は、これらの secrets を用意してください。

## 自動更新

`app/src/updater.rs` が起動 20 秒後に更新を確認し、新しい版があれば
`update_available` を emit します。UI はそれを受け取って上部バナーおよび「設定 ▸ アップデート」画面に表示し、
「インストールして再起動」でダウンロード（進捗プログレスバー表示）→ インストール → 再起動を行います。
起動時の自動確認は設定の `auto_update`（有効/無効）に従います。
また、「設定 ▸ アップデート」タブおよび `Help ▸ Check for Updates` からの手動確認・リリースノート確認も可能です。

**配信エンドポイント:**
GitHub Releases の `https://github.com/taitai2661/MIX-agent/releases/latest/download/latest.json` を使用します。
CI（`.github/workflows/desktop-release.yml`）内で、各プラットフォームビルド後に `scripts/merge-updater-manifests.mjs` が自動実行され、マルチプラットフォーム（macOS arm64/x64, Windows, Linux）対応の単一 `latest.json` が生成・発行されます。

署名鍵の準備・有効化手順:

```bash
# 1) 署名鍵を生成する（機密: リポジトリにコミットしないこと）
pnpm --filter mix-agent-desktop-workspace tauri signer generate -w ~/.tauri/mix-agent.key \
  -p "MIX-agent Desktop" --force

# 2) 生成された公開鍵（~/.tauri/mix-agent.key.pub）の文字列を
#    app/tauri.conf.json の plugins.updater.pubkey に登録

# 3) 生成された秘密鍵の内容およびパスワードを GitHub リポジトリの Secrets に設定:
#    - TAURI_SIGNING_PRIVATE_KEY
#    - TAURI_SIGNING_PRIVATE_KEY_PASSWORD
```

ローカル環境等で `pubkey` が未設定またはエンドポイントへの疎通ができない場合でも、アプリ起動が失敗しないよう安全にフォールバックされます。

## Backup / Restore

`mixd_storage::backup` が実体を実装している。

- ペイロードは `VACUUM INTO` で取った整合性のある SQLite スナップショット
  （稼働中の WAL DB をファイルコピーすると破れたページを取り込むため不可）。
- `tar` → zlib → AES-256-GCM。鍵は Argon2id でパスワードから導出。
- コンテナヘッダは `MIXBAK\x01\x00` + salt(16) + nonce(12) + ciphertext。
- **API key は含めない**。OS Keychain にあり、バックアップに書くと復号鍵が
  ディスクへ平文で出るため。復元後に再入力が必要。
- Restore は復号を検証したうえで `mix-agent.restored.db` に**ステージング**
  する。実行中プロセスは SQLite プールを開いているため、その場でファイルを
  差し替えると破損する危険がある。`Storage::open` が起動時に
  `mix-agent.db.pre-restore` へ退避してから差し替えるので、再起動 1 回で反映される。

## Schedule

`mixd_core::schedule::CronSchedule` が 5 フィールド / 6 フィールド（秒付き）と
`@daily` などのマクロを解釈し、`@daily` 等の Vixie cron の「日-of-month と
day-of-week の両方指定時は OR」 semantics を再現する。

`app/src/scheduler.rs` の daemon が 20 秒ごとに起床し、
`(anchor, now]`（anchor = 最終実行時刻、初回は作成時刻）に落入る発火時刻が
あれば Run を開始する。発火時に `lastRunAt = now` を書くため同じスロットは
二度実行されず、アプリ停止中に取りこぼしたジョブは次回起動時に**ちょうど1回**
追従する（取りこぼし分をまとめて再生しない）。

UI は cron 式を入力中に `preview_cron` で次回5回を表示し、誤記は登録時に拒否する。

## フェーズ

| Phase | 内容 | 状態 |
| --- | --- | --- |
| 0 | 雛形 / workspace / capabilities / CI | ✅ |
| 1 | storage + providers + 単発Chat | ✅ |
| 2 | tools + permission gate + egress | ✅ |
| 3 | Agent 自律性 + context 圧縮 / retrieval | ✅ |
| 4 | MCP host + Browser Operator | ✅ |
| 5 | Memory / Knowledge / Skills / Projects / Schedules / Agents | ✅ |
| 6 | tray / menu / notification / auto-update / backup / release CI | ✅ |
| 7 | macOS 完成 + Windows α + Linux α | ⏳ |

Phase 6 はコードと配布 CI の基盤まで入っています。定義は「更新なしの release CI」
として確定しています（`desktop-release.yml` は `.sig` / `latest.json` を発行せず、
`plugins.updater` は `active = false` / `endpoints: []` / `pubkey = ""` のまま）。
署名・公証の手順自体は CI に含まれますが、更新配布は行いません。
Phase 7（macOS 完成 + Windows α + Linux α）が未着手です。

## 空チャットの挨拶（2026-10-01 追記）

スレッドが空のときの挨拶は、`ui/src/features/chat/greeting/` が Signals から文を
合成して表示します。入力は 4 系統です。

| 系統 | 内容 |
| --- | --- |
| 日時 | 時間帯 8 区分・曜日 5 種・季節と季内位置・年内の日数 |
| 暦 | 日本の祝日（固定 14 種・ハッピーマンデー・振替休日・国民の休日）、二十四節気 24 種（太陽視黄経から日本標準時 0 時単位で算出）、季節の行事 16 種 |
| ワークスペース | スレッド数・メッセージ数・プロジェクト数・最終更新からの経過・サーフェス（`chat` / `desktop-agent`）・プロジェクト選択の有無 |
| ブラウザ履歴 | `localStorage`（`mix-agent-welcome`）の訪問間隔・初訪問時刻・直近 24 件の部品 ID |

見出しは「挨拶＋状況節＋締め」の 3 層を独立抽選し、文体タグ
（`polite` / `calm` / `casual`）と文形タグ（`question` / `declarative`）で互換判定
してから連結します。文形タグは文末記号から導いており、`sentence()` ヘルパーに
直接書けないためここが単一の情報源です。日本語は間なしで、英語は空白区切りで連結し、
`t()` は文単位で適用します。言語辞書の件数は `pnpm ui:test`（`i18n.keys.test.ts` の孤立キー検査）の出力で確認してください。

Docker 版（`docker/MIX-agent/docs/verification.md` の 2026-10-01 節）と同じ方式ですが、
`desktop/docs/architecture.md` の冒頭の方針どおりコードは共有していません。Desktop 版は
`スレッド` / プロジェクト / 差分 / Browser など自分の語彙で書き下し、提案も
Desktop が実際に持つ機能（差分・テスト・ブラウザ・プロジェクト）に寄せています。
既存のクイックプロンプト 4 件は提案プールに吸収したので `i18n.keys.test.ts` の
孤立キーは増えていません。

### 確認した項目

- UI の vitest（`pnpm ui:test` で件数を確認）が成功。うち greeting 配下が 7 ファイル。
- 二十四節気 24 種が、国立天文台「暦要項 令和 8 年 (2026)」の中央標準時表記と一致。
  分点瞬間は既知値（1990 / 2000 / 2026 年の春分）と 3〜8 分の差。
- 祝日 16 件（固定祝日・ハッピーマンデー・2026-05-06 の振替休日・2026-09-22 の
  国民の休日）を固定日付でアサート。
- 2 年分の 6570 通り（730 日 × 9 時間帯）の走査で、見出し 2205 通りと
  疑問符の連続・末尾記号・空白混入・英語訳の欠落がないことを確認。
  `desktop-agent` サーフェスでも同じ走査を別に実施しています。
- `pnpm ui:lint`（`--max-warnings 0`）と `pnpm ui:build` が成功。
  バンドルサイズは `pnpm ui:build` の出力で確認してください（500 kB 超の警告は
  変更前から出ています）。

### 未確認の項目

- 実機（Tauri / WebView）での画面表示、スクリーンリーダー、macOS 以外の OS での確認。
- 提案チップのクリックで送信欄が埋まる動作の実機確認。
- 英語表示での文の連結の品質確認。英訳は文単位へ対応させており、ネイティブ話者による
  レビューは未実施です。
- Rust 側のテスト（`cargo test` / `clippy` / `fmt`）は UI のみの変更のため
  実行していません。
- `localStorage` を保存できないプライベートモードでの実挙動は未確認です。

## 自律性・記憶・ツール実行の強化（2026-10-05 追記）

Phase 3〜5 の実装に対する、コード監査で確定した欠陥の修正。

- **検証ゲートをターゲット単位化**（`core/src/runs/verification.rs`）。台帳のキーをツール名から
  効果対象（パス／コマンド）に変更し、`a.rs` を読んでも `b.rs` の未確認書き込みは解除されない。
  登録は dispatch **前**に行い、同一ターン内の `write` → `read` で正しく閉じるようにした。
  `unverified_repair_turns` は台帳が空になった時点でリセットされ、複数ファイル作業でも
  2 回で打ち切られない。破壊的コマンド判定もトークン／リダイレクトベースに置き換え、
  `"rm "` が `"confirm "` に一致する誤検知を解消し、`find -delete` / `git restore` などを追加。
- **web ツールの重複検知**（`core/src/runs/stagnation.rs`）。`read_key` が `query` / `url` を
  見るようにし、無関係な 2 回目の検索が ReadLoop と誤判定されない。
- **todo の per-run 化**（`tools/src/todo.rs`）。`TodoStore` を `run_id` でキー化し、サブエージェントや
  スケジュール実行が親のチェックリストを上書きしない。
- **resume の状態保持**（`app/src/commands/runs.rs`, `core/src/runs/{engine,checkpoint_store}.rs`）。
  Run に `workspaceId` を保存して再開時に再利用し、checkpoint に `tool_calls` を載せて
  予算カウンタを引き継ぐ。
- **キャンセル伝播**。ツール dispatch・provider ストリームを `cancelled_signal` と race させ、
  途中キャンセルを `Cancelled` として終了。打ち切り時は残りの tool call に結果を補い、
  transcript に dangling な `tool_calls` を残さない。`terminal` の `max_wall_clock_ms` は
  10 分に上限。
- **検索の改善**（`core/src/retrieval.rs`）。CJK を unigram/bigram に分割、Memory は関連度 0 を除外し
  `active`/`candidate`/`established` のみ対象、取得した Memory を `reinforce`、Knowledge は
  文書あたり 2 チャンク、出力は項目 1200 字・全体 8000 字で打ち切り、「データであって指示ではない」と明示。
- **memory ツール**（`tools/src/memory.rs`, `app/src/lib.rs`）。エージェントが
  `recall` / `remember` / `forget` / `search_knowledge` を実行できる（`MemoryDispatch` 経由）。
- **revision 復元の所有チェック**（`core/src/memory.rs`）。
- **ツール実行の安全策**（`tools/src/network.rs`, `registry.rs`, `files.rs`）。
  HTTP 本文は 2 MiB で打ち切り、クロスオリジンリダイレクトで `Authorization`/`Cookie` を除去、
  web_search の件数と snippet を制限、browser の `navigate` に egress を適用、
  `files.search` の 1 行を 500 字に制限。

### 今回も未実装のもの

- **Knowledge の embedding 検索 / FTS5**。Memory 側は hybrid 検索（lexical +
  embedding、`retrieve_memories_hybrid`）実装済みだが、Knowledge は CJK 対応の
  lexical 検索のみで、ベクトル列・FTS5 インデックスは未整備。
- PTY の対話 terminal（stdin 送出）。timeout 上限のみ追加。
- `browser` の `evaluate` はページ内 `fetch` でローカル資源に到達し得る。`navigate` の
  egress は塞いだが、完全な隔離ではない。

## UI/UX の改善（2026-10-05 追記）

- **スレッドのサーフェス絞り込み**（`ui/src/features/shell/Sidebar.tsx`）。store には
  `threadFilter` / `setThreadFilter` が以前からありテストもあったが、UI が無く死んだ状態
  だった。サイドバー上部に `Segmented` を置き、会話の `mode` から所属サーフェスを引く
  `surfaceForMode()`（`ui/src/lib/ui.ts`）で「すべて / チャット / エージェント」を切り替える。
- **スレッド一覧のキーボード移動**。`↑` / `↓` / `Home` / `End` でタイトル間を移動する。
  rename 入力中は `[data-thread-open]` ガードで文字カーソル移動を妨げない。
- **ショートカット一覧**（`ui/src/features/shell/ShortcutHelp.tsx`）。`?` またはコマンド
  パレットから開き、`useAppShortcuts` のバインドを列挙する。入力欄にフォーカスがあるときは
  `?` を文字として通す（`e.target` が Element でない場合も含めてガード）。
- **失敗の可視化**。スレッドの Markdown エクスポートとアップデートのインストールは、失敗して
  も何も表示されない unhandled rejection だった。サイドバー / 更新バナーに `Alert` を出す。
- **ストリーミング時のレイアウト再計算**（`ui/src/features/shell/usePanelCap.ts`）。パネル幅の
  キャップを測る `MutationObserver` が、ストリーミング中に増えるノードごとに
  `getBoundingClientRect` を呼んでいた。1 フレームに 1 回へ間引く。

### 確認した項目

- `pnpm ui:typecheck` / `pnpm ui:lint`（`--max-warnings 0`）/ `pnpm ui:test`
  が成功（件数は `pnpm ui:test` で確認すること）。
- `pnpm --filter mix-agent-desktop-ui format:check`（prettier）が成功。
  既存ツリー全体への適用は未実施のため CI ゲートには未追加。新規ファイルは
  追加時に整形すること。
- モック IPC で Tauri を介さず dev プレビューを起動し、フィルター切替でエージェント 2 件 /
  チャット 4 件に絞られること、`?` で一覧が開き入力欄では開かないこと、
  1440 / 1024 / 820px で横スクロールが出ないことを DOM で確認。
- 実機（Tauri / WebView）での目視とスクリーンリーダーは未確認。

## 既知の残タスク

- auto-update の有効化（署名鍵 `TAURI_SIGNING_PRIVATE_KEY` の用意と
  `latest.json` 発行基盤 — Phase 6 は「更新なしの release CI」として確定済み）。
- Knowledge は lexical 検索のみ。embedding 検索は未実装（上記参照）。
- Memory の自動生成（Run 後の抽出）は未実装。手動 CRUD + revision + `memory` ツールのみ。
- シグナル / PTY ベースの対話 terminal（現在は上限付きの実行のみ）。
- OS プロセス sandbox（macOS / Windows / Linux とも未実装）。
- Provider プリセットは Docker 版の 52 件中 50 件（`cloudflare` と
  `azure-ai-foundry` が未移植）。

## ライセンス

**確定: Apache License 2.0**。リポジトリ全体の `LICENSE` が Apache-2.0 であり、
Desktop 側も `desktop/mix-agent-desktop/LICENSE` と `Cargo.toml` の `license` を
含めて Apache-2.0 に統一されている（MIT ではない）。
変更する場合はルート `LICENSE` / `NOTICE` / `desktop/mix-agent-desktop/NOTICE` と
`desktop/mix-agent-desktop/Cargo.toml` の `license` を同時に書き換えること。

## 関連ドキュメント

- [`docker/MIX-agent/docs/architecture.md`](../../docker/MIX-agent/docs/architecture.md)（Docker 版アーキテクチャ）
- [`desktop/README.md`](../../desktop/README.md)（Desktop 版入口・セットアップ / 開発フェーズ）
