# Desktop版 Chromium の初回インストール方式（設計記録）

> 状態: 実装済み（2026-10-28）。
> 現行の仕様（インストール先・解決順・固定マニフェスト・検証手順）は
> [`architecture.md`](architecture.md) の「Managed Chromium」を正とする。
> 実機検証の結果と未実施項目は [`../verification.md`](../verification.md) に記録している。
> この文書は、同梱をやめた理由・代替案の比較・移行方針だけを残す。

## 1. 決定

Desktop 版の Chromium を**アプリバンドルから外し、初回利用時にアプリ内からインストールする**。

- 配布物（DMG / NSIS / MSI / AppImage / deb）にブラウザ本体を含めない
- ユーザーがブラウザ機能を初めて使うときに、設定画面またはブラウザペインから明示的に導入する
- 保存先はアプリデータ配下（`<app-data>/chromium`）。既存の `find_downloaded_chromium()` が想定していたレイアウトをそのまま使う
- 未導入の場合はシステムの Chromium / Chrome / Edge へフォールバックする（設定で無効化可能）
- ダウンロード元は Chrome for Testing。バージョンと SHA-256 はアプリのリリース時に固定し、実行時に「最新版を無検証で取る」ことはしない

「インストーラー形式」は OS のパッケージマネージャー（winget / brew / apt）に任せず、
**アプリ自身が管理するランタイムコンポーネント**として導入する意味に解釈している。
CDP 互換性と再現性のため、OS 側の Chrome 更新に追従させるよりアプリ側でバージョンを固定する方が安全だからである。

## 2. 背景（同梱時の問題）

- 配布物が大きい。実測で DMG 255.67 MiB、展開後の `.app` 約 569 MiB、うち `app/resources/chromium` が 330 MB
- Chromium を使わないユーザーにも 330 MB を配っていた
- アプリ更新のたびに Chromium も丸ごと再配布される
- macOS の framework symlink を Tauri のリソースコピーが実体化し、展開後サイズがさらに膨らむ
- 同梱 Chromium の CPU アーキテクチャ検査など、リリース CI の責務が増えていた

## 3. 代替案の比較

| 方式 | 配布サイズ | オフライン | 再現性 | 実装コスト | 評価 |
| --- | --- | --- | --- | --- | --- |
| 同梱（変更前） | 大（255 MiB DMG） | ○ | ○（CI で固定） | 低（維持） | 配布サイズが課題 |
| **初回ダウンロード（採用）** | 小（数十 MiB） | △（初回のみ要ネット） | ○（SHA-256 固定） | 中 | サイズと再現性のバランスが良い |
| システム Chromium のみ | 最小 | ○（導入済みなら） | ×（ユーザー環境依存） | 低 | CDP 互換性・版差が読めない |
| OS パッケージマネージャー | 小 | ○ | △ | 中〜高 | Linux 以外で標準化しづらい |
| オフライン同梱版を別提供 | 大 | ○ | ○ | 中 | 企業向けの補助としてなら有り |

初回ダウンロードを基本にし、システムブラウザへのフォールバックを残す。
オフライン / 企業向けには「事前配置ディレクトリ」を用意する。

## 4. 決定事項

着手時に未決だった論点は、次のとおり決定して実装した。

| 論点 | 決定 | 理由 |
| --- | --- | --- |
| インストール開始のトリガー | 明示ボタンのみ | 数百 MB の通信を同意なく発生させない。テザリング・従量制・プロキシ環境での失敗を避ける |
| システムブラウザのフォールバック | 許可（設定で無効化可能） | 初回にネットワークが無い環境でもブラウザ機能を使える |
| バージョン戦略 | アプリのリリース時に固定 | CDP 互換性と再現性を優先。実行時に latest を解決しない |
| オフライン / 企業向け | `<app-data>/chromium` への手動配置 + 手順の案内 | 別インストーラーの配布・保守を増やさない |
| 同梱コードパス | 移行期間のフォールバックとして残す | 旧版からアップデートしたユーザーが再ダウンロードなしで動ける |
| アンインストール時の Chromium | 残す | プロファイルや成果物と同じ扱い。再インストール時の再取得を避ける |
| Linux arm64 | システムフォールバックのみ | Chrome for Testing が `linux64` しか提供していない |

## 5. 移行とリリース

- v0.6.0 で初回ダウンロードを導入する。既存の同梱版からアップデートした場合、初回にブラウザの再ダウンロードが必要になることをリリースノートに明記する
- ベータで失敗率（`network_unreachable` / `checksum_mismatch` / `launch_failed`）を確認する
- 取得スクリプトと `bundle.resources` の宣言は実装時に削除済み。互換のフォールバックだけを残す
- オフライン / 企業向けには、`<app-data>/chromium` へ手動配置する手順を提供する

## 6. プラットフォーム固有の注意

現行仕様に含まれない運用上の注意だけを残す。

### macOS

- `Google Chrome for Testing.app` の内部名を変更しない。変更すると Mach-O 署名が壊れる
- CfT は ad-hoc / linker-signed で、上流 zip でも `codesign --verify --deep --strict` は成功しない。署名検証を必須にせず、SHA-256 と `--version` の起動確認で担保する
- Gatekeeper の quarantine は通常付かないが、配布経路によっては付く。付いた場合の挙動は実機で確認する

### Windows

- x64（`win64`）を基本とする。ARM64 は CfT の `win-arm64` があるが、CI / 実機検証の対象外なら「未対応」と明示する
- 実行中の `chrome.exe` は置換できない。更新前に全バックエンドを閉じる
- SmartScreen / Defender が AppData 内の `chrome.exe` を警告する可能性があるため、リリースノートに記載する

### Linux

- Chrome for Testing は `linux64` のみ。arm64 Linux はシステム Chromium フォールバックのみとし、未対応を明記する
- `.deb` の `depends` に Chromium の実行時ライブラリ（`libnss3`, `libatk-bridge2.0-0`, `libcups2`, `libdrm2`, `libxkbcommon0`, `libgbm1`, `libasound2`, `libxcomposite1`, `libxdamage1`, `libxrandr2` など）を追加するか、起動失敗時に不足ライブラリを案内する
- AppImage はホスト側ライブラリに依存する。`ldd` / 起動テストで事前検出できるとよい

### 共通

- プロキシ環境では `reqwest` のシステムプロキシ対応を確認し、必要なら設定項目を追加する
- インストーラーは固定ホスト（`storage.googleapis.com`）のみへアクセスし、エージェントのツールからは呼べない。アプリの egress allowlist とは別扱いにする

## 7. 参照

- 現行仕様: [`architecture.md`](architecture.md) の「Managed Chromium（初回インストール方式）」
- 実装・検証の記録: [`../verification.md`](../verification.md) の 2026-10-28「Desktop Chromium のオンデマンドインストール化」
- 公開前の確認: [`../release-checklist.md`](../release-checklist.md)
- 固定マニフェスト: `apps/desktop/browser/src/chromium-manifest.json`（`pnpm chromium:pin` で更新）
