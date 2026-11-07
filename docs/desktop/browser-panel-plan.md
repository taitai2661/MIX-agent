# Desktop 右サイドパネル / ブラウザセッション刷新（計画）

> 状態: 実装済み（2026-11-06）。その後、Desktop は Tauri 子 WebView 方式へ移行し、Chromium / screencast は削除した（`webview-browser-plan.md` を参照）。
> 対象: `apps/desktop/ui` / `apps/desktop/browser` / `apps/desktop/app`
> 最終更新: 2026-11-06
> 関連: [`architecture.md`](architecture.md) / [`webview-browser-plan.md`](webview-browser-plan.md)

---

## 0. 結論

右サイドパネルのブラウザを「設定画面の延長」から **Codex のような
ファーストクラスのブラウザ面** に引き上げる。

1. **見た目**: Chrome と同じ順序の「タブ → アドレスバー → ページ」。
   アプリのセグメント見出しはブラウザ表示中だけ隠し、ページを全面に出す。
2. **比率**: 分割比を固定プリセット（380 / 560px）から **自由なドラッグ** に変える。
   既定は 50%。エージェントが明示した場合だけその比率を一度適用する。
3. **セッション**: タブ順の安定化、タブ復元、プロファイル／ログインの
   管理、エージェント操作中の表示を追加する。
4. **安全側**: ファビコンは外部取得しない（WebView からの直 fetch は
   egress ゲートを迂回するため）。頭文字アバターで代替する。

実装は P0〜P3 の 4 段階。最初の PR は **P0（比率）＋ P1（Chrome 風シェル）**
に絞る。想定は合計 7〜12 日。

### 実装メモ（2026-11-06）

- P0〜P3 をまとめて実装した。自由比率、Chrome 風シェル、タブ順の安定化、
  自動復元、`split_ratio`、右パネルからのセッション操作を含む。
- 追加で `Emulation.setDeviceMetricsOverride` によるビューポート同期を入れた。
  パネルの実寸に合わせてページを描画するため、縦長スプリットでも
  スクリーンキャストがレターボックスにならない。
- 自動復元はユーザー選択により「バックエンドが空のときだけ自動」。
  復元チップは出さない。
- ログイン情報のリセットは `⋮` メニューに移し、既存の確認ダイアログを使う。

---

## 1. 現状

- `RightPanel` が全パネル共通のヘッダー（アクティビティ / 使用 / 変更 /
  ブラウザのセグメント）を描画する。
- `BrowserPanel` はその下に `BrowserToolbar` → `BrowserTabStrip` →
  `BrowserViewport` の順で並ぶ。タブがツールバーの下にあり、Chrome とは逆。
- 幅は `rightPanelWidth`（px、既定 380、`MAX_RIGHT_PANEL_WIDTH = 900`）で、
  広げる操作は 380 ↔ 560 のトグルのみ。`usePanelCap` が狭いウィンドウで
  さらに `max-width` を掛ける。
- タブは `browser_invoke` の `info` を 2 秒ごとに読む。`list_tabs()` は
  `chromiumoxide` の `pages()`（HashMap）順なので、**ポーリングごとに
  並びが変わり得る**。
- セッション（プロファイル）はワークスペース単位でディスクに残るが、
  右パネルからはリセット・確認ができない（設定画面のみ）。
- ライブ表示はスクリーンキャスト + 2 秒スナップショットのフォールバック。
  再接続状態や「エージェント操作中」はツールバーの小さなチップだけ。
- エージェントの `browser` ツール開始時に自動オープンするが、幅は
  ユーザーの記憶値のまま。エージェントが比率を指定する経路は無い。

---

## 2. 目標 / 非目標

### 目標

- ブラウザ表示中は Chrome に近い 3 段構成（タブ / ナビ / ページ）。
- サイドパネル幅を **中央カラム 300px を残す範囲で自由** にドラッグできる。
- ブラウザの比率は既定 50%。エージェントが `split_ratio` を渡したときだけ
  その値へ寄せる。ユーザーがドラッグしたら以後はユーザー優先。
- タブ操作（新規 / 切替 / 閉じる / 再読込）をキーボードとマウスで完結。
- セッションの状態（ワークスペース、ログイン、エージェント操作中）が
  右パネルから分かる。ログイン情報のリセットも右パネルからできる。
- 右パネルを閉じて開き直したとき、タブ構成が実用上保たれる。

### 非目標

- Chromium の可視ウィンドウ表示（常に new headless を維持）。
- ブラウザのタブをドラッグで並べ替える（CDP に並び順の概念が無い）。
- ファビコンのネットワーク取得（egress 迂回になるため行わない）。
- 複数プロファイルの同時表示。プロファイルは従来どおり
  「1 ワークスペース = 1 プロファイル」。
- Docker / Web 版の `browser_*` ツール群の変更。Desktop の presentation
  ヒントだけを追加する。

---

## 3. 設計

### 3.1 目標レイアウト

```
┌──────────────────────────────────────────────┐
│ [● Google ×] [＋]              [⤢] [×]      │  ← タブ（Chrome 風）
├──────────────────────────────────────────────┤
│ ← → ↻ │ 🔒 https://www.google.com/  │ ↗ ⋮ │  ← ナビ / オムニボックス
├──────────────────────────────────────────────┤
│                                              │
│                 ページ（全面）                │
│                                              │
└──────────────────────────────────────────────┘
```

- `RightPanel` のセグメントヘッダーは `rightPanel === "browser"` のとき描画しない。
  パネル切替はタブ列右端の「⋮」メニューに移す。
- `BrowserTabStrip` を最上段へ、`BrowserToolbar` を 2 段目へ移動。
- ビューポートの `p-2` / 枠線を外し、`flex-1` で全面表示。
- 背景は `bg-bg`、非アクティブタブは `bg-inset/40`。既存のテーマトークンを使う。

### 3.2 自由な分割比率

固定 px の上限をやめ、**利用可能幅から最大値を毎回計算**する。

- `usePanelCap` を拡張し、右パネル用の `maxWidth` も返す
  （`usePanelBounds(row)` → `{ cap, rightMax }`）。
  - `rightMax = row.clientWidth - 左パネル幅 - ファイルツリー幅 - 300`
  - 中央カラムの最低 300px は現行 `CENTER_MIN` を踏襲。
  - 狭いときは `MIN_RIGHT_PANEL_WIDTH`（320px）を下限にクランプ。
- `ResizableHandle` の `max` に `rightMax` を渡す。`MAX_RIGHT_PANEL_WIDTH`
  は安全弁として 1600px 程度に上げ、実質は `rightMax` で決める。
- ドラッグ中は従来どおりローカル state、`pointerup` で永続化。
- ダブルクリックで 50% に戻す。キーボードは既存の ←/→/Home/End を維持。
- ウィンドウリサイズ時は「比率」を保つのが自然なので、ブラウザパネルに限り
  `ratio = width / rowWidth` を保存し、リサイズ時に再計算する。
  - `rightPanelRatio: number | null`（`null` = px 記憶値を使う）
  - 他パネル（使用 / 変更 / 差分 / ファイル）は従来の px のまま。

### 3.3 エージェントによる比率指定

「指定しない限り自由」を次のように実装する。

- `openBrowserPanel(by, options?: { ratio?: number })` を追加。
  - `by === "user"`: 記憶している比率 / 幅をそのまま使う。
  - `by === "agent"`: `options.ratio` があれば 0.2〜0.8 にクランプして適用。
    無ければ 0.5（初回）またはユーザーの記憶値。
- エージェントの指定経路は `browser` ツール引数の `split_ratio`（任意、0.2〜0.8）。
  - `apps/desktop/browser/src/interface.rs` の `browser_tool_spec()` に追加。
  - 実行時は無視してよい（presentation ヒント）。Desktop のみ。
  - `useShellEvents` の `tool_started` で `arguments.split_ratio` を読み、
    `openBrowserPanel("agent", { ratio })` または `setBrowserPanelRatio(ratio)`
    を呼ぶ。
- ユーザーがドラッグしたら `rightPanelRatio` を更新し、以後の
  エージェント指定が無い限りその比率を維持する（ロックはしない）。

### 3.4 タブ / セッション

#### タブ順の安定化（UI 側）

`useLiveFrame` にタブ順の ref を持つ。

- 新しく現れた `tab.id` は末尾に追加。
- 既知の id は現在の順序を維持。
- 消えた id は除去。
- `tabs` state はこの順序で並べ替えて返す。

バックエンドの `pages()` が HashMap 順でも、UI が毎ポーリングで
タブを入れ替えない。Rust 側の変更は不要。

#### タブストリップ

- 頭文字アバター: `host` の先頭 1 文字 + 安定した色（hash）。
  `about:blank` は Globe。
- アクティブタブ: `bg-bg` + 下線なしの角丸タブ。非アクティブ: `text-muted`。
- 閉じるボタンはアクティブ / hover / focus で表示。
- 中クリックで閉じる。`⌘W` はブラウザ内フォーカス時だけ close_tab。
- `＋` は `new_tab`。タブが多い場合は横スクロール（既存 `overflow-x-auto`）。
- タブ本数が 1 のときは閉じるボタンを出さない（最後のタブは
  `about:blank` に戻す扱いにするか、`close_tab` を許可するかは実装時に統一）。

#### セッションメニュー（⋮）

`BrowserToolbar` の右端に `DropdownMenu` を追加し、以下を集約する。

- 新しいタブ / 再読み込み / 停止
- フル表示 ↔ スプリット表示
- エージェント使用時の自動表示 ON/OFF
- このセッション: ワークスペース ID の短縮表示、プロファイル状態
- ログイン情報をリセット（`browserResetProfile`、危険確認つき）
- システムのブラウザで開く
- ブラウザ設定を開く
- 閉じる

これによりツールバーは「戻る / 進む / 再読込 / アドレス / ↗ / ⋮」に絞る。

#### セッション復元

- `localStorage` に `mix-agent-browser-session:<workspaceId>` を保存:
  `{ tabs: [{ url, title }], activeIndex, savedAt }`。
- `refreshInfo` のたびに更新（URL が `about:blank` のタブは除外）。
- パネルを開いた直後、バックエンドのタブが空 or `about:blank` 1 枚だけで、
  保存が 7 日以内なら自動復元する。ユーザーが `⋮` から全タブを閉じた場合は
  スナップショットも消すため、次回は復元しない。
- 復元は最初の URL を既存の `about:blank` タブへ `navigate` し、残りを
  `new_tab` で開き、最後にアクティブだったタブへ切り替える。
- アプリ再起動後も Cookie / ログインは Chromium プロファイルが保持する。
  タブの URL だけをこのスナップショットで補う。

#### バックエンド eviction からの復帰

プロセスが LRU で退避された場合は、パネルを開いたときの
`browser_session` が `null` を返し、保存済みスナップショットから自動復元する。
`browser_session` は Chromium を起動しないため、単にパネルを開いただけで
プロセスが立ち上がることはない。

### 3.5 ライブ表示

- `useLiveFrame` に `connection: "live" | "fallback" | "reconnecting"` を追加。
  - screencast が 3 秒途絶えたら `fallback`（既存）。
  - `info` も 3 回連続失敗したら `reconnecting`。
- `document.hidden` の間は screencast を止める（現状はポーリングだけ停止）。
  再表示時に `acquireScreencast` し直す。
- エージェントの `browser` ツールが running の間は、ツールバーに
  「エージェント操作中」を表示し、ビューポートの枠を `border-brand/40` にする。
  手動クリックは禁止しない（サーバー側で workspace ロック済み）。
- `frameError` はツールバー下の 1 行ステータスに出す（現在はビューポート下）。

### 3.6 セキュリティ

- ファビコンを `/favicon.ico` 等で取りに行かない。WebView からの
  サブリソース取得は egress ゲートを通らないため、ページが
  `link rel=icon` で私設アドレスを指すと迂回になり得る。
- 頭文字アバターは URL の host 文字列だけから生成する。
- セッションメニューの「ログイン情報をリセット」は既存の
  `useConfirm` の danger 確認を必須にする。
- タブ復元は保存済み URL を `new_tab` に渡すだけ。`gate_navigation` を
  通るので、私設アドレスや `file:` は従来どおり拒否される。

---

## 4. 実装フェーズ

### P0: 比率（1〜2 日）

- `usePanelCap.ts`: `rightMax` を返す `usePanelBounds` を追加。
- `RightPanel.tsx`: `rightMax` を `ResizableHandle` に渡す。
  ブラウザ時は 50% 既定、ダブルクリックリセット。
- `lib/ui.ts`: `MAX_RIGHT_PANEL_WIDTH` を 1600 に、`rightPanelRatio` と
  `setRightPanelRatio` を追加。`openBrowserPanel` に `options.ratio`。
- `App.tsx`: `shellRow` の幅を `RightPanel` に渡す（または hook 内で解決）。
- テスト: ドラッグで 50% を超えられる / 中央 300px を割らない /
  リサイズ後も比率維持 / 他パネルは px のまま。

### P1: Chrome 風シェル（2〜3 日）

- `BrowserPanel.tsx`: タブ → ツールバー → ビューポートの順へ。
- `RightPanel.tsx`: `rightPanel === "browser"` のヘッダーを省略。
- `BrowserTabStrip.tsx`: 頭文字アバター、中クリック、安定順序、閉じる表示。
- `BrowserToolbar.tsx`: オムニボックスを主役に。ライブ / 自動表示 /
  フル表示 / 設定を ⋮ メニューへ移動。
- `styles`: ブラウザビューポートの枠・余白を削除。
- テスト: 並び順、スクリーンショット相当の DOM 構造、キーボード。

### P2: セッション（2〜3 日）

- `useLiveFrame.ts`: タブ順、`connection`、セッションスナップショット、
  自動復元、`browser_session` による再開、document.hidden の screencast 停止。
- `BrowserSessionMenu.tsx`（新規）: ⋮ メニュー本体。
- `useShellEvents.ts`: `split_ratio` の読み取り。
- `interface.rs`: `split_ratio` をツール仕様へ追加。
- テスト: 自動復元、リセット確認、`split_ratio` 適用。

### P3: 仕上げ（1〜2 日）

- `⌘W` / `⌘1..9` のブラウザ内ショートカット（任意）。
- `docs/desktop/architecture.md` のサイドパネル節を更新。
- リリースノート草案。
- 実機確認: macOS / Windows のウィンドウ幅、狭い画面、HiDPI。

---

## 5. 主な変更ファイル

| ファイル | 変更 |
| --- | --- |
| `apps/desktop/ui/src/features/shell/RightPanel.tsx` | ブラウザ時のヘッダー省略、rightMax、比率適用 |
| `apps/desktop/ui/src/features/shell/usePanelCap.ts` | `PanelBounds` / `rightMax` / `rowWidth` |
| `apps/desktop/ui/src/features/shell/ResizableHandle.tsx` | ダブルクリックリセット |
| `apps/desktop/ui/src/features/browser/BrowserPanel.tsx` | 3 段レイアウト、セッション操作、自動復元 |
| `apps/desktop/ui/src/features/browser/BrowserTabStrip.tsx` | タブ UI、頭文字、中クリック |
| `apps/desktop/ui/src/features/browser/BrowserToolbar.tsx` | オムニボックス + ⋮ |
| `apps/desktop/ui/src/features/browser/BrowserViewport.tsx` | 全画面表示、手動入力バー、ビューポート同期 |
| `apps/desktop/ui/src/features/browser/BrowserSessionMenu.tsx` | 新規 |
| `apps/desktop/ui/src/features/browser/sessionSnapshot.ts` | 新規（タブ復元） |
| `apps/desktop/ui/src/features/browser/useLiveFrame.ts` | タブ順、connection、snapshot、復帰、viewport |
| `apps/desktop/ui/src/lib/ui.ts` | `rightPanelRatio`、`openBrowserPanel` の options |
| `apps/desktop/ui/src/features/shell/useShellEvents.ts` | `split_ratio` |
| `apps/desktop/browser/src/interface.rs` | `split_ratio` をツール仕様へ |
| `apps/desktop/browser/src/operator.rs` | `set_viewport` / `session_snapshot` |
| `apps/desktop/browser/src/chromiumoxide_backend.rs` | `Emulation.setDeviceMetricsOverride` |
| `apps/desktop/app/src/commands/browser.rs` | `browser_viewport` / `browser_session` |
| `apps/desktop/ui/src/lib/api.ts` | 上記 2 コマンドの API |
| `apps/desktop/ui/src/features/browser/*.test.tsx` | 追加 |
| `docs/desktop/architecture.md` | 仕様更新 |

---

## 6. 受け入れ条件

1. ブラウザを開くと、タブ → アドレスバー → ページの順で表示される。
2. 左端をドラッグして 50% 以上に広げられ、中央カラムは 300px を下回らない。
3. ウィンドウをリサイズしてもブラウザの比率が維持される
   （他パネルは px のまま）。
4. エージェントが `split_ratio: 0.65` で `browser` を呼ぶと 65% になる。
   指定が無い呼び出しではユーザーの最後の比率が変わらない。
5. タブを 3 枚開いて 10 秒待っても並び順が変わらない。
6. タブを閉じる / 新規 / 切替がマウスとキーボードで動く。
7. パネルを閉じて開き直すと、同じタブ構成が残っている
   （プロセスが生きている場合）。再起動後は保存済みタブを自動復元する。
8. ログイン情報のリセットが右パネルからでき、確認ダイアログが出る。
9. ファビコン目的の外部リクエストが発生しない。
10. `pnpm --dir apps/desktop/ui test` / `lint` / `cargo test -p mixd-browser` が通る。

---

## 7. リスク / 未決事項

| 論点 | 現時点の案 | 決めること |
| --- | --- | --- |
| エージェントの比率指定方法 | `browser` ツールの `split_ratio`（0.2〜0.8） | 決定: ツール仕様に載せる |
| 最後のタブを閉じたとき | `about:blank` に戻す | 決定: 最後の 1 枚は閉じずにリセット |
| 再起動後のタブ復元 | 自動復元 | 決定: バックエンドが空のときだけ自動 |
| 比率の保存単位 | ワークスペース単位 | 決定: ワークスペース + グローバル既定 |
| `⌘W` | ブラウザ内フォーカス時のみ | 今回は未実装（`⌘⇧B` / `⋮` で閉じる） |
| スクリーンキャスト停止 | `document.hidden` で停止 | 決定: 非表示中は停止 |

未決のうち実装を止めるものは無い。P0 / P1 は現案で着手できる。
P2 の `split_ratio` と復元の扱いだけ、着手前に確認したい。

---

## 8. 参照

- 現行仕様: [`architecture.md`](architecture.md) の「Browser Operator」
- Chromium 導入: [`chromium-on-demand-plan.md`](chromium-on-demand-plan.md)
- アクション契約: `packages/tool_contracts/browser.json`
- 実装: `apps/desktop/ui/src/features/browser/`,
  `apps/desktop/browser/src/interface.rs`,
  `apps/desktop/browser/src/operator.rs`
