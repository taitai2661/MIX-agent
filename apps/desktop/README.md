# MIX-agent Desktop

Tauri 2 + Rust + React 19 で実装された、**OpenCode のように自律動作するデスクトップ・コーディングエージェント**。
ローカルマシンの作業ディレクトリ（ワークスペース）に直接接続し、コード探索（`grep`/`glob`）、構造化計画立案、最小差分のコード編集（`edit`）、永続 PTY シェルでのビルド・テスト・検証ループ（`terminal`）を自律的に回せる開発相棒です。

Docker 版の中核機能（Provider Adapter / Run Engine / Tool Registry / SSE 契約）の仕様を踏襲しつつ、**Docker 不要の単一バイナリ・OS 安全ストレージ連携・ローカル直接作業**に特化しています。

> [!IMPORTANT]
> **Docker 版との完全分離**: `apps/desktop/` は Docker 版の Python / FastAPI バックエンドや PostgreSQL に一切依存しません。同一の仕様書（[`docs/desktop/architecture.md`](../../docs/desktop/architecture.md) および [`docs/desktop/api-mapping.md`](../../docs/desktop/api-mapping.md)）に基づいて独立して Rust で再実装されています。

---

## 主な特徴（Docker 版との違い）

| 特徴 | MIX-agent Desktop | Docker 版 |
| :--- | :--- | :--- |
| **主な用途・役割** | **デスクトップ・コーディングエージェント（OpenCode のように動作）**<br>ローカル PC でのコード探索・差分編集・テスト実行・開発支援 | **自宅鯖でチャット AI を自己ホストしたい**<br>24時間常時稼働、Web ブラウザで家中（スマホ/PC）から利用 |
| **得意なタスク** | コード編集、テスト/lint 検証ループ、ローカル開発タスク | チャット会話、Web 調査、常時稼働バックグラウンド実行 |
| **起動環境** | Docker 不要。`.dmg` / `.exe` / `.deb` インストーラーから即起動 | Docker Engine & Docker Compose が必要 |
| **プロセス構成** | 単一デスクトップバイナリ（tokio in-proc タスク） | 5 つの Docker コンテナ（app, web, db, runner, proxy） |
| **データ管理** | SQLite (WAL モード + FTS5 全文検索)。DB 本体は平文のため、保存時の保護は OS のディスク暗号化（FileVault / BitLocker / LUKS）に依存 | PostgreSQL (pg_trgm インデックス) |
| **秘密情報保護** | AES-256-GCM 暗号化ファイル。macOS は `.master_seed`（0600）方式、Windows は Credential Manager にマスター鍵。Linux はファイル方式（Secret Service があれば `MIX_SECRET_BACKEND=keychain-seed` で切替可） | 暗号化 DB カラム (AES-GCM + scrypt) |
| **ターミナル実行** | ローカルマシンの **永続 PTY シェル**（環境変数や `cd` が継続） | execution-runner コンテナ内の非 root シェル |
| **ブラウザ操作** | アプリ内のネイティブ子 WebView をエージェントと共有 | execution-runner コンテナ内の Playwright |
| **ファイル操作** | ホストマシンのファイルに直接アクセス・インライン差分編集 (`edit`) | マウントされたボリュームのみ操作可能 |

---

## ディレクトリ構成

```text
apps/desktop/
├── Cargo.toml            # Cargo workspace root（app / core / providers / tools / browser / mcp / storage）
├── rust-toolchain.toml   # 固定ツールチェーン（1.91.0 / stable + rustfmt / clippy）
├── rustfmt.toml          # rustfmt 設定
├── clippy.toml           # clippy 設定
├── package.json          # Desktop 側の pnpm workspace root
├── pnpm-workspace.yaml   # ui/ を指定
├── app/                  # Tauri 2 shell（エントリポイント、バイナリ、capabilities）
├── core/                 # Run Engine / state / events / context（純粋ロジック）
├── providers/            # wire 実装（OpenAI / Anthropic / Gemini）+ ラッパー
├── tools/                # Tool Registry + ツール実行基盤
├── browser/              # Browser tool contract（エンジン非依存）
├── mcp/                  # MCP client host (stdio / Remote HTTP)
├── storage/              # SQLite + 秘密ストア (file seed / keyring-rs)
├── scripts/              # UI / リリース用ヘルパー
└── ui/                   # React 19 + Vite + Tailwind（Desktop 専用ネイティブ UI）
```

---

## 必要環境

| ツール | バージョン | 用途 |
| :--- | :--- | :--- |
| **Rust** | 1.91 以上（固定: 1.91.0） | Cargo workspace のビルド |
| **Node.js** | 22 以上 | UI のビルド・テスト |
| **pnpm** | 10.x（`10.11.0`） | パッケージマネージャ |
| **Tauri CLI** | 2.x（`^2.12.1`） | `tauri dev` / `tauri build` |

### OS 別の追加要件
- **macOS**: Xcode Command Line Tools（macOS 12.0 以上推奨）
- **Windows**: Visual Studio Build Tools 2022 (C++ workload) + WebView2。ターミナルで POSIX コマンドを使う場合は Git for Windows（Git Bash）を推奨。無い場合は PowerShell にフォールバックします。
- **Linux**: `libwebkit2gtk-4.1-dev`, `libssl-dev`, `librsvg2-dev`, `libappindicator3-dev`, `libayatana-appindicator3-dev`, `libxdo-dev`, `libgtk-3-dev`, `patchelf`

### 内蔵ブラウザ（ネイティブ子 WebView）
Desktop版は Chromium を同梱・ダウンロードしない。Tauri の子 WebView
（macOS: WKWebView / Windows: WebView2 / Linux: WebKitGTK）をメインウィンドウに
追加し、チャット右側のブラウザパネル（Codex 風のツールバー・複数タブ・
フル/スプリット表示）から操作する。`⌘⇧B`（macOS）/ `Ctrl+Shift+B`
（Windows / Linux）で開閉でき、エージェントが `browser` ツールを使うと
自動で表示される。

ページのスクロール・IME・ホバー・コピペは OS WebView のネイティブ動作で、
エージェントも同じページを `eval_with_callback` で操作する。Cookie とログインは
このアプリ専用のブラウザデータに保存され、通常の Chrome とは共有されない。

---

## 開発とクイックスタート

リポジトリルートから、または `apps/desktop/` ディレクトリから実行できます。

### リポジトリルートから実行する場合（推奨）

```sh
# 依存関係のインストール
pnpm install

# デスクトップアプリの起動（Tauri 開発サーバー + Vite UI）
pnpm desktop:dev

# UI 単体での開発サーバー起動（ブラウザ確認用: http://localhost:5173）
pnpm desktop:ui:dev

# UI 単体テストの実行
pnpm desktop:ui:test

# 現在の OS 向けインストーラーのビルド
# macOS: .dmg / Windows: NSIS + MSI / Linux: AppImage + deb
pnpm desktop:build

# Windows 向け NSIS + MSI のみをビルド
pnpm desktop:build:windows

# macOS 向け .dmg のみをビルド
pnpm desktop:build:dmg
```

### `apps/desktop` ディレクトリ内で実行する場合

```sh
cd apps/desktop
pnpm install

# アプリ起動
pnpm tauri dev

# パッケージビルド
pnpm tauri:build:dmg # macOS .dmg
pnpm tauri:build     # 現在の OS の全ターゲット（Windows は NSIS + MSI）
```

初回起動時は Tauri が Cargo workspace 全体をビルドするため数分かかります。2回目以降はキャッシュが効いて高速に起動します。

### ヘッドレス診断

インストール済みバイナリは、ウィンドウを開かずに環境を確認できます。

```sh
"MIX-agent Desktop" --doctor          # 人が読む形式
"MIX-agent Desktop" --doctor --json   # サポート / CI 向け
"MIX-agent Desktop" --version
```

`--doctor` はデータディレクトリの書き込み可否、DB の schema version と `quick_check`、秘密情報バックエンド（`file` / `keychain-seed` / `keychain`）の到達性を読み取り専用で確認します。macOS の既定は `file` で、旧 Keychain seed の移行後は Keychain にアクセスしません。DB を変更しないため、破損時の一次切り分けに使えます。

### 言語サーバー（任意）

`lsp` ツールは PATH 上の言語サーバーを自動検出します。未導入でも他の機能は動きます。

| 言語 | サーバー |
| :--- | :--- |
| Rust | `rust-analyzer` |
| TypeScript / JavaScript | `typescript-language-server` |
| Python | `pyright-langserver` または `pylsp` |
| Go | `gopls` |

`MIX_LSP_RUST` / `MIX_LSP_TYPESCRIPT` / `MIX_LSP_PYTHON` / `MIX_LSP_GO` に実行ファイルのパスを設定すると、そのパスを優先します。


---

## 利用可能なツールと権限モデル

Desktop 版の Agent は、ローカルマシンでの作業を最大限効率化する多彩なツールを備えています：

| ツール名 | 概要 | 既定権限 |
| :--- | :--- | :--- |
| `files` | ファイルの read / write / list / search / delete。大容量ファイルはページング対応 | 読み取り: allow / 書き込み: ask |
| `edit` | 最小差分の文字列置換・新規作成・追記。曖昧なアンカーは拒否 | ask |
| `glob` | `.gitignore` を考慮した glob パターンによるファイルパス検索 | allow |
| `grep` | 高速な正規表現マッチングによるファイル内容検索 | allow |
| `symbols` | `map`（リポジトリ全体の構造マップ）/ `outline`（ファイル内の定義一覧）/ `references`（シンボル参照検索） | allow |
| `lsp` | 実言語サーバーによる `diagnostics`（型エラー・lint）と `definition`（宣言へジャンプ）。rust-analyzer / typescript-language-server / pyright・pylsp / gopls を自動検出 | allow |
| `git` | 読み取り専用の `status` / `diff` / `log` / `show` / `blame` | allow |
| `terminal` | **永続 PTY シェル**。`cd` によるカレントディレクトリ移動や環境変数が引き継がれる。`action: "send"` で実行中の REPL やプロンプトへ入力できる | ask |
| `workspace_check` | シェルなしで git status, git diff, python syntax を安全に確認 | allow |
| `knowledge` | SQLite + FTS5 によるドキュメントの登録・全文検索・Web クリップ保存 | 検索: allow / 書込: ask |
| `todo` | 複数ステップ作業のためのチェックリスト。ターン毎に AI へ自動注入 | allow |
| `skill` | 保存された `SKILL.md` の指示手順を読み取り | allow |
| `skill_resource` | Skill に同梱された `scripts/` / `references/` / `assets/` を一覧・読み取り（実行はしない） | allow |
| `agent` | 保存済みサブエージェントプロファイルへの処理委任。`tasks` で並列実行、`isolate: true` で使い捨て Git worktree に分離し、差分を受け取る | ask |
| `web_search` | DuckDuckGo による Web 検索 | allow |
| `web_fetch` | URL からのコンテンツ取得（egress 制御 & プライベート IP 拒否） | ask |
| `web_fetch_pdf` | Web 上の PDF からテキスト抽出 | ask |
| `browser` | アプリ内のネイティブ WebView を操作（snapshot + ref、クリック/入力、タブ、スクショ等）。localhost は「ブラウザのローカルアクセス」を ON にした場合のみ | ask |
| `mcp__<conn>__<tool>` | 接続された外部 MCP サーバーのツール | ask |

### 権限ゲート（Permission Gate）とスコープ制限

- **呼び出し単位の判定**: 単に「ファイル書き込み全体」を一括許可するのではなく、「どのパスへの書き込みか」「どのコマンドの実行か」といった**操作対象のスコープ**に絞って許可ルールが保存されます。
- **安全な既定値**: 読み取りや検索は即座に実行（`allow`）されますが、ファイル変更・シェル実行・ブラウザ操作は常にユーザーの確認（`ask`）を挟みます。

### ChatGPT（非公式・専用連携）

- 設定 ▸ **ChatGPT** の専用タブから、デバイスコードでログインします。
- プロバイダーのプリセットには表示されません。汎用の Provider 作成フォームからは作れず、接続は1つだけです。
- 接続後に取得したモデルだけが「モデル」タブに並びます。使用枠（5時間 / 週次）とログアウトも同じタブで管理します。

### バックグラウンドとスケジュール

- 設定 ▸ 一般の「ログイン時に起動」で OS の自動起動に登録します（macOS は LaunchAgent、Windows は Run キー、Linux は XDG autostart）。起動時は `--hidden` でトレイ常駐します。
- ウィンドウを閉じてもプロセスは残り、スケジュール・実行中の Run・承認待ちは継続します。完全終了はトレイメニューの「Quit」です。

### 変更レビューとロールバック

- Run 開始時に Git の作業ツリー（未コミット変更を含む）をスナップショットし、`refs/mix-agent/runs/<id>` に固定します。
- 実行後は「このRunの変更を取り消す」で、`edit` / `files` に加えて**ターミナルが作成・変更したファイル**も実行前へ戻せます。変更ファイル一覧は Activity パネルで確認できます。
- Git 管理外のフォルダでは従来どおり `edit` / `files` の変更だけが対象です。

### 大規模な会話

- サイドバーは 50 件ずつのカーソルページング（`updated_at` + `id`）で読み込み、下端までスクロールすると次のページを取得します。
- 40 メッセージを超えるトランスクリプトは `@tanstack/react-virtual` で可視領域だけを描画します。
- コンテキスト圧縮が起きた Run には「履歴を圧縮しました」マーカーを表示し、保持件数・退避したツール結果を確認できます。
