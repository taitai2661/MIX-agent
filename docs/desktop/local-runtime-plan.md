# MIX-agent Desktop — ローカル LLM ネイティブ連携（設計記録）

> 状態: L1〜L5 実装済み。Ollama ネイティブチャット（`num_ctx` / `keep_alive` / thinking フォールバック）、Auto のローカル優先、コンテキスト上限設定、プロセス所有権とジョブの競合対策まで実装済み。2026-10-28 に「モデル 0 件の識別済みランタイムを online として起動・pull 可能にする」「loopback と LAN の判定分離」「削除後の DB 同期」「承認カウントダウンの共有クロック」を追加。ハードウェア目安（RAM/VRAM からの推奨モデル表示）のみ未着手。実機は LM Studio で起動・停止・メタデータ取得を確認。Ollama は HTTP モックテストで確認済み（実機の pull / load / stop は未実施）。
> 現行の挙動は [`architecture.md`](architecture.md) の「Local runtime management」を正とする。本文は設計時点の判断理由と、コマンド・イベント・データモデルの詳細仕様を残す記録である。
> 対象: Desktop 版のみ（Tauri 2 / Rust / React）
> 方針: **外部ランタイム（Ollama / LM Studio / llama.cpp / vLLM 等）をアプリからネイティブに管理する**。推論エンジンやモデル重みの同梱は行わない。
> 既存の Phase 番号とは独立した **Local Runtime Track（L0〜L5）** として進める。

---

## 1. 結論

既存の `mixd_providers::discovery` は「検出して取り込む」ところまで完成している。
次に必要なのは、検出を単発のスキャンで終わらせず、**ランタイムの状態・起動停止・モデル管理・能力情報を一級の機能として扱うこと**。

優先順位は次のとおり。

1. **L1: メタデータのネイティブ化** — コンテキスト長・量子化・サイズ・ツール/vision 対応を実データから取得する。
2. **L2: ライフサイクル管理** — アプリから起動・停止・再起動し、状態を常時把握する。
3. **L3: モデル管理** — pull / delete / load / unload を進捗付きで行う。
4. **L4: UX と復旧** — Composer の状態表示、送信時自動起動、切断時の再試行、初回セットアップの短縮。
5. **L5（任意）: ネイティブチャット経路とローカル優先ルーティング** — `num_ctx` / `keep_alive` / thinking を正しく扱う。

L1 と L2 を先に出すだけで「入れた・起動した・モデルが見える・選べる」が完了し、体感のネイティブ度が大きく上がる。

---

## 2. 実装前の評価（履歴）

### 2.1 できていること

| 機能 | 実装 | 備考 |
| --- | --- | --- |
| ループバックのランタイム検出 | `apps/desktop/providers/src/discovery.rs` | 10 ポート × IPv4/IPv6、TCP + 指紋 + インストール痕跡 |
| ランタイム識別 | 同上 | Ollama / LM Studio / vLLM / llama.cpp / LocalAI / Jan / KoboldCpp / text-generation-webui / GPT4All |
| モデル一覧 | 同上 | Ollama は `/api/tags`、その他は `/v1/models` |
| ワンクリック取り込み | `apps/desktop/app/src/commands/discovery.rs` | provider 行 + adapter 登録 + model sync |
| API キー不要のローカル接続 | `apps/desktop/providers/src/openai.rs` | loopback では key gate を免除 |
| ローカル embedding | `apps/desktop/app/src/embedding.rs` | Ollama / LM Studio の既定モデルあり |
| モデルのコンテキスト長保持 | `apps/desktop/app/src/commands/models.rs` | DB に `contextWindow` を保存 |
| 能力フラグ保持 | 同上 | `vision` / `toolCalling` / `reasoning` |

### 2.2 足りないこと

| 不足 | 影響 |
| --- | --- |
| 起動・停止・再起動ができない | `launcher` は表示用の文字列だけで、実行パスも保持していない。ユーザーはターミナルへ戻る必要がある。 |
| モデルの pull / delete / load / unload がない | モデル取得は Ollama の CLI などアプリ外の作業。ロード状態も見えない。 |
| ネイティブ API を使っていない | Ollama の `/api/show`・`/api/ps`、LM Studio の `/api/v0/models` の情報を捨てている。 |
| コンテキスト長がほぼ `0` | Ollama の `/api/tags` には context がなく、`/api/show` を呼んでいない。`context.rs` は `0` を「不明」として 120k 文字の固定閾値にするため、8k モデルで溢れる。 |
| 能力判定が静的 | `verify_tools` は `provider.capabilities()` を返すだけで、実際のツール呼び出しを検証していない。OpenAI 互換は一律 `tool_calling: true`。 |
| 状態監視がない | ランタイムが落ちても Composer は気づかず、送信して初めて失敗する。 |
| 再取り込みが弱い | 同じラベルの provider は duplicate エラー。ランタイムのポート変更にも追従しない。 |
| 既定モデルが決まらない | 取り込んでもユーザーが別途モデルを選ぶ必要がある。 |
| ハードウェア適合の目安がない | どのサイズのモデルが動くか分からない。 |
| ログ・診断がない | 起動失敗の原因が UI から見えない。 |

### 2.3 既存コードの注意点

- `discovery.rs` は 1,000 行超。検出・指紋・インストール探索・モデル解析が同居しており、L1/L2 を足す前に分割したい。
- `locate_install()` は実行ファイルの **名前** しか返さない。起動には絶対パスが必要。
- `scan_local_runtimes()` はスキャン時に `/api/show` を呼ばない（意図的に高速化）。L1 では「速い一覧」と「重い詳細」を分離する。
- `verify_tools()` は静的な capability を保存するだけ。L1 で実プローブに置き換える。
- provider の `baseUrl` は `http://127.0.0.1:<port>/v1` 固定。ランタイムが別ポートで再起動した場合の再接続手段がない。
- ローカルのチャットは OpenAI 互換経路。`num_ctx` や `keep_alive` はこの経路では保証できない（L5 の課題）。

---

## 3. 目標と非目標

### 3.1 目標

- **G1**: インストール済みだが停止中のランタイムを、アプリから起動してそのままチャットできる。
- **G2**: モデルのコンテキスト長・サイズ・量子化・family・ロード状態を実データから表示する。
- **G3**: ツール / vision / reasoning の可否を、推測ではなくネイティブ情報または実プローブで判定する。
- **G4**: モデルの取得・削除・ロード・アンロードをアプリ内で完結させる。
- **G5**: ランタイムの状態を Composer と設定画面に常時表示し、切断時に復旧導線を出す。
- **G6**: 取り込み・既定モデル設定・初回送信までを最短 2〜3 クリックにする。
- **G7**: 上記を macOS / Windows / Linux で安全に動かす（プロセスの所有権とポート競合を守る）。

### 3.2 非目標

- llama.cpp / MLX などをアプリに同梱する推論エンジン（別プロジェクト）。
- モデル重みの自動ダウンロードをアプリのバンドルに含めること。
- ランタイム本体のサイレントインストール。
- Docker 版（`apps/server`）のローカル検出・起動管理。Docker 版は `host.docker.internal` 前提のまま。
- LAN の自動探索。手動 Base URL 指定は維持する。
- ポートを占有している任意のプロセスを kill すること。
- すべてのランタイムの完全なネイティブチャット対応。まずは管理 API を優先する。

---

## 4. 設計方針

### 4.1 ランタイムと provider の関係

- **provider は永続設定**（URL、ラベル、既定モデル、API キー有無）。
- **runtime は実行時の実体**（プロセス、状態、ロード中モデル、ジョブ）。
- 1 provider : 1 runtime endpoint を基本とする。同じ runtime を別ポートで 2 つ登録することは許容する。
- 永続化は当面 **provider の JSON に `local` オブジェクトを追加**し、モデルは既存の `models.data` を拡張する。新しいテーブルは作らない。
  - 理由: 既存の provider/model のライフサイクル（削除時の cascade、Auto 候補、embedding 解決）にそのまま乗れる。
  - 複数エンドポイントが常態化したら、L3 以降で `local_runtimes` テーブルへ切り出す。

### 4.2 能力は「ランタイム能力」と「モデル能力」を分ける

- **ランタイム能力**（`RuntimeCapabilities`）: 検出、起動、停止、pull、delete、load、unload、ログ、コンテキスト報告。
- **モデル能力**（`ModelCapabilities`）: chat、tools、vision、reasoning、embedding、コンテキスト長。
- UI は「このランタイムは delete できるか」「このモデルは tools が使えるか」を別々に描画する。ベンダー名で分岐しない。

### 4.3 チャット経路は当面 OpenAI 互換のまま

- 管理・メタデータ・モデル操作は **ネイティブ API**。
- チャットのストリーミングは既存の `OpenAiProvider` を再利用し、回帰リスクを抑える。
- `num_ctx` / `keep_alive` / thinking の正確な制御が必要になった時点で、Ollama の `/api/chat` 経路を L5 で追加する。
- それまでは「モデルの最大コンテキスト」と「サーバが実際に使うコンテキスト」を区別して表示し、圧縮閾値は安全側に倒す。

### 4.4 重い処理はジョブ + イベント

- scan / metadata enrichment / pull / load / start は即時応答させ、進捗は Tauri イベントで流す。
- ジョブは `job_id` を持ち、キャンセル可能にする。
- UI はコマンドの戻り値を待たず、イベントとスナップショットで状態を描画する。

### 4.5 安全境界

- **起動するのは、検出済みで絶対パスが解決できた実行ファイルだけ**。
- シェル文字列を組み立てない。`Command::new(path).args([...])` のみ。
- **アプリが起動した子プロセスだけ**を停止対象にする。ポート番号から PID を推測して kill しない。
- モデル名はベンダーの命名規則で検証し、パスや `..` を拒否する。
- 既定は loopback のみ。LAN は手動指定 + 明示的な許可。
- ログに API キーやプロンプト本文を出さない。

---

## 5. アーキテクチャ

```
WebView (React)
   │ invoke
   ▼
app/src/commands/local_runtime.rs        ← Tauri command surface
   │
   ▼
app/src/local_runtime/manager.rs         ← 状態機械・ジョブ・イベント
   ├── process.rs                         ← 子プロセス registry / logs / stop
   ├── jobs.rs                            ← pull/load 等の進捗・キャンセル
   └── health.rs                          ← 定期ヘルスチェック
   │
   ▼
mixd_providers::local                    ← ベンダー差を吸収（Tauri 非依存）
   ├── runtime.rs                         ← RuntimeId / capability matrix / state
   ├── ollama.rs                          ← /api/tags, /api/show, /api/ps, /api/pull ...
   ├── lmstudio.rs                        ← /api/v0/models + `lms` CLI
   ├── llamacpp.rs                        ← /health, /props, /v1/models
   ├── vllm.rs                            ← /health, /version, /v1/models
   └── model.rs                           ← LocalModel / ModelDetails / parse
```

### 5.1 状態機械

```
Unknown
  │ detect
  ▼
NotInstalled ──install──▶ Stopped ──start──▶ Starting ──ready──▶ Online
                             ▲                   │                  │
                             │                   └──error──▶ Error   │ health fail x2
                             │                                      ▼
                             └─────────────stop────────────── Stopping ──▶ Stopped
                                                                    │
                                                                    └──▶ Degraded
```

- `Online` は「モデル一覧が取得できた」だけでなく、ヘルスチェックに連続成功した状態。
- `Degraded` は一時的な失敗。2 回連続失敗で `Online` → `Degraded`、復帰で `Online` に戻す。
- `Error` は起動失敗・ポート競合など。`last_error` とログへの導線を持つ。
- `source` を別に持つ: `Managed`（アプリが起動） / `External`（ユーザーが先に起動）。`External` には stop を出さない。

### 5.2 データモデル

#### provider `data.local`

```json
{
  "runtime": "ollama",
  "executable": "/opt/homebrew/bin/ollama",
  "autoStart": null
}
```

- `executable`: 取り込み時に解決した絶対パス。見つからなくなったら再検出。
- `autoStart`: 省略時はグローバル設定（既定オフ）に従う。取り込み時には書き込まない。

#### model `data`（既存フィールドへの追加）

```json
{
  "contextWindow": 32768,
  "vision": true,
  "toolCalling": true,
  "reasoning": true,
  "auto": true,
  "sizeBytes": 4930000000,
  "parameterSize": "8B",
  "quantization": "Q4_K_M",
  "family": "qwen3",
  "loaded": true,
  "capabilitySource": "native",
  "capabilitiesVerifiedAt": "2026-10-27T12:00:00Z"
}
```

- `capabilitySource`: `native`（Ollama `/api/show` の capabilities 等） / `probe`（実プローブ） / `unknown`。
- `loaded` は `/api/ps` または LM Studio の `state` から更新する。
- `contextWindow` は **モデルの最大値**。実際のロード時コンテキストが別に取れる場合は `loadedContextWindow` を追加する。

#### settings

```rust
pub struct SettingsView {
    // 既存...
    pub local_scan_on_start: bool,     // 既定 true
    pub local_stop_on_exit: bool,      // 既定 true（アプリが起動した子プロセスだけ止める）
    pub local_auto_start: bool,        // 既定 false（provider 側の autoStart で上書き可能）
    pub local_keep_alive: Option<String>,      // 既定 None（ロード時は 5m）
    pub local_context_window: Option<u32>,     // 既定 None = 8K 上限
    pub local_prefer: bool,            // L5。ローカル優先ルーティング
}
```

### 5.3 Tauri コマンド

| コマンド | 用途 | 返り値 |
| --- | --- | --- |
| `local_runtime_snapshot` | 全ランタイムの現在状態 | `Vec<RuntimeSnapshot>` |
| `local_runtime_scan` | 既存スキャン + 状態 | `LocalScanView` |
| `local_runtime_start` | 起動 | `RuntimeSnapshot` |
| `local_runtime_stop` | 停止 | `RuntimeSnapshot` |
| `local_runtime_restart` | 再起動 | `RuntimeSnapshot` |
| `local_runtime_logs` | 直近ログ | `Vec<String>` |
| `local_runtime_open_app` | GUI アプリを開く | `()` |
| `local_model_pull` | モデル取得ジョブ開始 | `JobId` |
| `local_model_delete` | モデル削除 | `()` |
| `local_model_load` | ロード | `()` |
| `local_model_unload` | アンロード | `()` |
| `local_job_cancel` | ジョブキャンセル | `()` |
| `local_job_list` | 実行中ジョブ | `Vec<JobView>` |

既存の `scan_local_runtimes` / `probe_local_endpoint` / `import_local_runtime` は互換を保ちつつ、内部を新しい scanner / importer に委譲する。

### 5.4 イベント

```json
// local_runtime_event
{
  "runtime": "ollama",
  "state": "online",
  "previous": "starting",
  "endpoint": "http://127.0.0.1:11434",
  "version": "0.12.3",
  "error": null,
  "at": "2026-10-27T12:00:00Z"
}
```

```json
// local_job_event
{
  "jobId": "…",
  "runtime": "ollama",
  "kind": "pull",
  "model": "qwen3:8b",
  "status": "running",
  "completed": 123456789,
  "total": 4930000000,
  "message": "downloading",
  "at": "2026-10-27T12:00:00Z"
}
```

UI は `local_runtime_event` で状態バッジを、`local_job_event` で進捗バーを更新する。画面を開いた時は `local_runtime_snapshot` + `local_job_list` で補完する。

---

## 6. ランタイム別の対応

| ランタイム | 検出 | 状態 | 起動/停止 | モデル一覧 | 詳細 | pull | delete | load/unload | 備考 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Ollama | 既存 | `/api/tags` + `/api/ps` | `ollama serve` / 子プロセス停止 | `/api/tags` | `/api/show` | `/api/pull` | `/api/delete` | `/api/generate` の `keep_alive` | 本命。全機能を L3 までに |
| LM Studio | 既存 | `/api/v0/models` | `lms server start/stop` | `/api/v0/models` | 同左（state/context/quant） | `lms get` | CLI 対応時のみ | `lms load/unload` | CLI は `~/.lmstudio/bin/lms`。無ければ GUI 起動のみ |
| llama.cpp server | 既存 | `/health` | 実行ファイル + `-m` で起動 | `/v1/models` | `/props` | なし | なし | 起動/停止 = load/unload | GGUF 選択 UI が必要。L4 以降 |
| vLLM | 既存 | `/health`, `/version` | CLI 起動（引数が多い） | `/v1/models` | `/v1/models` | なし | なし | なし | L4 以降。起動は「コマンドを提示」でも可 |
| LocalAI | 既存 | `/readyz` | 限定的 | `/v1/models` | — | なし | なし | なし | 読み取り中心 |
| Jan | 既存 | アプリ起動 | `open_app` | — | — | なし | なし | なし | GUI のみ |
| KoboldCpp | 既存 | `/api/v1/model` | 実行ファイル + モデル | `/v1/models` | — | なし | なし | なし | 読み取り中心 |
| text-generation-webui | 既存 | `/v1/models` | 限定的 | `/v1/models` | — | なし | なし | なし | 読み取り中心 |
| GPT4All | 既存 | アプリ起動 | `open_app` | — | — | なし | なし | なし | GUI のみ |
| 汎用 OpenAI 互換 | 既存 | `/v1/models` | なし | `/v1/models` | — | なし | なし | なし | 手動 URL |

### 6.1 Ollama のネイティブ情報

- `GET /api/tags`: `name`, `size`, `details.parameter_size`, `details.quantization_level`
- `POST /api/show` `{"name": "..."}`: `details`, `model_info`（`*.context_length`）, `capabilities`（`completion` / `tools` / `vision` / `thinking`）
- `GET /api/ps`: ロード中モデル、`size`, `size_vram`, `expires_at`, `context_length`
- `POST /api/pull` `{"name": "...", "stream": true}`: NDJSON の進捗
- `DELETE /api/delete` `{"name": "..."}`
- `POST /api/generate` `{"model": "...", "keep_alive": "5m"}`: ロード。`keep_alive: 0` でアンロード
- `capabilities` が無い古い Ollama では、`model_info` + 実プローブへフォールバックする。

### 6.2 LM Studio のネイティブ情報

- `GET /api/v0/models`: `id`, `type`（`llm` / `vlm` / `embeddings`）, `arch`, `quantization`, `state`（`loaded` / `not-loaded` / `loading`）, `max_context_length`, `loaded_context_length`
- `lms ls --json` / `lms ps --json`: CLI 側の一覧・ロード状態
- `lms server start --port 1234` / `lms server stop`
- `lms load <model>` / `lms unload <model>` / `lms get <model>`
- 削除コマンドの有無はバージョンで異なるため、`RuntimeCapabilities.can_delete` で出し分ける。

---

## 7. 実装フェーズ

### L0 — 基盤の分割と契約（S）

**目的**: 振る舞いを変えずに、L1 以降を載せられる構造にする。

- `discovery.rs` を `local/` モジュールへ分割（`RuntimeId`, 指紋, インストール探索, パーサ）。
- `RuntimeCapabilities` / `RuntimeSnapshot` / `LocalModel` / `ModelDetails` / `RuntimeError` を定義。
- `LocalRuntimeManager` の骨格（状態の `RwLock`、イベント bus、スナップショット）を `app/src/local_runtime/` に追加。
- 既存 `scan_local_runtimes` の返り値に `state` / `capabilities` を追加（後方互換）。
- `docs/desktop/architecture.md` と `api-mapping.md` に新コマンドを追記。

**受け入れ**: 既存の検出・取り込みが回帰しない。`cargo test -p mixd-providers` / `cargo test -p mix-agent-desktop` が通る。

### L1 — メタデータと能力のネイティブ化（M）

**目的**: 「モデルの実力」を正しく表示し、誤った能力で実行しない。

- Ollama: `/api/show` を **同時実行 4 件まで**で取得。`contextWindow` / `parameterSize` / `quantization` / `family` / `vision` / `toolCalling` / `reasoning` を保存。
- LM Studio: `/api/v0/models` を優先し、`state` / `max_context_length` / `quantization` / `arch` / `type` を反映。
- llama.cpp: `/props` から `n_ctx` / モデル名を取得。
- vLLM: `/v1/models` の `max_model_len` を取得。
- `verify_tools` を実プローブに変更（最小の tool call を 1 回、`max_tokens` 小さく）。`capabilitySource` と `verifiedAt` を保存。
- スキャンは **2 段階**にする。
  - Stage 1: TCP + 指紋 + 一覧（高速、UI の初期表示）。
  - Stage 2: 詳細 enrichment（バックグラウンド、`local_runtime_event` で更新）。
- モデルの `contextWindow` が `0` のままなら、`context.rs` の圧縮閾値は安全側（固定 120k ではなく、ランタイム既定を考慮した下限）にする。

**受け入れ**:
- Ollama の 8k モデルで `contextWindow = 8192` が保存され、圧縮がその値で発火する。
- `tools` 非対応モデルにツール定義を送らない（または送っても UI が「未対応」と表示する）。
- 20 モデル保有時のスキャン初期表示が 1 秒以内、詳細は 3 秒以内に順次反映。

### L2 — ライフサイクル管理（M/L）

**目的**: アプリからランタイムを起動・停止し、状態を常に把握する。

- `locate_install` を絶対パス返却に変更。provider の `data.local.executable` に保存。
- `ProcessSupervisor`:
  - `tokio::process::Command` で起動。stdout/stderr をリングバッファへ。
  - `kill_on_drop` ではなく、明示的な stop（SIGTERM → タイムアウト → kill）。
  - アプリ終了時の扱いを設定 `local_stop_on_exit` で制御。
- 起動対象はまず **Ollama** と **LM Studio**。GUI のみのランタイムは `open_app`。
- ポート競合: 起動前に bind 確認。使用中なら「他のプロセスが使用中」と表示し、kill しない。
- `HealthMonitor`: 5 秒間隔（背景時は 30 秒）。2 回連続失敗で `Degraded`。
- UI: ランタイムカードに「起動 / 停止 / 再起動 / ログ」。初回ウィザードは「起動して続行」を出す。
- 再取り込み: 既存 provider を `(runtime, endpoint)` で見つけたら **update**。ラベル重複で失敗させない。ポートが変わったら `baseUrl` を更新する導線を出す。

**受け入れ**:
- 停止中の Ollama をアプリから起動し、そのまま最初のメッセージを送れる。
- 停止するとポートが解放される。アプリ終了で自前起動プロセスが残らない（設定に従う）。
- ポート競合時に他プロセスを殺さず、理由が UI に出る。
- ランタイムを手動で落とすと 2 回の失敗以内に `Degraded` になる。

### L3 — モデル管理（L）

**目的**: モデルの取得・削除・ロードをアプリ内で完結させる。

- Ollama:
  - `pull`: NDJSON を読み、`completed/total` を `local_job_event` で流す。キャンセルはストリーム drop。
  - `delete`: 確認ダイアログ。成功後に DB から model 行を削除。
  - `load/unload`: `/api/ps` で状態確認。`keep_alive` を設定可能に。
- LM Studio: `lms get` / `lms load` / `lms unload`（対応バージョンのみ）。`can_delete` が false なら UI に出さない。
- モデル一覧 UI:
  - サイズ、量子化、context、家族、ロード状態、能力バッジ。
  - 「取得」ダイアログ（モデル名入力 + 推奨候補）。
  - 「削除」「ロード」「アンロード」。
  - ディスク使用量の合計。
- 推奨候補は静的カタログ（例: `qwen3:8b`, `llama3.2:3b`, `gemma3:4b`, `deepseek-r1:8b`, `nomic-embed-text`）から始める。オンラインの巨大カタログは非目標。
- 取得後に `sync_models` を再実行し、既定モデルが未設定なら最初の chat モデルを設定する。

**受け入れ**:
- 1GB 超の pull で進捗が出て、キャンセルできる。
- 削除後にモデル一覧と DB が一致する。
- load/unload が `/api/ps` または `/api/v0/models` に反映される。
- 取得中にアプリを閉じても、次回起動で壊れた状態が残らない。

### L4 — UX と復旧（M）

**目的**: 「ローカルが動いている」ことをユーザーが意識しなくてよい状態にする。

- Composer にランタイム状態チップ:
  - `Ollama • qwen3:8b • オンライン`
  - `停止中 — 起動` / `モデル未ロード — ロード` / `エラー — 詳細`
- 送信時の挙動:
  - ローカル provider が停止 + `autoStart` なら、起動を待ってから送信（タイムアウト付き）。
  - 起動失敗時は送信を止め、エラーとログへの導線を出す。
  - モデル未ロードなら自動ロード（Ollama は初回リクエストで自動ロードされるため、待機表示）。
- 切断時の復旧:
  - `connection refused` を分類し、1 回だけ再起動 + 再試行。
  - それでも失敗したら Run を失敗扱いにし、会話とチェックポイントは保持。
- 取り込み時:
  - 既定モデルが未設定なら、ロード中 → chat 可能 → 名前順で 1 つ選ぶ。
  - 「このランタイムを既定にする」チェック。
- 初回ウィザード:
  - 検出 → 「起動」 → モデル一覧 → 「このモデルで始める」。
  - 未インストール時は公式のインストールページへのリンクと、コピー可能なコマンドを出す（サイレントインストールはしない）。
- 設定画面:
  - ランタイムごとの自動起動、終了時の停止、keep_alive、既定モデル。
  - ログビューア。
- 可能ならトレイメニューに「ローカルランタイム: オンライン / 停止」を追加（L4 の任意項目）。

**受け入れ**:
- 停止中の Ollama がある新規環境で、初回ウィザードから 3 クリック以内に最初のメッセージを送れる。
- ランタイムが会話中に落ちても、UI がフリーズせず、復旧導線が出る。
- 既定モデルが自動で設定され、Composer に表示される。

### L5 — ネイティブチャット経路とローカル優先（任意 / M）

**目的**: OpenAI 互換では届かない制御と、ローカル優先のルーティング。

- Ollama `/api/chat` アダプタ:
  - `options.num_ctx` を明示してコンテキスト溢れを防ぐ。
  - `keep_alive` をリクエスト単位で指定。
  - `think` / thinking のストリームを既存の `ReasoningDelta` に変換。
  - ツール呼び出しの変換（既存の OpenAI 互換との差分をテストで固定）。
- ローカル優先ルーティング:
  - `local_prefer` が有効で、ローカルがオンライン・能力一致・コンテキスト充足ならローカルを選ぶ。
  - 失敗時のみクラウドへフォールバック（設定で許可した場合）。
  - Auto の候補に「ローカルは無料」として価格 0 を反映（既存 `pricing` の挙動を確認）。
- ハードウェア目安:
  - 総 RAM / 空きディスク / （取得できれば）VRAM から、推奨モデルサイズを表示。
  - `sysinfo` 等の追加依存は、MSRV とバイナリサイズを確認してから決める。
- 速度表示: 初回トークンまでの時間と tokens/s を Run に記録（ネイティブ `/api/chat` の `eval_count` / `eval_duration` を利用）。

**受け入れ**:
- 8k モデルで `num_ctx=8192` が実際に使われ、長い会話でサーバ側の切り捨てが起きない。
- ローカル優先で開始し、失敗時のみクラウドに切り替わる。
- 速度とロード時間が診断画面で見える。

---

## 8. UX フロー

### 8.1 初回（インストール済み・停止中）

```
ようこそ
  → この PC で動かす
  → [検出] Ollama が見つかりました（停止中）
  → [起動] 起動中… → オンライン
  → モデル一覧（量子化 / サイズ / context / tools）
  → [このモデルで始める]
  → チャット（既定モデル設定済み）
```

### 8.2 既存ユーザー（起動時に自動検出）

- アプリ起動後、バックグラウンドでスキャン。
- 既存のローカル provider が停止 + `autoStart` なら自動起動。
- 失敗した場合だけ通知。成功時は静かに `Online` にする。

### 8.3 モデル取得

```
設定 → ローカル → Ollama → モデル
  → [モデルを取得]
  → 候補を選ぶ / 名前を入力
  → 進捗バー（残りサイズ / 速度 / キャンセル）
  → 完了 → 一覧に追加 → [ロード] → 既定に設定
```

### 8.4 会話中の切断

- Composer に `接続が切れました — 再起動` を表示。
- クリックで再起動 → 成功したら「再試行」で同じメッセージを送る。
- Run は失敗として保存し、チェックポイントから再開できる。

---

## 9. セキュリティ

| リスク | 対策 |
| --- | --- |
| 任意コマンド実行 | 検出済みの絶対パスのみ。`Command::new(path).args([...])`。シェル禁止。 |
| 任意プロセスの kill | アプリが起動した子プロセスの PID のみ停止。ポートから PID を引かない。 |
| ポート競合の誤認 | 起動前に bind 確認。使用中は起動せず、理由を表示。 |
| パストラバーサル | モデル名をベンダー規則で検証。`..`、`/` 始まり、ドライブレターを拒否。 |
| LAN への露出 | 自動スキャンは loopback のみ。LAN は手動 + 明示許可。 |
| ログ漏えい | API キー・プロンプト・ファイル内容をログに書かない。必要なら redaction。 |
| 悪意あるモデル名 / メタデータ | UI でエスケープ。モデル名をコマンド引数以外に使わない。 |
| アプリ終了後の孤児 | `local_stop_on_exit` の既定を定義し、起動時に自前プロセスを識別して掃除する。 |
| 署名・配布 | 既存の方針（ad-hoc 署名、公証なし）を維持。ローカル実行に追加の権限は要求しない。 |

---

## 10. テスト計画

### 10.1 Rust

- `mixd-providers`:
  - Ollama の `/api/tags` / `/api/show` / `/api/ps` / pull NDJSON のパース。
  - LM Studio `/api/v0/models` の state / context / quant / type。
  - llama.cpp `/props`、vLLM `/v1/models`。
  - `join_api`、loopback 判定、モデル名検証。
  - 能力マトリクス（delete 不可のランタイムで UI 用フラグが false）。
- `mix-agent-desktop`:
  - 状態機械（Stopped → Starting → Online → Degraded → Stopped、Error 経路）。
  - プロセス supervisor（fake runtime スクリプトで起動・停止・ログ・ポート競合）。
  - ジョブ（pull の進捗・キャンセル・失敗）。
  - `verify_tools` の実プローブと `capabilitySource`。
  - provider 再取り込み（同一 endpoint の update、ポート変更）。
- HTTP モックは `wiremock` などの dev-dependency を追加するか、`tokio::net::TcpListener` の小さな fixture で代替する（依存追加は最小化）。

### 10.2 UI

- `LocalRuntimeScan` の状態: 未検出 / 停止中 / 起動中 / オンライン / エラー。
- 進捗バー、キャンセル、削除確認、ロード/アンロード。
- Composer の状態チップと復旧導線。
- 初回ウィザードの「起動して続行」。
- i18n（ja/en）の追加文言。

### 10.3 手動検証

- macOS / Windows / Linux の各実機。
- Ollama（停止 → 起動 → pull → load → chat → unload → stop）。
- LM Studio（GUI 起動 / `lms` あり・なし）。
- llama.cpp server（GGUF 指定起動）。
- ポート競合（別プロセスが 11434 を占有）。
- アプリ終了・強制終了後の孤児プロセス。
- ネットワーク切断・ランタイム再起動中の Run。
- 既存ユーザーの provider がポート変更で再接続できるか。

### 10.4 CI

- `pnpm rust:fmt:check`
- `pnpm rust:clippy -- -D warnings`
- `cargo test --workspace`
- `pnpm ui:test`
- `pnpm ui:lint`
- `pnpm ui:typecheck`
- `docs/verification.md` に実機結果を追記（未実施を「確認済み」と書かない）。

---

## 11. 受け入れ基準（Definition of Done）

1. **ゼロ設定に近い初回体験**: Ollama インストール済み・停止中から、検出 → 起動 → モデル選択 → 最初のメッセージまで 3 クリック以内。API キー不要。
2. **正しいメタデータ**: 8k モデルで `contextWindow` が 8192 になり、圧縮がその値で発火する。量子化・サイズ・family が表示される。
3. **正しい能力**: tools / vision / reasoning のバッジが、ネイティブ情報または実プローブと一致する。未対応モデルにツールを送らない。
4. **ライフサイクル**: 起動・停止・再起動ができ、状態が 2 秒以内に UI に反映される。ポート競合で他プロセスを殺さない。アプリ終了で自前プロセスが残らない（設定に従う）。
5. **モデル管理**: pull の進捗とキャンセル、delete、load/unload が動作し、DB と UI が一致する。
6. **復旧**: 会話中にランタイムが落ちてもフリーズせず、再起動 → 再試行の導線が出る。
7. **回帰なし**: 既存のクラウド provider、Auto、Memory embedding、初回ウィザード、Docker 版に影響しない。
8. **品質ゲート**: fmt / clippy / Rust tests / UI tests / lint / typecheck がすべて成功する。

---

## 12. リスクと対策

| リスク | 対策 |
| --- | --- |
| ベンダー API のバージョン差 | capability 検出 + バージョン取得 + フォールバック。未知のフィールドは無視。 |
| LM Studio CLI のコマンド差 | `lms version` と `--help` で機能を判定。未対応機能は UI に出さない。 |
| プロセス停止の OS 差 | SIGTERM → タイムアウト → kill。Windows は Job Object / `taskkill` を検討。実機テスト必須。 |
| スキャンが重い | 2 段階 + 同時実行制限 + キャッシュ（60 秒）。 |
| `/api/show` の N+1 | 同時 4 件、モデル数上限、Stage 2 で非同期。 |
| コンテキストの誤表示 | 「モデル最大」と「ロード時」を区別。L5 で `num_ctx` を明示。 |
| 既定モデルの勝手な変更 | 既存の `default_model_id` がある場合は触らない。取り込み時のみ未設定なら設定。 |
| セキュリティ | 絶対パス・引数配列・所有プロセスのみ停止・loopback 既定。 |
| UI 肥大化 | 既存 `Card` / `DataTable` / `Badge` を再利用。新規依存を増やさない。 |
| Docker 版との差 | Desktop 固有機能として `docs/desktop/` に閉じる。Server 版は変更しない。 |

---

## 13. 実装順序と目安

```
L0 (基盤)
  └─▶ L1 (メタデータ / 能力)
        ├─▶ L2 (ライフサイクル)
        │     └─▶ L3 (モデル管理)
        │           └─▶ L4 (UX / 復旧)
        └─────────────────▶ L5 (任意: ネイティブチャット / ローカル優先)
```

| フェーズ | 目安 | 主な成果物 |
| --- | --- | --- |
| L0 | 2〜3 日 | モジュール分割、型、マネージャ骨格、イベント |
| L1 | 4〜5 日 | `/api/show`、`/api/v0/models`、能力プローブ、2 段階スキャン |
| L2 | 5〜7 日 | 起動・停止・再起動、状態監視、ログ、自動起動 |
| L3 | 5〜7 日 | pull / delete / load / unload、進捗、モデル UI |
| L4 | 3〜4 日 | Composer 状態、復旧、初回ウィザード、既定モデル |
| L5 | 4〜6 日 | Ollama ネイティブチャット、ローカル優先、速度表示 |

1 人で進める場合の合計目安は約 4〜5 週間。L1 + L2 だけでも「ネイティブ感」の大半が出るため、まずそこまでを 1 つのリリース単位にするのがよい。

---

## 14. 着手時の検討メモ

実装時に決めた内容は [`architecture.md`](architecture.md) の「Local runtime management」に反映済み。以下は着手時点の検討メモとして残す。

1. **自動起動の既定**: アプリ起動時に、既存のローカル provider を自動で起動するか（推奨: する。失敗時のみ通知）。
2. **終了時の扱い**: アプリが起動したランタイムを終了時に止めるか（推奨: 止める。外部起動のものは止めず、設定で「バックグラウンド維持」も選べる）。
3. **LM Studio の扱い**: `lms` CLI が無い環境では GUI 起動のみとし、pull/delete は出さない（推奨）。
4. **L5 の時期**: まず L1〜L4 を出し、`num_ctx` 問題が実運用で出てから L5 に着手する（推奨）。
5. **ハードウェア判定**: 追加依存を許容するか。まずは RAM とディスクだけの簡易表示でもよい。

---

## 15. 参照

- 既存検出: `apps/desktop/providers/src/discovery.rs`
- 既存コマンド: `apps/desktop/app/src/commands/discovery.rs`
- モデル同期: `apps/desktop/app/src/commands/models.rs`
- provider 登録: `apps/desktop/app/src/commands/providers.rs`
- 設定: `apps/desktop/app/src/commands/settings.rs`
- 埋め込み: `apps/desktop/app/src/embedding.rs`
- コンテキスト: `apps/desktop/core/src/context.rs`
- UI: `apps/desktop/ui/src/features/providers/LocalRuntimeScan.tsx`, `LocalDiscoveryCard.tsx`
- 初回: `apps/desktop/ui/src/features/onboarding/StepProvider.tsx`
- API 対応表: `docs/desktop/api-mapping.md`
- アーキテクチャ: `docs/desktop/architecture.md`
