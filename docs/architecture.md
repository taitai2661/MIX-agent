# アーキテクチャ

本ドキュメントでは、MIX-agent プロジェクト全体のアーキテクチャ構成と、各エディションの設計思想について解説します。

---

## 全体構成とモノレポ構造

MIX-agent は単一のモノレポ内で **Desktop 版** と **Docker 版** の 2 つのエディションを管理しています。両エディションは同一の設計思想と仕様契約（Provider Adapter、Run Engine、Tool Registry、Run Event kind、Capability 状態など）を共有していますが、**実装コードは完全に分離**されています。

```text
MIX-agent/
├── apps/
│   ├── desktop/            # Desktop 版: Tauri 2 + Rust + React 19（完全独立）
│   ├── server/             # Docker 版バックエンド: FastAPI (Python)
│   └── web/                # Docker 版フロントエンド: React 19 + Vite
├── docs/                   # 共通ドキュメント & 仕様書
│   └── desktop/            # Desktop 版専用の内部設計・API マッピング
├── packages/               # 共通ツール・スクリプト
└── compose.yaml            # Docker Compose 設定
```

| エディション | 技術スタック | アーキテクチャ | 目的・ユースケース |
| :--- | :--- | :--- | :--- |
| **Desktop 版** | Tauri 2 + Rust + React 19 | 単一バイナリ + tokio in-proc + SQLite + Keychain | 手元のローカル PC 作業、Docker 不要、高速・低リソース |
| **Docker 版** | FastAPI + PostgreSQL + React 19 | マルチコンテナ（Compose）+ Runner 隔離 + Proxy | **自宅鯖でのチャット AI 自己ホスト**、LAN/VPN 共有、NAS(ZimaOS)、常時稼働 |

> Desktop 版の詳細な内部構造や Rust クレート設計については、[`docs/desktop/architecture.md`](desktop/architecture.md) を参照してください。

---

## Docker 版アーキテクチャ

Docker 版（`apps/server`, `apps/web`）は、FastAPI と PostgreSQL を中核とするモジュラーモノリスです。

```text
React (Web UI) ─► REST / SSE ─► FastAPI (apps/server)
                                    ├─► PostgreSQL (DB / pg_trgm / JSONB)
                                    ├─► Provider Adapter (OpenAI / Anthropic / Gemini / etc.)
                                    └─► Run Engine ─► Tool Registry ─► Permission
                                                         ├─ 本体組込（Memory / Web）
                                                         ├─ execution-runner (コンテナ内操作)
                                                         └─ mcp-runner (MCP サーバー実行)
                                                                 │
                                                                 ▼
                                                            egress-proxy ─► 許可ドメイン (HTTPS)
```

### 1. プロセスと並行処理
- FastAPI は 1 プロセスで起動します。メモリ内の Task 管理や承認の排他制御は単一プロセス前提です。
- 会話単位の有効な Run は部分ユニークインデックスにより 1 件に制限されます。

### 2. データストア（PostgreSQL）
- ユーザー、セッション、Run、Event、ToolCall、Approval、RunCheckpoint は専用の構造化カラムを持ちます。
- Provider、Model、Agent、Tool、MCP、Memory などの拡張設定は専用テーブルの JSONB カラムに保存します。
- マイグレーションは Alembic で管理され、Memory 本文には `pg_trgm` インデックスを作成して部分一致検索を行います。

### 3. Provider Adapter と Capability Probe
- `Adapter.stream()` は各 Provider の出力を共通の Run イベント形式へ変換します。署名や暗号化された推論履歴は保持しつつ UI には不要な内部トークンを公開しません。
- **Capability 状態**: 各モデルの Tool Calling 能力は「対応 / 非対応 / 未確認」の 3 状態を保持します。未確認の場合は副作用のないダミー関数を用いたプローブで自動確認します。

### 4. 実行状態と自律性（Agent モード）
- 状態遷移: `queued → running → waiting_approval → running → completed`
- **チェックポイント**: 長時間実行される Agent モードでは 10 ステップごとに中間スナップショットを保存し、任意のチェックポイントからの再開（Resume）が可能です。
- **停滞検知と自己修復**: 繰り返しの失敗や読み取り停滞を検知し、モデルへ修復ガイダンスを注入します。
- **階層的要約（Tiered Summary）**: コンテキスト上限に近づいた際、重要タスク状態（ゴールや未完了タスク）を保護しながら過去の会話履歴を段階的に要約圧縮します。

### 5. セキュリティと隔離（Runner & Egress Proxy）
- **隔離コンテナ**: シェル実行やファイル操作を行う `execution-runner`、および MCP サーバーを動かす `mcp-runner` は、非 root・read-only filesystem・Capability 削除で起動します。ホストの Docker Socket や DB パスワードはマウントされません。
- **通信制御**: Runner からの外部接続はすべて `egress-proxy` を経由します。管理者が明示的に許可した公開ドメインへの接続のみを許可し、Private IP へのアクセスは自動的に遮断されます。
- **Secret 保護**: API Key や秘密情報は AES-GCM で暗号化されて保存されます。

---

## Desktop 版との設計対比

| 概念 | Docker 版 (`apps/server`) | Desktop 版 (`apps/desktop`) |
| :--- | :--- | :--- |
| **通信契約** | FastAPI REST API (`/runs/...`) + SSE | Tauri IPC (`#[tauri::command]`) + Window イベント (`run_event`) |
| **ストレージ** | PostgreSQL + Alembic | SQLite (WAL モード) + sqlx マイグレーション |
| **秘密情報** | DB 内 AES-GCM 暗号化 | **OS Keychain** (keyring-rs) |
| **ブラウザ自動化** | コンテナ内 Playwright Chromium | 子プロセス実 Chromium (chromiumoxide) |
| **ターミナル** | コンテナ内の一時的シェル | ホスト OS の **永続 PTY シェル** |
| **マルチユーザー** | 管理者アカウント・セッション認証あり | 完全ローカル・単一ユーザー前提（認証不要） |
