# Desktop ネイティブ子 WebView ブラウザ（実装記録）

> 状態: 実装済み（2026-11-06）
> 対象: `apps/desktop/app/src/webview_browser`, `apps/desktop/ui/src/features/browser`
> 関連: [`architecture.md`](architecture.md) / [`browser-panel-plan.md`](browser-panel-plan.md)

## 1. 結論

Desktop 版の Chrome for Testing / `chromiumoxide` / screencast を削除し、
Tauri の子 WebView（`WebviewBuilder` + `Window::add_child`）へ置き換えた。

- ページは WKWebView / WebView2 / WebKitGTK がネイティブ描画する。
- React はタブ列・オムニボックス・パネル矩形だけを持つ。
- エージェントは `eval_with_callback` で同じページを操作する。
- Docker 版の Playwright `browser_*` ツールは変更しない。

## 2. 構成

```text
Tauri Window "main"
├── main WebView (React)
│   ├── BrowserTabStrip
│   ├── BrowserToolbar
│   └── BrowserSurface（矩形だけ通知）
└── child WebView "browser-<workspace>-<tab>"
    └── ページ本体（ネイティブ入力）

Agent browser tool
   │ BrowserHost::invoke
   ▼
WebviewBrowserHost (app/src/webview_browser)
   ├── tabs / active tab
   ├── JS bridge (eval_with_callback)
   ├── screenshot adapter
   └── artifact index
```

## 3. 実装上の決定

- `tauri` の `unstable` 機能を有効化し、`Window::add_child` を使う。
- `capabilities/default.toml` は `windows = ["main"]` ではなく
  `webviews = ["main"]` にする。`windows` 指定は同一ウィンドウ内の子 WebView に
  メイン画面の IPC 権限を継承させるため、外部ページに Tauri コマンドを
  公開してしまう。
- Chrome 固有の CDP は使わない。`snapshot` は `data-mix-ref` を付与する
  DOM 走査、`click` / `type` は DOM イベントで実装する。
- macOS のスクリーンショットは `WKWebView.takeSnapshot`。Windows / Linux は
  プラットフォームアダプタの追加が必要。
- 初回リリースでは完全なリクエスト単位 egress プロキシを入れていない。
  `on_navigation` と JS の fetch/XHR/WebSocket ラッパーで private / loopback /
  metadata を遮断する。画像・CSS・iframe・Worker は対象外なので、旧 CDP
  `Fetch` ガードより弱い。必要なら `WebviewBuilder::proxy_url` とローカル
  CONNECT プロキシを追加する。

## 4. Docker 版への影響

変更しないもの:

- `apps/server/mix_agent/tools/registry.py`
- `runners/execution/main.py`
- `runners/browser_provisioner/`
- `compose.yaml`
- `apps/web/src/features/chat/BrowserPanel.tsx`
- `apps/web/src/app/browser-install-notifier.tsx`
- `tests/unit/test_browser_provisioner.py`

維持する共有契約:

- `packages/tool_contracts/browser.json` のアクション語彙
- `scripts/check-browser-parity.py`（Desktop の `interface.rs` を参照）

確認コマンド:

```sh
python3 scripts/check-browser-parity.py
pytest tests/unit/test_browser_provisioner.py
docker compose config -q
docker compose build browser-runner browser-provisioner
pnpm --dir apps/web test
```

## 5. 削除したもの

- `apps/desktop/browser/src/chromiumoxide_backend.rs`
- `apps/desktop/browser/src/installer.rs`
- `apps/desktop/browser/src/operator.rs`
- `apps/desktop/browser/src/profile.rs`
- `apps/desktop/browser/src/chromium-manifest.json`
- `apps/desktop/scripts/pin-chromium.mjs`
- `apps/desktop/app/resources/chromium`
- UI の live view / install / manual action コンポーネント
- `chromiumoxide` / installer 用依存
- `.github/workflows/desktop-release.yml` の `verify-chromium-pin`

## 6. 既知の制約

- `full_page` スクリーンショットは表示範囲のみ。
- 子 WebView の合成イベントは `isTrusted=false`。`click` / `type` は
  多くのサイトで動くが、ネイティブ入力が必要なサイトではユーザー操作を使う。
- macOS はブラウザデータがアプリ全体で共有される（macOS 14+ の
  data store identifier を使えば分離可能）。
- Windows の `add_child` は Z 順の既知問題がある。P0 実機検証で
  問題が出る場合は `WebviewWindow` 方式へのフォールバックが必要。
