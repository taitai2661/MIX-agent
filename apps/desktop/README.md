# MIX-agent Desktop

Tauri 2 + Rust + React 19 で実装された、個人向けローカル AI ワークスペース。
Docker 版の中核機能（Provider Adapter / Run Engine / Tool Registry / SSE 契約）の仕様を踏襲しつつ、**Docker 不要の単一バイナリ・OS Keychain 連携・ローカル直接作業**に最適化されたネイティブデスクトップアプリです。

> [!IMPORTANT]
> **Docker 版との完全分離**: `apps/desktop/` は Docker 版の Python / FastAPI バックエンドや PostgreSQL に一切依存しません。同一の仕様書（[`docs/desktop/architecture.md`](../../docs/desktop/architecture.md) および [`docs/desktop/api-mapping.md`](../../docs/desktop/api-mapping.md)）に基づいて独立して Rust で再実装されています。

---

## 主な特徴（Docker 版との違い）

| 特徴 | MIX-agent Desktop | Docker 版 |
| :--- | :--- | :--- |
| **主な用途・ニーズ** | **手元の PC ですぐ使いたい**<br>ローカルマシンのコーディング・ファイル直接編集 | **自宅鯖でチャット AI を自己ホストしたい**<br>常時稼働、Web ブラウザで家中（スマホ/PC）から利用 |
| **起動環境** | Docker 不要。`.dmg` / `.exe` / `.deb` インストーラーから即起動 | Docker Engine & Docker Compose が必要 |
| **プロセス構成** | 単一デスクトップバイナリ（tokio in-proc タスク） | 5 つの Docker コンテナ（app, web, db, runner, proxy） |
| **データ管理** | SQLite (WAL モード + FTS5 全文検索) | PostgreSQL (pg_trgm インデックス) |
| **秘密情報保護** | **OS Keychain**（macOS Keychain / Windows Credential Manager / Linux Secret Service） | 暗号化 DB カラム (AES-GCM + scrypt) |
| **ターミナル実行** | ローカルマシンの **永続 PTY シェル**（環境変数や `cd` が継続） | execution-runner コンテナ内の非 root シェル |
| **ブラウザ操作** | 同梱 Chromium / システムブラウザを tokio 子プロセスで直接制御 | execution-runner コンテナ内の Playwright |
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
├── browser/              # Browser Operator + chromiumoxide backend
├── mcp/                  # MCP client host (stdio / Remote HTTP)
├── storage/              # SQLite + OS Keychain (keyring-rs)
├── scripts/              # fetch-chromium.mjs（同梱 Chromium 取得スクリプト）
└── ui/                   # React 19 + Vite + Tailwind（Desktop 専用ネイティブ UI）
```

---

## 必要環境

| ツール | バージョン | 用途 |
| :--- | :--- | :--- |
| **Rust** | 1.82 以上（推奨: 1.91.0） | Cargo workspace のビルド |
| **Node.js** | 22 以上 | UI のビルド・テスト |
| **pnpm** | 10.x（`10.11.0`） | パッケージマネージャ |
| **Tauri CLI** | 2.x（`^2.12.1`） | `tauri dev` / `tauri build` |

### OS 別の追加要件
- **macOS**: Xcode Command Line Tools（macOS 12.0 以上推奨）
- **Windows**: Visual Studio Build Tools 2022 (C++ workload) + WebView2
- **Linux**: `libwebkit2gtk-4.1-dev`, `libssl-dev`, `librsvg2-dev`, `libappindicator3-dev`, `libayatana-appindicator3-dev`, `libxdo-dev`, `libgtk-3-dev`, `patchelf`

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

# macOS 向け .dmg のビルド
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
pnpm tauri:build     # 全ターゲット
```

初回起動時は Tauri が Cargo workspace 全体をビルドするため数分かかります。2回目以降はキャッシュが効いて高速に起動します。

---

## 利用可能なツールと権限モデル

Desktop 版の Agent は、ローカルマシンでの作業を最大限効率化する多彩なツールを備えています：

| ツール名 | 概要 | 既定権限 |
| :--- | :--- | :--- |
| `files` | ファイルの read / write / list / search / delete。大容量ファイルはページング対応 | 読み取り: allow / 書き込み: ask |
| `edit` | 最小差分の文字列置換・新規作成・追記。曖昧なアンカーは拒否 | ask |
| `glob` | `.gitignore` を考慮した glob パターンによるファイルパス検索 | allow |
| `grep` | 高速な正規表現マッチングによるファイル内容検索 | allow |
| `terminal` | **永続 PTY シェル**。`cd` によるカレントディレクトリ移動や環境変数が引き継がれる | ask |
| `workspace_check` | シェルなしで git status, git diff, python syntax を安全に確認 | allow |
| `knowledge` | SQLite + FTS5 によるドキュメントの登録・全文検索・Web クリップ保存 | 検索: allow / 書込: ask |
| `todo` | 複数ステップ作業のためのチェックリスト。ターン毎に AI へ自動注入 | allow |
| `skill` | 保存された `SKILL.md` の指示手順を読み取り | allow |
| `agent` | 保存済みサブエージェントプロファイルへの処理委任 | ask |
| `web_search` | DuckDuckGo による Web 検索 | allow |
| `web_fetch` | URL からのコンテンツ取得（egress 制御 & プライベート IP 拒否） | ask |
| `web_fetch_pdf` | Web 上の PDF からテキスト抽出 | ask |
| `browser` | 実 Chromium を自動操作（ページ遷移、クリック、スクショ等） | ask |
| `mcp__<conn>__<tool>` | 接続された外部 MCP サーバーのツール | ask |

### 権限ゲート（Permission Gate）とスコープ制限

- **呼び出し単位の判定**: 単に「ファイル書き込み全体」を一括許可するのではなく、「どのパスへの書き込みか」「どのコマンドの実行か」といった**操作対象のスコープ**に絞って許可ルールが保存されます。
- **安全な既定値**: 読み取りや検索は即座に実行（`allow`）されますが、ファイル変更・シェル実行・ブラウザ操作は常にユーザーの確認（`ask`）を挟みます。
