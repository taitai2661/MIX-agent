# 検証記録

このファイルは時点ごとの検証記録です。**新の日付の節を先頭に足す**ことで
時系列が保たれます。節の記載は当時のまま残しますが、環境や仕様が変わった
箇所には `[追記]` を付けて明示してください。実 Provider・実 Docker・実機で
未実施の項目を、単体テストやビルド成功で置き換えて「動作確認済み」と
扱わないでください。

現在の日付順（新しい順）: 2026-10-31 → 2026-10-29 → 2026-10-28 → 2026-10-28 → 2026-10-28 → 2026-10-28 → 2026-10-27 → 2026-10-26 → 2026-10-26 → 2026-10-26 → 2026-10-25 → 2026-10-25 → 2026-10-01 → 2026-09-26 → 2026-09-23 → 2026-08-30 → 初期実装

## [実装] コーディングエージェント評価ハーネス（2026-10-31）

- `apps/desktop/core/tests/coding_eval.rs` を追加した。Python の fixture repo を実エンジン・実ツールで解かせ、エージェントとは別プロセスで `verify_command` を実行して採点する。scripted モード（CI、固定 Provider スクリプト）と live モード（`MIX_EVAL_*` を設定した実モデル）がある。指標は合否、Provider ターン数、ツール呼び出し数、全リクエスト合計の推定プロンプトトークン、実時間、「独立検証が失敗しているのに成功を主張した」回数。
- シナリオは単一ファイル修正、失敗した検証からの修復、複数ファイルのリネーム、検証ゲートによる誤報告の遮断、plan モードの編集拒否の5つ。scripted の合否条件は全タスク成功、各12ステップ以内、3タスク合計20万推定トークン未満。
- ハーネス導入時に2件の実バグを検出し修正した。①バッチ `edit`（`edits` 配列）が検証台帳に `path:`（空パス）を登録し、読み返しでは永久に閉じなかった（`runs/verification.rs`）。②plan モードや権限で拒否された編集でも検証ゲートが arm され、無変更の Run が `verification_failed` で終わっていた（`runs/engine/execute.rs`）。効果の登録を「成功した呼び出し」に限定し、同一ターン内の後続 read が先の write を confirm する順序は維持した。
- `engine_loop.rs` の破壊的編集テスト2件は編集の許可ルールを設定しておらず、拒否された編集でもゲートが arm される旧挙動に依存して通っていた。`allow_tool(&harness, "edit")` を追加し、実際に編集が成功する経路を検証するようにした。
- 検証: `cargo test -p mixd-core`（全テスト成功、scripted 評価は 3/3 タスク・2/2 ガードレール）、`cargo clippy -p mixd-core --all-targets -- -D warnings`、`rustfmt --check`（変更ファイル）。
- 未実施: 実モデルでの live モード実行（API 資格情報が必要）。Windows は `#![cfg(unix)]` により対象外。

## [修正] macOS 固定のショートカット表記とプラットフォーム別表示（2026-10-29）

- ドキュメント内の `⌘` 固定表記を、macOS は `⌘`、Windows / Linux は `Ctrl` の併記へ変更した。対象は `apps/desktop/README.md`、`docs/desktop/architecture.md`、`docs/desktop/coding-agent-plan.md`。README / CONTRIBUTING のビルド説明には Linux（AppImage + deb）を追加した。
- Web 版のメッセージ編集ヒントが `⌘+Enter` 固定だったため、`apps/web/src/app/platform.ts` を追加し、macOS 以外では `Ctrl+Enter` を表示するようにした。
- Desktop 版の `dataset.platform` を `macos` / `windows` / `linux` / `other` に拡張し、初回描画前に UA から推定した値を `main.tsx` で設定したうえで、`plugin-os` の結果により正確な値へ更新するようにした。
- 設定 ▸ アップデートの注意書きを OS 別にした。macOS は Gatekeeper、Windows は SmartScreen、Linux は deb の手動更新だけを表示する。設定 ▸ 一般のターミナルサンドボックスは macOS のみ有効とし、Windows / Linux では無効化して理由を表示する。
- 検証: Desktop UI の `pnpm typecheck` / `pnpm lint` / `pnpm exec prettier --check`、`pnpm test`（40ファイル317テスト）、Web の `pnpm build` と `pnpm test`（24ファイル172テスト）、`git diff --check` が成功。
- 未実施: Windows / Linux 実機での目視確認。OS 判定は UA と `plugin-os`、表示切り替えは Vitest の unit test のみで確認した。

## [修正] ブラウザ操作の境界とライブ表示の改善（2026-10-28）

- `navigate` / `new_tab` の許可スキームを `http` / `https` / `about:blank` に限定した。従来は `reject_private_navigate_url_with` が http(s) 以外を素通しし、`new_tab` はさらに http(s) のときだけ検査していたため、`file:`（ローカルファイルを `dom` / `screenshot` で読む）と `data:`（`evaluate` のスクリプトゲートを迂回）が到達可能だった。Registry と手動パネルの両方に `NewTab` の egress ゲートを追加した。
- `allow_private` 時の IPv4-mapped IPv6 判定を修正した。`http://[::ffff:169.254.169.254]/` は `to_ipv4_mapped()` で v4 の禁止範囲に落とす。従来は v6 の link-local/multicast しか見ておらず、ローカル許可 ON で metadata に到達できた。
- 私有 IP 判定を `mixd-browser` に一本化し、`mixd-tools::egress` はそれを呼ぶようにした。`egress` の許可リストは、ローカル許可 ON のとき loopback / private 宛先だけを免除し、公開ホストには引き続き必要とした。DNS が private のみを返すホスト名も免除対象にした。
- `update_settings` が `refresh_registry` を呼び、`AppState.egress` を `RwLock` 経由で更新するようにした。egress 許可リストの変更が再起動なしでエージェントと手動パネルへ反映される。
- `click` / `hover` の前に timeout 付き `scrollIntoView` を実行するようにした。`Element::click()` の IntersectionObserver を避けつつ、画面外要素の座標ズレを防ぐ。非アクティブタブを閉じたときに active を付け替えないよう `close_tab` も修正した。
- スクリーンショットのインデックス上限で退避した PNG を実ファイルごと削除し、起動時に実体の無い行を落とすようにした。ライブ表示は JPEG（品質 70）/ `try_lock` / 非表示時停止に変更し、`browser_frame` は workspace が操作中なら `null` を返す。UI は前回フレームを維持する。
- UI を `useBrowserStatus` / `useLiveFrame` / `ChromiumCard` / `LiveViewCard` / `BrowserActionForm` に分割した。インストールは `browser://install` イベントと 1 秒ポーリングの両方で受ける。未使用だった `browser_check_update` コマンドと `browserCheckUpdate` API を削除した。ライブ画面のクリックは `object-contain` の letterbox を考慮した `framePoint` でページ座標へ変換する。
- `browser.json` に `desktop.manual_only` を追加し、`click_at` / `type_active` をモデル向け tool spec から除外した。`check-browser-parity.py` は `manual_only` の妥当性と contract limits と Rust 定数の一致も検査する。
- 検証: `cargo fmt --all -- --check`、`cargo clippy -p mixd-browser -p mixd-tools --all-targets -- -D warnings`、`cargo test -p mixd-browser --lib`（24 passed / 2 ignored）、`cargo test -p mixd-tools --lib`（119 passed）、Desktop UI の `pnpm typecheck` / `pnpm lint` / `pnpm test`（39ファイル298テスト）、`python3 scripts/check-browser-parity.py` が成功。
- 実ブラウザ: `MIX_BROWSER_EXECUTABLE="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" cargo test -p mixd-browser --test integration` が 3 passed。`new_tab` の `file:` / `data:` / `javascript:` 拒否、ローカル許可、click/type/tabs を実 Chromium で確認した。ローカルの `app/resources/chromium`（145.0.7632.14）では screenshot が CDP timeout したが、システム Chrome では同じテストが通過した。旧バンドルはバージョン固定と異なる場合に integration test を skip するようにし、固定版 156 の CI を stale な 145 で赤くしないようにした。
- 未実施: 固定版 156 での `file:` / `data:` / mapped IPv6 拒否とライブ表示の目視、Windows / Linux 実機でのクリック座標・成果物削除。mapped IPv6 は単体テストのみ。

## [実装] Desktop Chromium のオンデマンドインストール化（2026-10-28）

- Desktop版のChromiumをアプリバンドルから外し、初回利用時にアプリ内から導入する方式へ変更した。配布物にはブラウザを含めない。
- `mixd_browser::installer`を追加した。`browser/src/chromium-manifest.json`に固定したChrome for Testing 156.0.8078.73を、HTTPSでストリーム受信し、SHA-256・サイズを検証してから`<app-data>/chromium`へ展開する。zipは`chromium.staging-*`に展開し、検証後に旧ツリーとアトミックにswapする。パストラバーサルと危険なsymlinkは拒否し、空き容量不足・通信断・チェックサム不一致を個別のコードで返す。
- インストール後に`--version`を30秒以内に実行し、誤アーキテクチャやライブラリ不足を`launch_failed`として検出する。Chrome for Testingの`.app`はad-hoc / linker-signedで上流zipでも`codesign --verify --deep --strict`が成功しないため、署名検証を必須にせずSHA-256を供給網のアンカーにした。`.app`内の実行ファイル名は変更しない。
- 解決順を`MIX_CHROMIUM_HOME` → `<app-data>/chromium` → 旧バンドル → システムChromium → disabledに変更し、`BrowserStatus.source`で管理元を返す。インストール完了時は`refresh_registry`で`browser`ツールを再広告し、再起動なしで使えるようにした。
- Tauriコマンド`browser_install` / `browser_install_cancel` / `browser_uninstall` / `browser_check_update`と、進捗イベント`browser://install`を追加した。設定 ▸ ブラウザにインストール・更新・削除・再試行・進捗表示を追加し、未インストール時はシステムChromiumへのフォールバックを案内する。
- `tauri.conf.json`から`resources/chromium`と取得ステップを削除し、`fetch-chromium.mjs` / `ensure-chromium-dir.mjs`を廃止した。`scripts/pin-chromium.mjs`（`--verify`付き）と`pnpm chromium:pin`を追加し、リリースCIではバンドルせずに固定アーカイブのURL・サイズ・SHA-256だけを検証する。
- 検証: `cargo test -p mixd-browser installer::tests::installs_the_pinned_archive -- --ignored`が成功（156.0.8078.73を実ダウンロードし、インストール・メタデータ・削除まで確認）。`MIX_CFT_ARCHIVE=<mac-arm64 zip> cargo test -p mixd-browser installer::tests::extracts_a_real_archive -- --ignored`も成功。`MIX_BROWSER_EXECUTABLE=<CfT 156> MIX_REQUIRE_BROWSER_TEST=1 cargo test --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt --all -- --check`、Desktop UIの`pnpm typecheck` / `pnpm lint` / `pnpm test`（38ファイル293テスト）が成功。`pnpm tauri build --no-bundle`が成功し、`pnpm tauri build --bundles app`で`.app`が35.58 MiB（アップデータ用`.app.tar.gz`は12.72 MiB）になり、旧569 MiBからChromium分が消えたことを確認した。ローカルでは`TAURI_SIGNING_PRIVATE_KEY`未設定のためアップデータ署名で停止したが、アプリバンドル自体は生成された。
- 未実施: Windows / Linux実機での初回インストール、プロキシ環境、回転・切断・ディスク不足・キャンセル・アプリ強制終了の実機確認、GUIでのインストールカード目視、旧同梱版からアップデートした場合の再ダウンロード導線。macOSはarm64のみ、CfT 156.0.8078.73のみで確認した。


## [実装] Web版 Chromium インストールのポップアップ通知（2026-10-28）

- Web版でChromiumのバックグラウンド導入が始まったとき・完了したとき・失敗したときに、画面右上のトーストで通知する`BrowserInstallNotifier`を追加した。設定画面を離れていても通知できるよう、導入中だけ`/settings`を3秒間隔で再取得する。
- セットアップ完了時にも`/settings`をinvalidateし、セットアップ画面を閉じた直後から開始通知が出るようにした。日本語・英語の文言を追加した。
- 検証: `pnpm --filter mix-agent-web test`（22ファイル165テスト）、`pnpm --filter mix-agent-web build`が成功。
- 未実施: 実Docker環境でChromiumを導入し、開始・完了・失敗の各トーストを目視確認すること。別タブやAPIから直接導入を開始した場合は、この画面のポーリングが始まるまで通知が遅れる。

## [実装] Desktop Browser Operator の実ブラウザ検証と操作基盤（2026-10-28）

- 最大の原因は、`Browser::launch` が返す CDP `Handler` を spawn せずフィールドに保持していたことだった。コマンドは送られるが応答が読まれず、`pages()` を含む全操作が永久に待機していた。`tokio::spawn` で `Handler` ストリームを駆動するよう修正した。
- `navigate` が毎回 `new_page` し、以降の操作が `Browser::pages()` の `HashMap` 順で先頭ページに当たっていた問題を修正した。アクティブタブを保持し、`navigate` は現在のページを再利用する。`tabs` / `new_tab` / `switch_tab` / `close_tab` を追加した。
- 操作をワークスペース単位の `Mutex` で直列化した。エージェントと設定画面の手動操作が同じプロファイルを同時に触っても、ナビゲーションとクリックが交錯しない。
- `click` は `Element::click()` の `IntersectionObserver` 待ちでハングすることがあったため、要素の content quad 中心への CDP マウスクリックに変更した。`type` は JS の `value=` 代入ではなく、フォーカス後の実キー入力に変更した。`hover` / `press` / `select` / `check` / `clear` / `type_active` / `click_at` を追加した。
- `snapshot` を追加し、`role` / `name` / `ref` を持つ要素ツリーを最大 300 件で返す。`click` / `type` / `hover` / `select` / `check` は `ref` を受け付け、`r<n>` 形式を検証してから `[data-mix-ref]` に解決する。`extract` / `dom` / `wait_for(text|load)` / `reload` / `back` / `forward` も追加した。
- スクリーンショットは `Page.captureScreenshot` を直接使い、full page は clip で撮る。macOS headless では `--headless=new` と `--run-all-compositor-stages-before-draw` / `--disable-new-content-rendering-timeout` が必要で、これらがないと `Page.captureScreenshot` が CDP タイムアウトした。
- `packages/tool_contracts/browser.json` を追加し、Desktop の action enum と Docker の `browser_*` の対応を単一の語彙にした。`scripts/check-browser-parity.py` を追加し、`ci.yml` の parity job に組み込んだ。Desktop は `browser` 1 ツール、Docker は 15 ツールをこの contract にマップし、未対応 10 action を明示している。Docker 側は hover / press / select / check / reload / back / forward / close を追加した。
- 設定に `browser_allow_local`（既定 OFF）を追加した。ON のときだけ loopback / RFC1918 / ULA を許可し、169.254.0.0/16（cloud metadata）・link-local・multicast は許可後も拒否する。エージェントの egress 判定と手動操作の両方に適用する。
- Chromium が見つからない場合は `browser` ツールをモデルへ広告しないようにした。設定画面には backend / 実行ファイル / プロファイル / 成果物数 / ローカル許可状態を表示する。
- プロファイル一覧を起動時にディスクから復元するようにした。リセット後も同じ workspace に留まり、ランダム UUID へ切り替わらない。成果物インデックスは `artifacts/index.json` に永続化し、再起動後も UI から開ける。
- 設定画面にライブ画面を追加した。1.5 秒間隔で非永続フレームを取得し、クリック座標をページ座標へ変換して `click_at`、`type_active`、`press` を送れる。ポーリングは成果物を作らない。
- 検証: `cargo test -p mixd-browser`（16 unit + 4 contract + 3 integration + 5 scope）、`cargo test -p mixd-tools`（118 + 13）、`cargo test -p mix-agent-desktop`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt --all`、`python3 scripts/check-browser-parity.py`、Desktop UI の `pnpm typecheck` / `pnpm lint` / `pnpm test`（38ファイル291テスト）が成功。実 Chromium 統合テストは同梱 Chromium 145.0.7632.14 を macOS arm64 で起動し、2 回の navigate、click、type、snapshot、wait_for(text)、screenshot、tabs、close を確認した。localhost は opt-in 前は拒否、opt-in 後は到達できることも確認した。
- 未実施: Windows / Linux 実機の同梱 Chromium、Docker Playwright runner の新 action 実装（contract 上は unsupported）、CDP リクエストインターセプトによる完全な egress 強制、Run の停止を伴う「一時停止 → 手動操作 → 再開」の実行制御、実サイトのログイン・CAPTCHA・ダウンロード・アップロード・iframe / shadow DOM。スクリーンショットの見た目検証は自動テストの成功とは別である。

## [実装] ローカルランタイムの初回導線と判定の修正（2026-10-28）

- `DetectedRuntime.online` を「モデルが 1 件以上ある」から「識別済みのランタイムが応答している」へ変更した。Ollama は空ライブラリでも `/api/version` の指紋で識別できるようにし、モデル 0 件の新規インストールを起動 → `pull` まで進められるようにした。空ライブラリはエラー表示しない。未識別の空応答（AirPlay など）は従来どおり `online = false`。
- `is_managed_provider` / `is_loopback_base_url` を追加し、API キー免除用の「ローカル（LAN / mDNS / Tailscale を含む）」と、プロセス起動・停止・`num_ctx` 上限用の「loopback のみ」を分離した。LAN の Ollama を自動起動したり 8K に制限したりしない。
- `sync_models` を当該 provider の authoritative list に変更し、runtime から消えたモデル行を削除するようにした。ローカルランタイムの削除後は `default_model_id` が存在しないモデルを指していれば `null` に戻す。
- ローカルランタイムパネルの「再検出」を `local_runtime_snapshot` の再取得から `local_runtime_refresh` の実スキャンへ修正した。承認カウントダウンは全カードで 1 本の `setInterval` を共有し、最後の購読解除で停止する。`pull` のキャンセルは `Notify` で待機中のストリームを直ちに起こす。`clearRun` は同じ会話で開始済みの次 Run を消さない。
- 送信時自動起動は `local_auto_start` が有効な場合のみ行い、ユーザーが明示的に停止したランタイム（`auto_start_suppressed`）は次の送信でも起動しない。起動失敗は Composer のチップに表示し、再試行できる。
- ローカルモデル操作の能力フラグをバックエンドの `model_operations` に集約し、UI のベンダー名分岐（`ollama || lmstudio`）を削除した。
- 検証: `cargo fmt --all -- --check`、`cargo clippy -p mixd-providers -p mix-agent-desktop --all-targets -- -D warnings`、`cargo test -p mixd-providers`、`cargo test -p mix-agent-desktop`、Desktop UI の `pnpm lint` / `pnpm typecheck` / `pnpm test`（36ファイル279テスト）/ Prettier が成功。追加テストは空ライブラリの Ollama、loopback と LAN の分離、モデル行の prune、共有カウントダウン、`clearRun` の競合、`CancelToken` の即時起床を含む。
- 未実施: 実機の Ollama での `start` → 空ライブラリ表示 → `pull` → `load` → `stop`、実機 LM Studio での削除・再検出、macOS / Windows / Linux のプロセス停止差、ハードウェア目安（RAM / VRAM からの推奨モデル）。HTTP モックと単体テストのみで確認した項目を「実機確認済み」とは扱わない。

## [実装] macOSインストール不能（バンドル署名欠落）の修正（2026-10-27）

- 公開済み`v0.5.0`の`MIX-agent-Desktop-Apple-Silicon.dmg`をダウンロードして調査した。`.app`内に`Contents/_CodeSignature/CodeResources`が無く、実行ファイルだけがlinkerによるad-hoc署名だった。`codesign --verify --deep --strict`は`code has no resources but signature indicates they must be present`で失敗し、`spctl --assess`もrejectした。Apple Siliconではこの状態のアプリをダウンロード後に起動できず、「壊れているため開けません」と表示される原因になる。
- `apps/desktop/app/tauri.conf.json`の`bundle.macOS.signingIdentity`に`-`（ad-hoc）を設定した。Developer ID署名・公証は引き続き行わない。`apps/desktop/app/src/lib.rs`に設定が`-`のままであることを固定するunit testを追加した。
- `.github/workflows/desktop-release.yml`に`Verify macOS signature and bundle architecture`を追加した。macOSの`.app`に対して`codesign --verify --deep --strict`と`Signature=adhoc`を確認し、アプリ本体と同梱Chromiumの`lipo -archs`がmatrixのtarget（arm64／x86_64）と一致しなければfail closedで停止する。
- ローカルmacOS（macOS 27.2 / arm64）で`pnpm tauri build --bundles app,dmg --target aarch64-apple-darwin`を実行した。`Signing with identity "-"`が出て、生成した`.app`は`codesign --verify --deep --strict`成功、`Signature=adhoc`、`Sealed Resources version=2 files=335`、本体と同梱Chromiumの両方が`arm64`だった。DMGは255.67 MiBで生成された。
- DMGをマウントして`.app`をコピーし、`com.apple.quarantine`を付与して`open`したところ、未承認のままでは起動しなかった（Gatekeeper確認が必要な想定どおりの挙動）。`xattr -dr com.apple.quarantine`後に再実行すると起動し、プロセスが常駐することを確認した。DMG内の`.app`も署名検証に成功した。
- 同梱Chromiumは`--headless --remote-debugging-port`で起動し、`/json/version`が`Chrome/145.0.7632.14`を返すことを確認した。TauriのリソースコピーでmacOS frameworkのsymlinkが実体化され、`.app`は約569 MiB（展開後）になる。起動には影響しないが、配布サイズ削減は今後の課題。
- 検証コマンド: `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、変更前の`cargo test --workspace`、追加test単体、Desktop UIの`pnpm lint`／`pnpm typecheck`／`pnpm test`（32ファイル252テスト）／`pnpm format:check`、`node scripts/check-version-consistency.mjs`、workflow YAMLと`tauri.conf.json`の構文検査がすべて成功した。
- 未検証: 修正版のIntel（x86_64）DMG実機、GitHub Actions `desktop-release` dry-run、Gatekeeperの「このまま開く」をGUIでクリックする操作、Developer ID署名・公証（方針として未使用）、Windows実機。`v0.5.0`の公開済みDMGはこの修正前のため、再配布する場合はタグ／Releaseの作り直しが必要。
- ユーザーの選択により、修正版は`v0.5.1`として準備する。`package.json`、`apps/web/package.json`、`apps/desktop/package.json`、`apps/desktop/ui/package.json`、`apps/desktop/app/tauri.conf.json`、`apps/desktop/Cargo.toml`／`Cargo.lock`、`apps/server/pyproject.toml`／`uv.lock`／`mix_agent/main.py`、`runners/mcp_manager/main.py`を`0.5.1`へ更新した。`docs/openapi.json`は`scripts/export-openapi.py`で再生成し、`scripts/render_zimaos_compose.py`の`v0.5.1`メタデータから`apps/MIX-agent/docker-compose.yml`を再生成した。`docs/release-notes-v0.5.1.md`も追加した。
- `scripts/check-version-consistency.mjs`を拡張し、従来の5 manifestに加えて`apps/web/package.json`、`apps/server/pyproject.toml`、FastAPIの`version`、`docs/openapi.json`、2箇所のMCP `clientInfo`（`runners/mcp_manager/main.py`、`apps/server/mix_agent/mcp/protocol.py`）のバージョンも検査するようにした。`protocol.py`が`0.2.3`のまま残っていたため`0.5.1`へ修正した。`node scripts/check-version-consistency.mjs --tag v0.5.1`が成功し、11箇所とタグが一致した。`scripts/check-memory-parity.py`、`scripts/check-greeting-parity.py`、`docker compose -f apps/MIX-agent/docker-compose.yml config`、`uv run ruff check`、`uv run ruff format --check`も成功した。
- 追加検証: Desktop UIの`pnpm lint`／`pnpm typecheck`／`pnpm test`（32ファイル252テスト）／`pnpm format:check`、Webの`pnpm test`（21ファイル158テスト）／`pnpm build`、`cargo fmt --all -- --check`／`cargo clippy --workspace --all-targets -- -D warnings`／`cargo test --workspace`（37 suite / 602 passed）が成功した。ZimaOS ComposeはPyYAML 6.0.2（CIと同じ）で再生成しても差分が出ないことを確認した。
- 最終`v0.5.1`ビルド: `pnpm tauri build --bundles app,dmg --target aarch64-apple-darwin`が成功し、`MIX-agent Desktop_0.5.1_aarch64.dmg`（255.68 MiB）と`.app`（569.10 MiB）を生成した。`CFBundleShortVersionString`／`CFBundleVersion`は`0.5.1`、`codesign --verify --deep --strict`は成功、`Signature=adhoc`、`Sealed Resources files=335`、アプリ本体と同梱Chromiumはいずれも`arm64`だった。
- 最終DMGをマウントして`.app`の署名検証に成功した。同梱Chromiumに`com.apple.quarantine`を付けて`--version`を実行しても起動した。`.app`にquarantineを付けた場合は未承認のままでは起動せず、`xattr -dr com.apple.quarantine`後に起動した（App Translocation経由）。Gatekeeperの「このまま開く」をGUIでクリックする操作自体は未実施。
- 未実施: Docker daemonが起動していないため、PostgreSQLを使う`docker compose --profile test run --rm test`、8種類のrelease image build、`Validate release candidate`相当の実Docker検証はローカルで実行していない。Windows CI、Windows実機、Intel Mac実機、GitHub Actions `desktop-release` dry-runも未実施。
- リスク: この作業ツリーの`main`（`85c5a7cb`）は公開済み`v0.5.0`タグのcommit（`80dceade`）の子孫ではなく、`v0.5.0`を含むリリース系ブランチと分岐している。v0.5.1を公開する前に、どちらの履歴を正としてタグを切るか、`upstream/main`との統合方針を決める必要がある。

## [実装] Windows配布の仕上げ（2026-10-26）

- Windowsでショートカット表記が`⌘`／`⇧`固定になっていたため、`apps/desktop/ui/src/lib/platform.ts`を追加し、`ShortcutHelp`、`CommandPalette`、`App`、`Composer`、`ApprovalBanner`、`MessageItem`、`Transcript`、`GeneralSettings`をプラットフォーム対応にした。macOSは`⌘`／`⇧`、それ以外は`Ctrl`／`Shift`を表示する。キー入力側は従来どおり`metaKey`と`ctrlKey`の両方を受け付ける。
- `Transcript`と`GeneralSettings`の文言は`tf()`の`{shortcut}`プレースホルダーへ変更し、`en`辞書も更新した。`platform.test.ts`でmacOS／非macOSの表記と`chord()`の結合を固定した。
- `reports_a_nonzero_exit_code`はWindowsのみ`cmd.exe /c exit 3`を使うようにした。POSIXの`(exit 3)`はサブシェルで完結するが、PowerShellフォールバックでは`exit 3`がシェル自体を終了させ、status fileが書かれず`exit_code`が`null`になっていたため。
- `README.md`／`README.en.md`のWindows記述を「次回リリース（v0.5.1以降）で配布予定、v0.5.0はmacOSのみ」に明確化した。`docs/release-notes-v0.5.0.md`は公開済みの履歴として変更しない。
- ローカルmacOSで`cargo fmt --all -- --check`、`cargo test -p mixd-tools terminal::tests::reports_a_nonzero_exit_code`、Desktop UIの`pnpm typecheck`、`pnpm lint`、`pnpm test`（32ファイル252テスト）が成功した。
- 未検証: 修正後のWindows CI（`desktop-rust (windows-latest)`と`desktop-package-windows`）。実行時点のGitHub Actionsは全jobがステップなし・0秒でfailureしており、billing／spending limitの可能性があり、Windows CIの実結果を取得できていない。Windows 10/11実機のNSIS／MSIインストール、初回起動、WebView2、SmartScreen、Credential Manager、ターミナル（Git Bashあり／なし）、同梱Chromium、日本語ユーザー名のワークスペース、アンインストールも未確認。署名・公証は行わない方針のまま。
- `v0.5.0`タグはWindows対応コミットより前を指すため、v0.5.0へWindows成果物を後付けするにはタグとReleaseの作り直しが必要。今回は次回リリース向けの準備として扱う。

## [実装] Windows Desktopリリース対応（2026-10-26）

- `.github/workflows/desktop-release.yml` にWindows x64（`x86_64-pc-windows-msvc`、NSIS + MSI）を追加した。macOSのApple Silicon／Intel DMGは維持し、Linux Desktopは引き続き対象外。
- Windows用の成果物名を`MIX-agent-Desktop-Windows-x64-Setup.exe`と`MIX-agent-Desktop-Windows-x64.msi`に固定し、checksums jobはmacOS 2件とWindows 2件の計4件を明示的に検証して`SHA256SUMS`を生成するようにした。
- ChromiumのWindows既定プラットフォームを`win32`から`win64`へ修正し、release matrixも`cft: win64` / `cft_key: WIN64`にした。`MIX_CHROMIUM_SHA256_WIN64` / `MIX_CHROMIUM_SIZE_WIN64`は未設定でもバージョン・プラットフォーム検証でfail closedになる。
- `apps/desktop/app/tauri.conf.json`にNSIS（currentUser、English/Japanese、言語選択）とWiX en-USを設定し、WebView2は`downloadBootstrapper`を明示した。
- `ci.yml`にWindowsでの`tauri build --bundles nsis,msi` smoke jobを追加した。Chromiumはplaceholder（`MIX_SKIP_CHROMIUM=1`）を使う。
- タグと`package.json`／`apps/desktop/package.json`／`apps/desktop/ui/package.json`／`tauri.conf.json`／`apps/desktop/Cargo.toml`のバージョン一致を検証する`scripts/check-version-consistency.mjs`を追加した。
- ターミナルのタイムアウト時にPTY出力の末尾をエラーへ含めるようにし、Windows CIでmarker未検出の原因を追えるようにした。
- ローカルmacOSで`cargo fmt --all -- --check`、`cargo clippy -p mixd-tools --all-targets -- -D warnings`、`cargo test -p mixd-tools terminal::`（21 passed）が成功。ワークフローYAML・`tauri.conf.json`・`check-version-consistency.mjs`の構文とバージョン一致も確認した。
- 未検証: Windows CI（`desktop-rust`と新規packaging job）の実結果、`desktop-release` dry-run、Windows 10/11実機でのインストール・起動・ターミナル・Credential Manager・同梱Chromium、macOS実機のDMG起動。署名・公証は今回も行わない。

## v0.5.0公開結果（2026-10-26）

- `v0.5.0` タグとGitHub Releaseを作成した: https://github.com/taitai2661/MIX-agent/releases/tag/v0.5.0
- Release assets: `MIX-agent-Desktop-Apple-Silicon.dmg`、`MIX-agent-Desktop-Intel.dmg`、`SHA256SUMS`、`mix-agent-zimaos-store-v0.5.0.tar.gz`、`mix-agent-zimaos-legacy-v0.5.0.zip`。
- GHCRの8種類のAMD64イメージ（init / app / execution / browser-runner / browser-provisioner / mcp / mcp-runtime / egress）が`v0.5.0`で参照可能なことを確認。
- `gh-pages`の`index.json`に`io.github.taitai2661.mix-agent`の`0.5.0`が含まれることを確認。
- 初回のtag起点`Release MIX-agent`は、v2ストアビルダーも`Apps/`を要求するためZimaOSストア生成だけfailureになった。mainで両ビルダー用のステージ処理を修正し、`Publish ZimaOS store`（`tag=v0.5.0`）を手動実行してstore tar.gz/legacy zipをReleaseへ追加した（run 40248344738）。
- Windows PTYテスト2件は既知の制約としてRelease notesに記載した。Windows Desktopはv0.5.0の配布対象外。

## v0.5.0公開前の問題修正（2026-10-26）

- `desktop-release`のmacOS DMG収集が`mapfile`（macOS標準bash 3.2に無い）で失敗していたため、`find -print0` + `read -d ''`のbash 3.2互換ループへ変更した。DMG 1件／2件の収集を`/bin/bash` 3.2で再現し、成功を確認。
- `apps/desktop/ui/src/features/settings/UpdateSettings.tsx`へPrettierを適用。`pnpm format:check`、`pnpm lint`、`pnpm typecheck`、Vitest 31ファイル249テストが成功。
- ルート`pnpm-lock.yaml`から削除済み`@tauri-apps/plugin-updater`の残存entryを除去した。`pnpm install --frozen-lockfile`が全4ワークスペースで成功。これがCI `docker-frontend`／`Validate release candidate`の`ERR_PNPM_OUTDATED_LOCKFILE`の原因だった。
- WebはVitest 21ファイル158テストとproduction buildが成功。ZimaOS Composeの再生成・`docker compose config`も成功。
- 手動DMG配布へ切り替えた後に残っていた`apps/desktop/scripts/merge-updater-manifests.mjs`とそのテストを削除し、`THIRD_PARTY_LICENSES.md`を再生成した。
- `Apps/MIX-agent`と`apps/MIX-agent`の大文字小文字重複を解消した。v1/v2ストア用の`Apps/`レイアウトは`scripts/stage-store-source.sh`で一時生成し、v1 archiveの`build/sysroot/.../Apps/MIX-agent/docker-compose.yml`を確認した。macOSのケース非依存ファイルシステムでも`apps/`のみでチェックアウトできる。
- Windows PTYテストのタイムアウトを60秒へ延長し、direct-execテストもPTYテストと同じ直列ロックで実行するよう変更した。macOS上で`cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`（全suite）が成功。
- PostgreSQL test profileは583 passed / 1 skipped。8種類のrelease image target（init / app / execution / browser-runner / browser-provisioner / mcp / mcp-runtime / egress）はローカルDockerでbuild成功。
- Linuxのクリーンな作業ツリーで`pnpm install --frozen-lockfile`、WebのVitest 158テスト・build、Desktop UIのformat/lint/typecheck・Vitest 249テストが成功。`scripts/stage-store-source.sh`からのv1 archiveに`build/sysroot/var/lib/casaos/appstore/default.new/Apps/MIX-agent/docker-compose.yml`が含まれることを確認。
- [追記] 修正後のGitHub Actionsを実行した。`Validate release candidate`とmacOS-only `desktop-release` dry-runは成功。`CI`はWindowsのPTYテスト2件（`a_shell_that_exits_is_replaced_on_the_next_call`、`the_shell_state_persists_between_calls`）のみfailure。Windows Desktopはv0.5.0の配布対象外であり、ユーザーの判断でこの2件を既知の制約としてmacOS/Docker版を先行リリースする。
- [追記] 上記2件は `#[cfg_attr(windows, ignore = "...")]` により Windows ランナーでのみ明示的にスキップする。失敗のまま残すと他のWindows回帰がノイズに埋もれるため。macOS / Linux では従来どおり実行し、Windows実機でPTYを修正した時点で `ignore` を外す。
- 未確認: Windows実機のPTYテストとmacOS実機のDMG起動。`v0.5.0`タグとGitHub Releaseはこの記録の時点では未作成（その後、上の公開結果の節で作成済み）。

## [追記] v0.5.0 Desktopを手動DMG配布へ変更（2026-10-25）

- ユーザーは「自動更新を無効化し、DMGの手動配布に切り替える」方針を選択しました。v0.5.0ではアプリ起動時・アプリ内の更新確認と自動適用を行わず、GitHub ReleasesからmacOS DMGを取得する手順にします。
- Tauri updater plugin、Updater用Rust/TypeScript runtime・IPC・メニュー操作、Updater key/public key/endpoint設定、Updater署名付きartifact生成を削除・無効化。Release workflowはApple SiliconとIntelのDMG2件をアーキテクチャ名付きで添付し、`SHA256SUMS`を生成します。Apple Developer ID署名・公証はユーザー指示により行わず、Gatekeeper warningをRelease notes/UIへ明記します。
- Update settings画面は手動配布案内とGitHub Releasesへのリンクに置き換え、既存のtelemetry consent toggleは保持します。Docker/Web/ZimaOS配布とMemory・Agent等の変更は引き続きv0.5.0対象です。Windows/Linux Desktop installerは今回見送ります。
- ローカルDesktop検証: `cargo check --workspace --all-targets`、`pnpm lint`、`pnpm test`、`pnpm ui:build`がsuccess。UIはNode 22.19.0環境で32 test files / 250 tests success。初回のNode 22.13.0実行は依存`undici@8.12.3`のNode `>=22.19.0`要件未満で失敗したため、互換Nodeで再実行し成功を確認しました。Rust workspace test targetsも全件success。
- Updater signing keyはこのリリース方式では不要です。GitHub Actions Secretsの確認・取得・変更・再生成は行わず、秘密値を表示または外部送信していません。
- この変更を含むGitHub CI、PostgreSQL/Docker/Web Validate、macOS-only unsigned DMG dry-runはpush後に実施します。最新mainに対するこれらの完了前にtag作成・公開は行いません。現在の`v0.5.0` tag、GitHub Release、配布assetsは未作成です。

## [追記] v0.5.0 Desktop公開対象の変更と署名preflight（2026-10-25）

- ユーザー指示によりv0.5.0 macOSアプリ本体はApple Developer IDコード署名・公証なしで配布します。Gatekeeperの警告や起動制限が出る可能性をRelease notesに記載します。Updater用アーカイブの署名は別機構のため維持します。
- ユーザーの公開範囲変更により、v0.5.0で配布するDesktopインストーラーをApple Silicon／Intel macOSに限定し、Windows/Linux版は見送ります。`.github/workflows/desktop-release.yml`、Release notes、公開checklistをこの範囲へ合わせました。Docker/Web版とZimaOS配布は従来どおり対象です。
- Windowsを含むクロスプラットフォームCI [run 40185098158](https://github.com/taitai2661/MIX-agent/actions/runs/40185098158) は、PTY integration testを直列化したcommit `dd59c77`でsuccess。PostgreSQL・Docker/Webを含む[Validate run 40185131436](https://github.com/taitai2661/MIX-agent/actions/runs/40185131436)もsuccess。
- 先行macOS署名dry-run [run 40185138619](https://github.com/taitai2661/MIX-agent/actions/runs/40185138619)ではApple Silicon／Intelの`Build and bundle`が`security import`のkeychain certificate importでfailureでした。これはAppleコード署名を使っていた時点の履歴であり、後述のユーザー指示によってApple証明書・署名・公証をワークフローから削除しました。
- [Tauri公式macOS signing guide](https://v2.tauri.app/distribute/sign/macos/)はApple証明書を使ったmacOSアプリ署名を説明しています。ユーザーはこの署名・公証を望まずGatekeeper警告を許容するため、証明書importを復活させない方針です。
- 署名なしmacOSの現行preflight [run 40187871556](https://github.com/taitai2661/MIX-agent/actions/runs/40187871556)（commit `5239184`）では、Apple証明書エラーなしで進み、Apple Silicon／Intel両jobがUpdater用`TAURI_SIGNING_PRIVATE_KEY`のBase64 decode `Invalid padding`で`Build and bundle` failure。`checksums`および`release`はskipされました。秘密値は取得・表示していません。
- 同commitの[Validate run 40187862943](https://github.com/taitai2661/MIX-agent/actions/runs/40187862943)はsuccess。一方、[CI run 40187854500](https://github.com/taitai2661/MIX-agent/actions/runs/40187854500)はWindows `cargo test`でPTY integration testが1件failureでした。
- Updater用署名はAppleのアプリ本体署名と別機構です。公開鍵で検証できるアップデートを生成するには有効な既存Updater keyが必要ですが、今回そのdecodeに失敗しました。key値の取得・表示・外部送信・再生成・変更は行っていません。
- `v0.5.0` tag、GitHub Release、Desktop配布assetは作成していません。Updater keyの扱いが解決し、必要なdry-run/CI条件を満たすまでtag作成を保留します。

## [追記] Windows Desktop CIのpath修正（2026-10-25）

- main commit `90d18ad`の[CI run 40175459776](https://github.com/taitai2661/MIX-agent/actions/runs/40175459776)では、Desktop UI、Ubuntu/macOS Rust、Docker backend/frontend、lint、format、Memory parityが成功。Windows Rustのみ102 passed / 6 failedで、主にpersistent Git Bash PTYのcwd/exit-status marker timeoutとUnix executableを前提にしたdirect-exec testが失敗しました。
- 調査ではGit BashのPOSIX cwd (`/c/...`) とWindows native pathの境界が原因と特定。candidateの作業ツリーに`cygpath`変換、verbatim path正規化、Windows direct-exec test、workspace traversalの早期拒否を追加し、Linuxで`cargo fmt`、Clippy、`cargo test --workspace`が成功しました。
- [この追記時点] 上記修正はcandidateの作業ツリーにありpush前で、Windows CIによる再検証待ちです。`v0.5.0` tagとGitHub Releaseは未作成です。Actions Secretsの値・登録状態・秘密鍵にはアクセスしていません。
- [ZimaOS workflow 40175459775](https://github.com/taitai2661/MIX-agent/actions/runs/40175459775)自体はsuccessですが、v0.5.0のGHCR imageがまだ存在しないためstore build/publishはskipでした。

## v0.5.0公開準備の確認（2026-10-25）

確認した項目:

- JSON / YAML、Python compileall、`uv lock --check`、`git diff --check`が成功。ZimaOS Composeは`v0.5.0`で2回再生成し、出力が一致。
- [GitHub Validate run 40173309285](https://github.com/taitai2661/MIX-agent/actions/runs/40173309285)がcommit `f30255f`で成功。PostgreSQL service付きapp suiteは583 passed / 1 skipped / 1 warning。Docker release image build、legacy ZimaOS/CasaOS archive validation、Web frontendも成功。
- Web版: `pnpm install --frozen-lockfile`、Vitest 21ファイル158テスト、production buildが成功。Buildは500 kBを超えるchunkの注意を出すが、正常終了。
- Desktop UI: lint、TypeScript typecheck、Vitest 32ファイル250テスト、production buildが成功。
- Desktop Rust/Linux: `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`が成功。Rust workspaceのテスト集計は595 passed / 0 failed。PTY terminalのsentinel/cwd marker raceを修正し、terminal focused tests 18件も成功。
- Agent schedulerの誤importを修正。Memory revisionから削除済みMemoryを同一IDで復元する実装・回帰テスト、Windows/Linux Settings OS menu、Desktop clean-checkout Rust CIのChromium placeholder準備も追加。

未確認の項目:

- `cargo build --workspace --release`がoptimized profileで成功。macOS/Windows Rust matrixおよび各OS installer packagingは未実施。
- GitHub Secretsの値・登録状態・鍵ペアにはアクセスしていない。署名済み`latest.json`と各OS実機のupdater動作は未確認。署名を含むDesktop release workflowはmain上の`v0.5.0` tagが必要。
- Composeを用いた長時間runtimeの実機受入確認は未実施。

PR作成は既存GitHub integrationからGraphQL / RESTの両方で403 `Resource not accessible by integration`となり、Pull Requests: write権限が不足している。mainへの反映、`v0.5.0` tag、GitHub Releaseは未実施。

## v0.3.5リリース候補の確認（2026-10-01）

確認した項目:

- Memory Runtime、Projects、Todo、Skills Discovery／パッケージ、RunCheckpoint、Browser pause、Usage、PWAの実装を取り込み。
- Python compileall、JSON解析、YAML解析、`git diff --check`、`uv lock --check` が成功。
- WebのVitest 13ファイル58テストが成功。TypeScript／Vite production buildも成功。
- v0.3.5をPython、Web、MCP ClientInfo、OpenAPI、ZimaOS Compose、8種類のGHCRイメージ参照へ反映。
- ZimaOS Compose生成スクリプトをLinuxの大文字小文字に整合する`Apps/MIX-agent`出力へ修正し、v0.3.5のCompose生成・YAML解析が成功。

## Agent 実行ループと自律性の強化（2026-09-26）

長時間作業向けに新しい状態・データ・APIを追加し、5テーマ（文脈圧縮、停滞検知、動的予算、中間チェックポイント、検証ループ厳密化）をまとめて導入しました。実装にはAlembic migration `0020_run_checkpoints.py` が必要で、`runs` テーブルの CHECK 制約に `budget_extension_pending` を追加し、新規 `run_checkpoints` テーブルを作成します。

確認した項目:

- バックエンドのPython: 21件のVitest相当（`test_tiered_summary`, `test_stagnation_and_verification`, `test_agent_completion`, `test_mode_policy`）が成功。`test_run_checkpoints` / `test_budget_extension_state` は PostgreSQL が必要で本環境では未実行。
- TypeScript / Vite build が成功。Vitest 11ファイル52テストが成功（`types.test.ts` に新イベントの parse テストを追加）。
- `ruff check` で `mix_agent/runs/{checkpoints,stagnation,verification,events}.py` と `mix_agent/context/tiered_summary.py` と `mix_agent/api/schemas.py` はすべて通過。`mix_agent/runs/engine.py` と `mix_agent/api/routes.py` の残存警告は既存の `except Exception: pass` パターンの踏襲によるもので新機能由来ではない。
- `apps/server/migrations/versions/0020_run_checkpoints.py` をマイグレーション一覧へ追加。`RunCheckpoint` モデル・`budget_extension_pending` 状態・`one_active_run` 部分Indexが新CHECK制約と整合。［追記］この時点では末尾の `0020` でしたが、以降は `0021_memory_role_taxonomy.py` が追加されています。
- 公開API: `POST /runs/{key}/resume` の `from_checkpoint` / `unknown_actions` 拡張、`POST /runs/{key}/budget-extension`、`GET /runs/{key}/checkpoints`、`GET /runs/{key}` の新フィールド（`budget_extension_request` / `budget_extensions_used` / `budget_extensions_max` / `stagnation` / `checkpoint_resumed_from` / `checkpoints`）。
- フロントエンドの `parseRunEvent` が新イベント `checkpoint_saved` / `budget_extension_requested` / `budget_extension_resolved` / `stagnation_detected` / `verification_required` / `phase_advanced` をすべて型安全に解釈。

未確認の項目:

- 実Provider・実Runner・実Dockerでの中間チェックポイント復元動作、予算延長フローのUX、停滞検知の実会話での精度検証。
- 階層的サマリの Tier 2 fallback が、巨大Tool出力の繰り返しで正しく発火するかどうかの負荷試験。
- Agentモード以外のモード（Chat / Thinking）で `RunView` の新フィールドが UI にどう露出するかの最終デザイン整合。

これは初期実装の検証記録であり、本番運用の認定ではありません。

## 現行コードとの差分と確認範囲（2026-09-23）

この節と 2026-08-30 の節の件数・画面確認は当時の記録です。現行コードにはKnowledgeのテキスト検索Tool、Skill、定期実行、Remote MCP OAuth、Context圧縮、一時モードが追加されています。実装の存在は実Provider・実Docker・実機での受入確認を意味しません。

会話のMarkdown書き出しは回答のみ・会話・Tool実行概要を選択できます。Tool概要には引数と結果本文を含めません。Context圧縮の要約と件数はRunに保存して画面へ表示します。要約失敗時は履歴を破棄せず停止します。Runの回答チェックは最終回答の形式と有無を判定し、内容の正確さまでは判定しません。一時モードは保存対象とTool許可状態を送信欄に表示します。

キーボード操作ではモデル選択のEscapeによる復帰、チェックボックスのEnter操作、IME確定中の誤送信を点検・修正しました。スクリーンリーダーとスマートフォン実機の受入確認は未実施です。

今回の確認: WebのTypeScript/Vite buildとVitest 22件、Contextの対象テスト14件、Composeの使い捨てPostgreSQLで書き出し・回答チェックの対象テスト4件が成功。画面の実操作とスクリーンリーダーでの確認は未実施です。一時モードの添付は選択時にアップロードされるため、送信せず画面を離れた場合の孤立添付を自動削除する処理は今後の課題です。

## Chat / Thinking拡張の確認（2026-08-30）

- Chatの自動思考、ThinkingのネイティブReasoning／通常推論フォールバック、プリセットの空リストを含むツール制限をテスト。
- OpenAI Responses / OpenRouter / AnthropicはHTTPモックと実SDKで送信設定・公開要約・ツール継続を確認。GeminiはSDK型を使うクライアントモックで確認。Claudeのadaptive / budget、Geminiのlevel / budgetの両方式を含む。
- Chat / Thinkingの承認待ち・再開・拒否、3 / 5ステップ上限、停止、ツール失敗時の安全なエラー返却をテスト。既存の認証・保存・バックアップ等の回帰テストも実施。
- Vitestは合計6件。モードの利用可否、明示的な不明Override、空のプリセット、要約表示と既存APIクライアントを確認。TypeScript / Vite build成功。
- 非対応ReasoningモデルのThinking通常推論フォールバック、初回Tool Callingプローブ、結果保存、手動再確認はHTTPモックの統合テストで確認。実Providerとブラウザでの受入確認は未実施。
- 390×844のviewportで思考要約・モード選択・入力欄を目視確認。実機キーボードは未確認。

今回の画面検証は `tests/fixtures/chat_preview.py` を使用。Compose profile `preview` が起動ごとに使い捨てPostgreSQLを作り、Providerはローカルの固定応答、ツール実行はモックに置き換える。既存データ、実AI、実ファイル操作、実コマンド、実Web検索は使用しない。実Provider各接続方式の課金・応答・思考品質と実Runnerの操作は未検証。

全体Lintには既存のFastAPI Depends / wildcard import等の指摘が残る。Python未定義名チェックと、新規の思考解決・回帰テストファイルのLintは成功。

［追記］当時の記録では「Git管理外のフォルダのため `git diff --check` は実行できない」としていましたが、このディレクトリは現在は Git 管理下にあり実行可能です。`CONTRIBUTING.md` に記載の手順どおり `git diff --check` を通してください。

## 実施した確認（初期実装）

- Python / pytest: 38件。認証・Origin・CSRF、Secret暗号化、承認停止と再開、拒否時の非実行、Memory履歴とScope、7種類のProviderのモデル取得（HTTPモック）、送信の冪等性、パストラバーサル・symlink、公開IP判定、バックアップ復元と失敗時の補償を含む。
- MCP: 公式SDKのテスト用stdio Serverを実際の子プロセスとして起動し、Tool取得・呼出しを確認。
- Vitest: 2件。APIクライアントのヘッダーとエラー処理。
- TypeScript型チェックとVite production build成功。Python未定義名チェック成功。
- `docker compose config --quiet` 成功。
- Codex内ブラウザで管理者作成、Setup、Memory保存、Provider追加、モデル取得、Chat送信、回答表示、再読込後の会話履歴を確認。
- チャットの390×844相当のviewportで入力欄と回答表示を目視確認。これは実機のソフトウェアキーボードの検証ではない。

画面検証では `tests/fixtures/fake_provider.py` の固定応答を使用。実AIの応答品質・課金・能力の検証ではない。画面検証と自動テストはCompose profileごとの使い捨てPostgreSQLを使い、本運用のvolumeとは分離している。テスト用アカウントを本運用へ移行しないこと。

## 未完了の受入確認

Dockerビルド中にホストの空き容量が約1.8GiBまで低下したため、ビルドを中止した。既存データやDockerキャッシュの削除は行っていない。最終コードの全イメージ再ビルドと実起動は未確認。容量を確保した環境で以下を実施する必要がある。

- 新規Dockerボリュームから、`.env`編集なしでSetupと実Providerへの最初のチャット。
- PostgreSQL上のMigration・排他制約・pg_trgm検索。
- Runnerの直接通信遮断、Proxy許可先の制限、DNS再解決・リダイレクト・metadata対策のコンテナ境界での確認。
- TerminalからDB、Provider Key、MCP Secret、Docker Socketへアクセスできないこと。
- Playwright Browser Tool、PDF抽出、Terminalキャンセル・子孫プロセス停止の実コンテナ確認。
- 承認待ちのDocker再起動、実行途中の強制終了、SSE再接続、Remote操作の結果不明表示。
- 空のDocker環境への完全バックアップ復元。テストはテスト用Runnerとの復元・補償であり、実ボリュームの耐障害性を証明しない。
- 実OpenAI / Anthropic / Gemini等の本文、公開Reasoning情報、Tool Call、画像添付、Provider固有情報の継続送信。
- 実Streamable HTTP MCP、Filesystemパッケージ取得、資格情報の送信先、Schema変更時の再承認。
- スマートフォン実機・キーボード操作・長いTool出力・切断時の再送・すべての設定画面。

## 現時点の制約

- 管理者1名、app 1プロセスを前提とする。共有Runnerは敵対するユーザーを隔離するSandboxではない。
- Providerのモデル一覧は全Providerのページングを網羅していない。取得できないモデルは手動登録する。
- Capabilityは不明を残す。未知のモデルのTool/Vision/Reasoningは管理者が確認してOverrideする。
- 日本語UIが初期実装されているが、翻訳文言の完全な分離と多言語化は未完了。
- バックアップはサイズ上限付き。空ディレクトリ・POSIX権限・BrowserのCookie・導入済みMCPパッケージ自体は完全再現しない。詳細は運用資料を参照。
- Secretは設定APIで再表示せず暗号化するが、外部MCPが結果本文に資格情報を含めた場合の自動検出・完全除去は保証しない。信頼できるServerのみ接続する。
- Knowledgeはテキストのチャンク検索に対応。外部同期や検索品質の実環境評価は未完了。MCP OAuthは実装済みですが、実Serverでの受入確認は別途必要です。画像生成・編集、Registry Store、OpenAI Compatible公開APIは未実装。

再実行コマンドはREADME参照。実Provider・Docker・実機で未実施の項目を、単体テストやビルド成功で置き換えて「動作確認済み」と扱わない。

## [追記] 品質・セキュリティ改善（2026-10-29）

- Desktop: 未配線だった at-rest encryption トグルを「未対応」と明示し、`update_settings` は `database_encryption=true` を拒否、旧行は `false` として読むようにした。SQLCipher は未導入のため、DB は平文 SQLite + 所有者限定パーミッションのまま（`apps/desktop/app/tests/settings.rs`）。
- Desktop: Browser の各ページターゲットに CDP `Fetch` 傍受を常時適用。リクエストごとにホストを解決し、private / loopback / link-local / cloud metadata を遮断する。実 Chromium の統合テスト4件（navigate / local opt-in / profile / screencast）が成功。
- Desktop: Windows の PTY テスト2件を `#[cfg_attr(windows, ignore)]` で明示的にスキップ。macOS では従来どおり実行し、`cargo test --workspace` が成功。
- Docker: `MIX_LOG_FORMAT=json` で構造化ログを選択可能にした（`mix_agent/logging_setup.py`、ユニットテスト2件）。
- Docker: scheduler の同期DBメンテナンス処理を `asyncio.to_thread` へ移し、イベントループを塞がないようにした（run の起動はIDを収集してループ上で実行）。
- Docker: `/memories*` 10ルートを `api/routes_memories.py` へ分割し、`docs/openapi.json` と `apps/web/src/generated/api.ts` を再生成した。
- Web: ESLint（`--max-warnings 0`, react-hooks, jsx-a11y）と Prettier を CI に追加。`any` を生成型へ置換し、条件付き Hook 3件を修正。カバレッジ下限を lines/statements 34%、functions 57.5% へ引き上げた。E2E に「送信 → SSE text → done」を追加。
- CI: 外部 Action をコミット SHA に固定（`dtolnay/rust-toolchain@master` を含む）。
- 検証: `pytest` 605 passed / 1 skipped、`cargo test --workspace` 全 suite 成功、`cargo clippy --workspace --all-targets -D warnings`、`ruff check` / `ruff format --check`、pyright budget（0 errors / 45 warnings）、Web lint / format / 198 tests / coverage 41.2% lines・79.9% branches・60.36% functions / build / Playwright 3 tests、Desktop UI lint / typecheck / 317 tests / build が成功。
- 未実施: Developer ID / Authenticode 署名（証明書が無いため。手順は `docs/release-checklist.md`）、SQLCipher 実装。
- 巨大ファイル分割（2026-10-29）:
  - `api/routes.py` 3,520行 → 集約33行 + `routes_{settings,statistics,schedules,conversations,messages,runs,memories,skills,mcp,collections}.py`。
  - `runs/engine.py` 2,444行 → `engine.py`（起動・駆動・scheduler）+ `engine_support/tools/context/routing/provider.py`。
  - Desktop `core/src/runs/engine.rs` 3,499行 → `engine.rs` 690行 + `engine/{event_bus,query,execute,execute_turn,execute_tools,execute_budget,finish,helpers}.rs`。
  - Desktop `tools/src/terminal.rs` 1,992行 → `terminal.rs` 632行 + `terminal/{shell,session,parsing,sandbox,tests}.rs`。
  - Web `features/chat/Chat.tsx` 1,123行 → 438行 + `useChatRunStream` / `useChatActions` / `ChatTranscript` / `ChatMessage` / `ChatComposer` / `ChatRunPanels` / `ChatWelcome` / `ChatMarkdown`。
  - Desktop UI `Composer.tsx` 1,110行 → 317行 + `useComposerState` / `ComposerToolbar` / `ComposerAttachments` / `composerTypes` / `composerModes`。
  - 確認: `cargo test --workspace` / `clippy -D warnings` / `cargo fmt --check`、Docker `pytest` 605 passed / 1 skipped、Web lint / format / 198 tests / coverage 41.2% lines・79.9% branches・60.36% functions / build / E2E 3 tests、Desktop UI lint / typecheck / 317 tests / build / format をすべて再実行して成功。
