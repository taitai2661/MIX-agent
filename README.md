<div align="center">

# MIX-agent

**ローカル・セルフホストのための AI ワークスペース**

[![CI](https://github.com/taitai2661/MIX-agent/actions/workflows/ci.yml/badge.svg)](https://github.com/taitai2661/MIX-agent/actions/workflows/ci.yml)
[![Release](https://github.com/taitai2661/MIX-agent/actions/workflows/release.yml)](https://github.com/taitai2661/MIX-agent/actions/workflows/release.yml)
[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)
[![Docker](https://img.shields.io/badge/docker--compose-2496ED?logo=docker&logoColor=white)](compose.yaml)

日本語 | [English](README.en.md)

</div>

---

MIX-agent は、外部のクラウドサービスに依存せず、自分のマシンまたは信頼できるネットワーク内で安全に AI を活用するためのワークスペースです。

本リポジトリでは、利用シーンに合わせて **Desktop 版** と **Docker 版** の 2 つのエディションを提供しています：

- **Desktop 版**: **「OpenCode のように自律動作するデスクトップ・コーディングエージェント」**。手元のローカル PC でコード探索・計画・複数ファイル編集・永続 PTY ターミナルでのテスト＆修正ループを直接回せる開発相棒（Tauri 2 + Rust + React）。Docker 不要でインストーラーから即座に起動できます。
- **Docker 版**: **「自宅サーバー（自宅鯖）や NAS でチャット AI を自己ホストしたい」** ニーズに応えるセルフホストサーバー（FastAPI + PostgreSQL + React）。24時間稼働させ、Web ブラウザ経由でスマホやタブレット、ノート PC など家中からアクセスできます。

> [!NOTE]
> **開発ステータス**: 初期開発版です。インターネットへの無制限な一般公開や、不特定多数のマルチテナント利用、悪意ある未知のコードを安全に実行する用途には使わないでください。

---

## どっちを選ぶべき？（エディション比較）

「ローカル PC で OpenCode のようにコーディングやファイル作業を自律実行させたいか」「自宅鯖でプライベート AI を常時ホストしたいか」など、用途に合わせて選択してください。

| 比較項目 | Desktop 版 (`apps/desktop`) | Docker 版 (`apps/server`, `apps/web`) |
| :--- | :--- | :--- |
| **主な用途・役割** | **デスクトップ・コーディングエージェント（OpenCode のように動作）**<br>ローカル PC でのコード探索・差分編集・ビルド/テスト実行・作業支援 | **自宅鯖でのチャット AI 自己ホスト**<br>24時間常時稼働、LAN/VPN 共有、NAS(ZimaOS/CasaOS)、HomeLab |
| **得意なタスク** | リポジトリ探索・コード編集・ターミナル実行（git/npm/test） | チャット会話・Web調査・長時間のバックグラウンド自律タスク |
| **アクセス元** | インストールした PC 単体（ネイティブ GUI） | **Web ブラウザ**（スマホ・タブレット・PC 等からアクセス） |
| **稼働形態** | PC 起動時のみ（手軽なネイティブ起動） | **自宅鯖で 24 時間常時稼働**（バックグラウンド実行） |
| **導入の手軽さ** | ⭐⭐⭐ **極めて簡単**（Docker 不要）<br>Releases からインストーラーをダウンロードして起動 | ⭐⭐ **標準**（Docker 環境が必要）<br>`git clone` 後に `docker compose up -d` で起動 |
| **ユーザーインターフェース** | OS ネイティブウィンドウ、差分プレビュー、システムトレイ | Web ブラウザ（レスポンシブ UI） |
| **技術スタック** | Tauri 2 + Rust + React 19 | FastAPI (Python) + PostgreSQL + React 19 |
| **プロセス構成** | 単一バイナリ + tokio in-proc（+ 子プロセス Chromium） | Docker Compose（app, web, db, runner, proxy コンテナ群） |
| **データ・秘密情報の保存** | ローカル SQLite (WAL/FTS5) + **OS Keychain** | PostgreSQL + AES-GCM 暗号化（Docker ボリューム） |
| **端末・ファイル操作** | ローカルマシンのファイルを直接編集、**永続 PTY シェル** | マウントされた `/project` や専用コンテナ内での操作 |
| **ブラウザ操作** | 同梱 Chromium / システムブラウザを直接自動操作 | execution-runner コンテナ内の Playwright |
| **セキュリティ・境界** | OS ユーザー権限。**操作単位の承認ルール**（パス/コマンド等） | コンテナによるプロセス・ネットワーク隔離（egress-proxy） |

---

## 導入・起動方法

### Desktop 版で起動（デスクトップ・コーディングエージェント）

Docker や複雑なサーバー設定なしで、OpenCode のようにローカル PC のワークスペースで直接コーディング支援やファイル作業を実行できます。

#### 1. インストーラーから導入（推奨）

[GitHub Releases](https://github.com/taitai2661/MIX-agent/releases) からお使いの OS に合わせた最新インストーラーをダウンロードします：

- **macOS**:
  - Apple Silicon (M1/M2/M3/M4): `MIX-agent-Desktop-Apple-Silicon.dmg`
  - Intel Mac: `MIX-agent-Desktop-Intel.dmg`
  - *インストール手順*: `.dmg` を開き、`MIX-agent Desktop` を `Applications` フォルダへドラッグ＆ドロップします。
  - *macOS Gatekeeper の注意*: 開発版のため未署名・未公証です。初回起動時に警告が出た場合は、Control キーを押しながらアプリアイコンをクリックして「開く」を選択するか、ターミナルで `xattr -cr "/Applications/MIX-agent Desktop.app"` を実行してください。
- **Windows (x64)**:
  - ※ Windowsインストーラーは次回リリース以降で配布します。v0.5.0時点ではmacOSのみです。
  - 推奨: `MIX-agent-Desktop-Windows-x64-Setup.exe`（NSIS。日本語/英語、ユーザー単位インストール）
  - `MIX-agent-Desktop-Windows-x64.msi`（英語。組織配布向け）
  - *対応 OS*: Windows 10 1809 以降 / Windows 11
  - *WebView2*: 初回インストール時にブートストラップが導入を試みます。WebView2 が未導入の場合はインターネット接続が必要です。
  - *Windows SmartScreen の注意*: 未署名のため「Windows によって PC が保護されました」と表示される場合があります。「詳細情報」→「実行」を選択してください。
  - *ターミナル / Git*: [Git for Windows](https://gitforwindows.org/)（Git Bash）を推奨します。未導入の場合、ターミナルは PowerShell にフォールバックしますが、POSIX コマンド（`ls` / `grep` など）や Git 連携の一部は使えません。
- **Linux**: 現在の GitHub Release では配布していません。必要な場合はソースからビルドしてください。

ダウンロード後は、Release に添付された `SHA256SUMS` とファイルのチェックサムを照合してください。

#### 2. ソースコードから起動・ビルド

Rust (1.82+)、Node.js (22+)、pnpm を使用します。

```sh
# 依存関係のインストール
pnpm install

# デスクトップ版の起動（開発モード）
pnpm desktop:dev

# 現在の OS 向けインストーラーのローカルビルド
# macOS: .dmg / Windows: NSIS + MSI
pnpm desktop:build

# macOS 向け .dmg のみをビルド
pnpm desktop:build:dmg
```

詳細な設定やアーキテクチャは [`apps/desktop/README.md`](apps/desktop/README.md) を参照してください。

---

### Docker 版で起動（自宅鯖・セルフホスト）

自宅サーバー、HomeLab、NAS（ZimaOS/CasaOS）等で 24 時間常時稼働させ、自分専用のプライベートなチャット AI / 自律エージェント環境を自己ホストできます。外部クラウドに会話ログを預けることなく、宅内 LAN や VPN 経由でスマホや PC からブラウザで快適にアクセスできます。

Docker Desktop、または Docker Engine と Compose v2 を用意します。
リポジトリを clone して直下で起動するだけで、追加の設定ファイル作成なしですぐに立ち上がります。

```sh
git clone https://github.com/taitai2661/MIX-agent.git
cd MIX-agent
docker compose up -d --build
```

起動後、ブラウザで [http://localhost:8080](http://localhost:8080)（自宅鯖の場合は `http://<自宅鯖のIP>:8080`）を開いて初期管理者アカウントを作成します。
自宅鯖ローカルで動作する Ollama / LM Studio / vLLM 等に接続する場合、Docker 内からはホストマシンの `localhost` ではなく `host.docker.internal` を使用します。

#### プロジェクトフォルダの接続

作業対象のリポジトリを `/project` にマウントすると、そのフォルダの `AGENTS.md`（または `CLAUDE.md`）が指示書として自動読み込みされます：

```sh
MIX_PROJECT_DIR=/path/to/your/repo docker compose up -d
```

#### 更新とバックアップ

```sh
git pull --ff-only
docker compose up -d --build
```
データは Docker ボリュームに安全に保存されます（`docker compose down -v` はボリュームを消去するため実行しないでください）。更新前に Web 管理画面から暗号化バックアップを取得することを推奨します。

---

## 主な機能（コア機能）

どちらのエディションでも、AI 実行エンジンとして高度な自律性と豊富な連携機能を備えています：

### 1. Chat / Thinking / Agent の 3 つのモード
- **Chat**: 素早い対話と必要に応じた検索・作成。思考や Tool Calling が未対応のモデルでも通常対話が可能。
- **Thinking**: Reasoning（思考）対応モデルの能力を引き出し、論理的な思考ステップを経てタスクを実行。
- **Agent**: 長時間の作業を自律完遂するための実行ループ。チェックポイント保存、停滞検知と自己修復、厳格な検証ループ、予算動的調整を備えます。

### 2. 52 種類の Provider 対応 & 自動能力判定
- OpenAI、Anthropic、Gemini、OpenRouter、Groq、Mistral、DeepSeek、Cohere、Ollama、LM Studio などの主要 Provider プリセットを網羅。
- **Capability Probe**: モデルの Tool Calling 対応状況を実行前に副作用のない安全なプローブで自動検証。

### 3. Tool Registry と柔軟な権限管理
- ファイル操作、コード編集、永続ターミナル、Web 検索、ブラウザ操作、メモリ、ナレッジ検索などの組み込みツール。
- **権限制御**: ツール呼び出しごとに `Always Allow` / `Ask` / `Deny` を指定可能。承認ルールはスコープ（特定パスや特定コマンド）に絞って永続化可能。

### 4. MCP（Model Context Protocol）連携
- stdio および Streamable HTTP / Remote MCP に対応。
- 外部 MCP サーバーが提供するツールをシームレスに Agent から利用可能。

### 5. Skills とナレッジ管理
- `SKILL.md`（Agent Skills 仕様）のインポート・エクスポートに対応。
- 外部ドキュメントのチャンク検索（Knowledge）や、プロジェクト固有の指示書連携。

---

## 安全上の境界とセキュリティモデル

エディションによってセキュリティとプロセスの隔離方式が異なります：

### Desktop 版のセキュリティモデル
- **OS 権限**: ログイン中のローカルユーザーの権限でネイティブ動作します。
- **秘密情報**: API Key や認証情報は、OS 標準の安全なストレージ（macOS Keychain, Windows Credential Manager, Linux Secret Service）に暗号化して保管されます。
- **権限ゲート**: ファイル書き込み、コマンド実行、ブラウザ操作などは既定で確認ダイアログ（`Ask`）が表示され、許可したパスやコマンドのみが実行されます。

### Docker 版のセキュリティモデル
- **コンテナ隔離**: 実行部（Runner）は非 root、read-only rootfs、Capability 制限でコンテナ起動します。ホストや Docker Socket はマウントされません。
- **通信制限**: Runner は内部ネットワークと egress-proxy を経由して通信し、管理者によって許可された公開ドメインへの接続のみを許可します。Private IP への接続は自動的に拒否されます。
- **ネットワーク公開**: 標準では `localhost:8080` のみ公開。LAN 公開やリバースプロキシ構成時は `MIX_BIND` や `PUBLIC_ORIGIN` を適切に設定してください。

---

## 開発

本リポジトリは pnpm ワークスペースを用いたモノレポ構成です。

```sh
# 依存関係のインストール
pnpm install

# --- Desktop 版の開発 ---
pnpm desktop:dev        # デスクトップアプリ起動
pnpm desktop:ui:dev     # UI 単体開発サーバー
pnpm desktop:ui:test    # UI 単体テスト
cargo test --workspace --manifest-path apps/desktop/Cargo.toml # Rust コアテスト

# --- Docker / Web 版の開発 ---
pnpm web:dev            # Web フロントエンド開発サーバー
pnpm web:test           # Web フロントエンドテスト
docker compose --profile test run --rm test # バックエンドテスト
```

詳細は [CONTRIBUTING.md](CONTRIBUTING.md) を参照してください。

---

## 主なドキュメント

| ドキュメント | 内容 |
| :--- | :--- |
| [Desktop 版 README](apps/desktop/README.md) | Desktop 版の構成、ツール仕様、ビルド手順の詳細 |
| [システムアーキテクチャ](docs/architecture.md) | プロジェクト全体の構造とエディションの設計思想 |
| [Desktop アーキテクチャ](docs/desktop/architecture.md) | Desktop 版（Rust + Tauri 2）の内部設計と API マッピング |
| [運用ガイド](docs/operations.md) | Docker 版の運用、バックアップ・復元手順 |
| [検証状況](docs/verification.md) | 実機・Provider での動作検証ログ |
| [ZimaOS ガイド](docs/zimaos.md) | ZimaOS / CasaOS コミュニティストアでの導入 |
| [コントリビューション](CONTRIBUTING.md) | 開発の流れと PR の提出手順 |

---

## ライセンス

本プロジェクトは [Apache License 2.0](LICENSE) で提供されます。詳細は [NOTICE](NOTICE) を参照してください。
