# MIX-agent — ChatGPT / Codex 使用枠 対応計画

> 状態: **Docker 版 実装済み（Phase 1 縦切り） / Desktop 版 未着手**
> 対象: Desktop 版（`apps/desktop`）と Docker 版（`apps/server` + `apps/web`）
> 基準: 2026-11 時点の OpenAI Codex CLI `main` の実装を外部契約の正とする

## 1. 目的と確定事項

ChatGPT Plus / Pro などの **Codex 使用枠（サブスクリプション枠）** を、MIX-agent の
Provider として推論に使い、同時に残量（5 時間枠・週次枠・クレジット）を UI に表示する。

確定したスコープ:

| 項目 | 決定 |
| :--- | :--- |
| エディション | Desktop 版・Docker 版の両方 |
| 機能 | ① 契約枠での推論 ② 残量・リセット時刻の表示 |
| 認証 | MIX 内で新規 OAuth ログイン（`~/.codex/auth.json` の取り込みはしない） |
| 既存方式 | API キー Provider は残し、ChatGPT 枠は追加の認証方式として併存させる |
| 課金表示 | サブスク枠の Run は従量課金 $0 として記録し、統計のコストを汚さない |

非目標（v1 ではやらない）:

- `codex exec` / Codex CLI の子プロセス委譲
- 既存 Codex ログインのインポート、複数アカウントの同時保持
- ワークスペース管理、レートリミットリセットクレジットの消費
- 画像生成、Realtime、Responses WebSocket、Codex のメモリ/コンパクション機能

> **重要（非公式・自己責任）**: 本対応は OpenAI の公式 API ではなく、Codex CLI が
> 使う非公開バックエンドを利用する **非公式な連携** である。OpenAI の利用規約・
> アカウント制限に抵触する可能性、仕様変更・遮断・予告なき停止のリスクがある。
> 利用は **自己責任** とし、MIX-agent は動作・継続利用・アカウント状態を保証しない。
> そのため、明示的なオプトイン（初回同意チェック + 設定で有効化）と、
> API キー方式へのフォールバックを必須にする。既定では無効にする。

## 2. 外部契約（Codex CLI 準拠）

実装時に `openai/codex` の以下をフィクスチャとして固定し、変更検知できるようにする。

### 2.1 OAuth

| 用途 | 値 |
| :--- | :--- |
| Issuer | `https://auth.openai.com` |
| Client ID | `app_EMoamEEZ73f0CkXaXp7hrann` |
| Scope | `openid profile email offline_access` |
| 認可 | `GET /oauth/authorize`（PKCE S256、`state` 必須） |
| トークン交換 | `POST /oauth/token`（authorization_code は form、refresh は JSON） |
| 失効 | `POST /oauth/revoke` |
| ブラウザコールバック | `http://127.0.0.1:1455/auth/callback`（使用中なら 1457） |
| デバイスコード | `POST /api/accounts/deviceauth/usercode` → `POST /api/accounts/deviceauth/token` |
| デバイス認証 URL | `https://auth.openai.com/codex/device` |

認可 URL には Codex と同じ追加パラメータを付ける:
`id_token_add_organizations=true`, `codex_cli_simplified_flow=true`, `originator=<値>`。

ID トークンの `https://api.openai.com/auth` クレームから
`chatgpt_account_id` / `chatgpt_plan_type` / `chatgpt_user_id` を読む。
メールアドレスはトップレベル `email`（なければ `profile.email`）。

### 2.2 推論（Responses API）

| 用途 | 値 |
| :--- | :--- |
| Base URL | `https://chatgpt.com/backend-api/codex` |
| 推論 | `POST /responses` |
| モデル一覧 | `GET /models?client_version=<MIX のバージョン>` |
| 使用量 | `GET https://chatgpt.com/backend-api/wham/usage` |

必須ヘッダ:

```http
Authorization: Bearer <access_token>
chatgpt-account-id: <account_id>
originator: <originator>
session-id: <run または会話の安定 ID>
thread-id: <同上>
Accept: text/event-stream
Content-Type: application/json
```

リクエストの要点:

```json
{
  "model": "gpt-5.x-codex",
  "instructions": "...",
  "input": [],
  "tools": [{ "type": "function", "name": "...", "description": "...", "parameters": {} }],
  "tool_choice": "auto",
  "parallel_tool_calls": true,
  "reasoning": { "effort": "medium", "summary": "auto" },
  "store": false,
  "stream": true,
  "include": ["reasoning.encrypted_content"],
  "prompt_cache_key": "<run id>"
}
```

`/models` のレスポンスは `{ "models": [{ "slug", "display_name", "context_window", ... }] }`。
OpenAI 互換の `{ "data": [{ "id" }] }` ではないため、専用パーサーが必要。

### 2.3 残量（クォータ）

`GET /backend-api/wham/usage` のレスポンス:

```json
{
  "plan_type": "plus",
  "rate_limit": {
    "allowed": true,
    "limit_reached": false,
    "primary_window": {
      "used_percent": 42,
      "limit_window_seconds": 18000,
      "reset_after_seconds": 3600,
      "reset_at": 1767225600
    },
    "secondary_window": {
      "used_percent": 12,
      "limit_window_seconds": 604800,
      "reset_at": 1767800000
    }
  },
  "credits": { "has_credits": true, "unlimited": false, "balance": "..." },
  "additional_rate_limits": []
}
```

推論レスポンスのヘッダでも同じ情報が返る。5 時間枠・週次枠の即時更新に使う。

- `x-codex-primary-used-percent` / `-window-minutes` / `-reset-at`
- `x-codex-secondary-used-percent` / `-window-minutes` / `-reset-at`
- `x-codex-credits-has-credits` / `-unlimited` / `-balance`
- SSE の `codex.rate_limits` イベント（`used_percent` / `window_minutes` / `reset_at`）

## 3. 共通設計

### 3.1 認証情報モデル

トークンは DB に置かず、両エディションの既存暗号化ストアに JSON で保存する。

```json
{
  "access_token": "...",
  "refresh_token": "...",
  "id_token": "...",
  "account_id": "...",
  "email": "user@example.com",
  "plan_type": "plus",
  "expires_at": 1767225600,
  "last_refresh": "2026-11-01T12:00:00Z"
}
```

- Desktop: `SecretStore` の `provider/<id>` / `oauth` スロット
- Docker: 既存 `Secret` 行（AES-GCM）に JSON 文字列。`secret_id` は従来どおり
- Provider 行にはトークンを書かない。`auth_mode` / `account_id`（マスク表示用）/
  `plan_type` / `billing_mode` だけを持つ

### 3.2 トークン更新

1. リクエスト前に `expires_at - 5 分` を切っていたら refresh
2. `401` を受けたら **1 回だけ** refresh して再試行
3. refresh は Provider ごとの `Mutex` で直列化（同時 Run での連打を防ぐ）
4. `invalid_grant` / `refresh_token_expired|reused|invalidated` は恒久エラーとして
   「再ログインが必要」と UI に返す。トークンは残さない
5. refresh 成功時は必ず保存してからメモリを更新する

### 3.3 クォータモデル

両エディションで同じ正規化型を使う。

```text
QuotaSnapshot {
  plan_type: Option<String>
  captured_at: DateTime
  windows: [
    { id: "5h",  used_percent: f64, window_minutes: i64, resets_at: DateTime }
    { id: "week", used_percent: f64, window_minutes: i64, resets_at: DateTime }
  ]
  credits: Option<{ has_credits, unlimited, balance }>
  limit_reached: bool
}
```

- 取得は 60 秒キャッシュ。UI の「更新」で強制再取得
- 推論ヘッダ / SSE イベントが来たらキャッシュを上書き
- 15 分以上古い場合は UI に「古い情報」と表示（Codex の stale 閾値に合わせる）
- `/wham/usage` が失敗しても推論は止めない。ヘッダ由来のキャッシュで表示を継続

### 3.4 セキュリティ

- PKCE S256、`state` の照合、コールバックは `127.0.0.1` のみにバインド
- ログアウト時は `/oauth/revoke` を呼び、成功可否に関わらずローカル削除
- トークン・ID トークン・認可コードをログ、エラー本文、診断画面、バックアップに出さない
- Docker の OpenAPI / 生成型にトークンを含めない
- 設定で「ChatGPT 連携（実験的）」を明示的に ON にした場合だけプリセットを表示

## 4. Desktop 版の実装

### 4.1 現状のギャップ

- `apps/desktop/providers/src/openai.rs` は `chat/completions` のみ
- `openai_compat_wire::WireApi::Responses` は未実装のスタブ
- `ProviderKind` に ChatGPT がない
- `ProviderRequest` に Responses 用の `session_id` / `prompt_cache_key` がない
- アシスタントターンの reasoning アイテムを保持する場所がない

### 4.2 追加・変更するファイル

| ファイル | 変更 |
| :--- | :--- |
| `providers/src/responses.rs` | 新規。Responses API の共通 SSE アダプター |
| `providers/src/chatgpt.rs` | 新規。ChatGPT 認証・ヘッダ・クォータ・モデル一覧 |
| `providers/src/lib.rs` | `ProviderKind::ChatGpt`、`QuotaSnapshot`、`Message.native`、`StreamEvent::NativeOutput`、`Provider::quota()` |
| `providers/src/preset/mod.rs` | `chatgpt` プリセット（category「サブスクリプション」） |
| `app/src/commands/chatgpt_auth.rs` | 新規。PKCE ログイン、デバイスコード、ログアウト、クォータ |
| `app/src/commands/providers.rs` | 認証方式・アカウント表示、ChatGPT 用モデル取得、削除時のトークン削除 |
| `app/src/lib.rs` | `build_provider` / `register_adapter` の ChatGPT 分岐、コマンド登録 |
| `core/src/runs/engine/execute.rs` | アシスタントメッセージへ native アイテムを付与 |
| `core/src/runs/engine/execute_turn.rs` | `TurnResult.native_items`、`ProviderRequest.session_id` |
| `ui/src/features/providers/ProvidersPane.tsx` | ログインカード、アカウント・プラン表示 |
| `ui/src/features/chat/UsagePanel.tsx` | 5 時間 / 週次バー、リセット時刻、クレジット |
| `ui/src/lib/i18n/en.ts` | 新規文言（日本語キー → 英語） |

### 4.3 Responses アダプター

`responses.rs` は ChatGPT 専用にせず、**公式 OpenAI Responses API でも再利用できる**
汎用アダプターにする。ChatGPT 固有部分（認証・ヘッダ・Base URL）は `chatgpt.rs` に分離する。

メッセージ変換:

| MIX | Responses |
| :--- | :--- |
| `ProviderRequest.system` | `instructions` |
| `user` | `{ role: "user", content: [{ type: "input_text", text }] }` |
| `assistant`（native なし） | `{ role: "assistant", content: [{ type: "output_text", text }] }` + `function_call` |
| `tool` | `{ type: "function_call_output", call_id, output }` |
| `assistant.native`（あり） | 保存済みの reasoning / message / function_call をそのまま展開 |
| `ToolSpec` | `{ type: "function", name, description, parameters }` |
| `ReasoningControl` | `reasoning.effort`（low / medium / high / xhigh） |

SSE イベント対応:

| イベント | `StreamEvent` |
| :--- | :--- |
| `response.output_text.delta` | `TextDelta` |
| `response.reasoning_summary_text.delta` | `ReasoningDelta` |
| `response.output_item.added`（function_call） | `ToolCallStart` |
| `response.function_call_arguments.delta` | `ToolCallArgumentDelta` |
| `response.output_item.done`（function_call） | `ToolCallFinish` |
| `response.completed` | `NativeOutput(response.output)` → `Finished` |
| `response.failed` / `response.incomplete` / `error` | `Error` |
| `codex.rate_limits` | クォータキャッシュ更新（UI イベントなし） |

ツールループの継続性:

- `store: false` のため、reasoning を次ターンへ返さないと推論の連続性が落ちる
- `Message` に `native: Vec<serde_json::Value>` を追加し、`response.output` を保持
- `TurnResult` に `native_items` を追加し、ツール実行後の assistant メッセージへ付与
- 会話履歴の JSON にも保存（`#[serde(default)]` で後方互換）
- 他アダプターは `native` を無視する

その他:

- `ProviderRequest.session_id` を追加し、`run_id` を `prompt_cache_key` /
  `session-id` / `thread-id` に使う
- `max_tokens` は `max_output_tokens` にマップ
- `temperature` / `top_p` は ChatGPT バックエンドが拒否する場合があるため送らない
  （プリセット側で無効化）
- ストリーム全体は既存の `transport::with_stream_deadline` で上限を設ける

### 4.4 OAuth（Desktop）

`app/src/commands/chatgpt_auth.rs` に Tauri コマンドを追加:

| コマンド | 内容 |
| :--- | :--- |
| `chatgpt_login_start(provider_id)` | PKCE 生成、`127.0.0.1:1455` で待受、ブラウザを開く。`login_id` を返す |
| `chatgpt_login_status(login_id)` | `pending` / `complete` / `error`。完了時にトークンを保存 |
| `chatgpt_login_cancel(login_id)` | 待受を閉じる |
| `chatgpt_login_device_start(provider_id)` | デバイスコード開始（ポート不可時のフォールバック） |
| `chatgpt_login_device_poll(login_id)` | 認可完了をポーリング |
| `chatgpt_logout(provider_id)` | revoke + ローカル削除 |
| `chatgpt_quota(provider_id)` | キャッシュまたは `/wham/usage` を返す |

実装メモ:

- `code_verifier` は 32 バイト乱数の base64url、`code_challenge` は SHA-256
- `state` は 32 バイト乱数の base64url。コールバックで不一致なら 400
- コールバック成功時は「MIX に戻ってください」の最小 HTML を返して閉じる
- ポート 1455 が使用中なら 1457。両方不可ならデバイスコードへ誘導
- 既存の `open` クレートでブラウザを開く（追加依存なし）
- JWT の署名検証はしない（表示・account_id 抽出のみ。API 側で認証される）

### 4.5 Provider 登録と UI

- プリセット `chatgpt` を追加（`kind: "chatgpt"`, `api_key_required: false`）
- `build_provider` は `ChatGptProvider::new(base_url, ChatGptAuth)` を返す
- `register_adapter` は `chatgpt` のとき `api_key` ではなく `oauth` スロットを読む
- `ProviderView` に `auth_mode` / `account_email` / `plan_type` / `billing_mode` を追加
- `create_provider` 直後にログインを開始し、成功後に `connect_provider` でモデル取得
- 未認証のままなら「ChatGPT でログイン」ボタンを出す（再ログインも同じ）
- `delete_provider` は `api_key` と `oauth` の両スロットを削除
- `update_provider` は API キーが空でも OAuth トークンを消さない

UI:

- ProvidersPane: プリセット選択時に API キー欄を隠し、ログインカードを表示
  - アカウントのメール（マスク可）、プラン、最終更新、ログアウト
  - 未認証・期限切れ・失効の 3 状態を明示
- UsagePanel: 5 時間枠と週次枠のバー、使用率、リセット時刻、クレジット残高
  - サブスク枠のときはコスト欄を「サブスクリプション（従量課金なし）」に置換
  - `refetchInterval: 60_000`、手動更新ボタン、15 分超で stale 表示
- ModelPicker: ChatGPT のモデルには「ChatGPT 枠」バッジ（任意）

### 4.6 課金・統計

- Provider 行に `billing_mode: "subscription"` を持たせる
- `core/src/pricing.rs` のコスト計算で subscription は $0 を返す
- トークン数は従来どおり記録（コンテキスト計測・利用傾向のため）
- Auto ルーティングのコスト評価でも $0 として扱う

## 5. Docker 版の実装

### 5.1 カタログとトランスポート

- `apps/server/mix_agent/providers/catalog.py`
  - `PRESETS` に `chatgpt` を追加（`kind: "openai"`, `transport_id: "chatgpt_responses"`,
    `discovery_id: "chatgpt_models"`, `api_key=False`, 既定 URL は Codex バックエンド）
  - 件数アサート（現在 52）を更新
  - `TRANSPORT_IDS` に `chatgpt_responses` を追加
- `apps/server/mix_agent/providers/adapters.py`
  - `_chatgpt` トランスポートを追加。既存 `_openai` のリクエスト組み立てを共通関数へ抽出
  - `AsyncOpenAI(api_key=access_token, base_url=CHATGPT_CODEX_BASE_URL,
    default_headers={chatgpt-account-id, originator, session-id})`
  - `list_models` に `chatgpt_models` 分岐（`/models?client_version=`、`{models:[{slug}]}`）
  - `quota()` を追加（`/wham/usage` の正規化 + 60 秒キャッシュ）
  - 401 時に 1 回 refresh して再試行
  - 作成直後の自動モデル同期は `chatgpt` ではスキップする（未認証で必ず失敗するため）。
    ログイン完了後に明示的に `sync-models` を呼ぶ
- `apps/server/mix_agent/providers/chatgpt.py`（新規）
  - `ChatGptCredentials`、`exchange_code`、`refresh`、`revoke`、
    `request_device_code`、`poll_device_token`、`fetch_usage`、`parse_id_token`

### 5.2 認証情報の受け渡し

`Adapter(provider, key)` は従来どおり秘密文字列を受け取る。`preset_id == "chatgpt"`
のときだけ `ChatGptAuth` を内部で構築し、JSON をパースする。refresh は
`ChatGptAuth` が `SessionLocal` を新規に開き、`secret_id` の行だけを更新する
（実行中の DB セッションを壊さない）。`401/403` は `_chatgpt_responses` が
1 回だけ refresh して再試行する。呼び出し側 5 箇所のシグネチャは変えない。

これにより、`engine_provider.py` / `engine_context.py` / `memory/jobs.py` /
`memory/embeddings.py` / `routes_providers.py` / `providers/probing.py` の
既存の `Adapter(provider, key)` がそのまま ChatGPT に対応する。

### 5.3 デバイスコード API

Docker 版はサーバーが遠隔にあるため、`localhost` コールバックは使えない。
**デバイスコードを正式ルート**にする。

| エンドポイント | 内容 |
| :--- | :--- |
| `POST /providers/chatgpt/device/start` | `{provider_id}` → `{login_id, verification_url, user_code, expires_in}` |
| `GET /providers/chatgpt/device/{login_id}` | `{status: pending|complete|error, error?}` |
| `POST /providers/chatgpt/device/{login_id}/cancel` | 中止 |
| `POST /providers/{key}/logout` | revoke + Secret 削除 |
| `GET /providers/{key}/quota` | `QuotaSnapshot`（60 秒キャッシュ） |
| `POST /providers/{key}/quota/refresh` | 強制再取得 |

- ログイン状態はプロセス内の TTL 付き辞書（15 分）。uvicorn は `--workers 1` 前提
- 再起動で進行中のログインは失効する（ユーザーはやり直す）
- 完了時に `providers.data.auth` を更新し、Secret に認証 JSON を保存
- 監査ログ `provider.chatgpt_login` を残す（トークンは含めない）

### 5.4 Web UI

- `apps/web/src/features/providers/Providers.tsx`
  - `chatgpt` プリセット選択時に API キー欄を隠す
  - 「ChatGPT でログイン」→ デバイスコードのモーダル
    - 認証 URL、ワンタイムコード、コピー、ポーリング、キャンセル
  - アカウント・プラン・最終更新・ログアウト
  - クォータカード（5 時間 / 週次 / クレジット、リセット時刻）
- `apps/web/src/lib/queries.ts`（または相当）に `useChatGptQuota` を追加
- `apps/web/src/generated/api.ts` は `scripts/export-openapi.py` →
  `pnpm generate:api` で再生成
- 日本語・英語の文言を追加

### 5.5 統計・課金

- `apps/server/mix_agent/usage.py` の `pricing.resolve` 前に
  `provider.data.billing_mode == "subscription"` を確認し、コスト 0 を記録
- 統計画面では「サブスク枠」バッジを付け、USD だけの比較から除外できるようにする

## 6. データモデル

### Desktop

| 保存先 | 内容 |
| :--- | :--- |
| `providers.data` | `kind: "chatgpt"`, `preset: "chatgpt"`, `baseUrl`, `auth: {account_id, email, plan_type}`, `billing_mode: "subscription"` |
| `SecretStore` `provider/<id>` / `oauth` | 認証 JSON（暗号化） |
| `SecretStore` `provider/<id>` / `api_key` | 使わない（既存 Provider との互換のため残す） |
| メモリ | `QuotaSnapshot`（60 秒キャッシュ） |

SQLite のマイグレーションは不要（`data` は JSON）。`Message.native` は
会話履歴 JSON に追加されるが、既存行は `default` で読める。

### Docker

| 保存先 | 内容 |
| :--- | :--- |
| `providers.data` | `preset_id: "chatgpt"`, `kind: "openai"`, `base_url`, `auth: {account_id, email, plan_type}`, `billing_mode: "subscription"` |
| `providers.data.secret_id` → `secrets.data` | 認証 JSON（AES-GCM） |
| プロセス内 | デバイスコード状態（15 分）、`QuotaSnapshot`（60 秒） |

DB マイグレーションは不要。

## 7. 実装順序と見積もり

| Phase | 内容 | 目安 |
| :--- | :--- | ---: |
| 0 | 契約の固定: 実アカウントでデバイスコード → `/responses` → `/wham/usage` をスパイク。SSE・モデル一覧・ヘッダをサニタイズしてフィクスチャ化 | 1 日 |
| 1 | Docker 縦切り: `chatgpt_responses` + 認証 JSON + refresh + モデル取得 + クォータ。API で推論成功まで | 2〜3 日 |
| 2 | Desktop Responses アダプター: `responses.rs` + native リプレイ + ツールループ + 単体テスト | 2〜3 日 |
| 3 | Desktop OAuth + Provider 登録: PKCE、Keychain、401 再試行、デバイスコード fallback | 2 日 |
| 4 | Desktop UI: ProvidersPane、UsagePanel、i18n、$0 課金表示 | 1〜2 日 |
| 5 | Docker OAuth + Web UI: デバイスコード API、モーダル、クォータカード、OpenAPI 再生成 | 2 日 |
| 6 | 仕上げ: 両エディションの受け入れ試験、ドキュメント、リリースノート、障害時の縮退 | 1〜2 日 |

合計 **11〜14 日**。Phase 1 と Phase 2 は Phase 0 の契約固定後に並行可能。
Phase 1 を先に置くのは、既存の OpenAI SDK 経由で外部 API の挙動を最速で
検証できるため。Desktop は Responses 実装が重いので、契約が固まってから着手する。

## 8. 受け入れ条件

1. Codex CLI を入れていない環境で、MIX の UI から ChatGPT ログインが完了する
2. ログイン後、モデル一覧が取得でき、チャットとツール呼び出しが動く
3. ツール実行を挟むマルチターンで、推論が継続する（native リプレイ）
4. 5 時間枠・週次枠・リセット時刻・プラン・クレジットが表示される
5. 推論後にクォータ表示が更新される（ヘッダ / SSE）
6. MIX 再起動後もログインが維持され、access token が期限切れなら自動更新される
7. トークン失効時は「再ログインが必要」と表示され、無限リトライしない
8. ログアウトで revoke とローカル削除が行われる
9. トークンが DB・ログ・バックアップ・OpenAPI・診断画面に出ない
10. サブスク枠の Run が統計上 $0 で記録される
11. Docker 版は遠隔ホストでもデバイスコードでログインできる
12. API キー Provider の既存挙動が変わらない

## 9. リスクと対策

| リスク | 対策 |
| :--- | :--- |
| 非公開 API の仕様変更・遮断 | Phase 0 のフィクスチャで変更検知。失敗時は API キーへ誘導。設定で無効化可能 |
| 利用規約・アカウント制限 | 実験的機能として明示。既定 OFF。初回に注意書き。`originator` の扱いは実測して決定 |
| ChatGPT バックエンドが第三者クライアントを拒否 | `originator` を MIX 固定で試験。拒否される場合は Codex 互換値の利用可否を法務・規約面も含めて判断し、無理なら API キー方式のみに縮退 |
| Responses のツールループが不安定 | native リプレイを必須にし、記録 SSE で回帰テスト。`store` の可否も実測 |
| モデル一覧が取れない | 既知モデルの静的フォールバック（`gpt-5.x-codex` 系）を同梱 |
| Docker の遠隔 OAuth | デバイスコードを正式ルートに。コールバック前提にしない |
| クォータ API のレート制限 | 60 秒キャッシュ、手動更新、ヘッダ優先 |
| トークン漏洩 | 既存の暗号化ストア、ログ redaction、診断画面はメタ情報のみ、テストで検査 |
| リフレッシュ競合 | Provider 単位の Mutex、保存してからメモリ更新、401 は 1 回だけ |
| 既存 Provider への回帰 | `ProviderKind::ChatGpt` を追加するだけで既存分岐は変えない。全 Provider のテストを維持 |

## 10. 実装時に実測して固定する項目

製品判断ではなく、外部仕様の確認項目:

- `originator` に MIX 独自値を送って受理されるか
- `OpenAI-Beta: responses=experimental` が必要か（現行 CLI の HTTP リクエストを正とする）
- `store: true` が許容されるか（許容なら native リプレイを簡略化できる）
- ChatGPT バックエンドが `temperature` / `top_p` を受け付けるか
- `/models` の `client_version` に何を入れるか（MIX のバージョンでよいか）
- access token の実際の TTL と refresh の推奨 skew
- 5 時間 / 週次の window 秒数がプランごとに変わるか

## 11. テスト計画

### 共通

- 認証 JSON の暗号化・復号・後方互換
- ID トークンのクレーム抽出（account_id / plan / email）
- 401 → refresh → 再試行、恒久エラー → 再ログイン
- クォータ正規化（5h / week / credits / stale）
- トークンがログに出ないこと（redaction テスト）

### Desktop

- `providers` クレート: SSE イベント変換、ツール呼び出し、usage、native リプレイ
- `core` クレート: ツールループ 2 ターンで reasoning が保持されること
- Tauri コマンド: PKCE 生成、state 不一致、ポート fallback、ログアウト
- UI: ProvidersPane の 3 状態、UsagePanel のバーと stale 表示
- 既存の `cargo test --workspace` / `clippy -D warnings` / `ui:test` / `ui:typecheck`

### Docker

- `pytest`: デバイスコード開始 → ポーリング → トークン交換 → Provider 更新
- `respx` 等で `/responses` と `/wham/usage` をモック
- モデル一覧の `{models:[{slug}]}` パース
- 401 再試行、refresh 失敗時の分類
- `ruff` / `pyright` / OpenAPI 再生成の差分なし

### 手動（リリース前）

- 実アカウントで Desktop / Docker の両方
- 5 時間枠を使い切る直前の表示、リセット後の表示
- ネットワーク断、トークン失効、アプリ再起動
- スクリーンリーダーでクォータカードが読めること

## 12. ドキュメント更新

実装完了時に以下を更新する（この計画書は経緯として残す）。

- `docs/desktop/architecture.md`: Provider 認証方式、ChatGPT 連携、クォータ
- `docs/architecture.md`: Docker 版の Provider 認証とクォータ
- `docs/desktop/api-mapping.md`: 新しい Tauri コマンドと REST の対応
- `docs/openapi.json`: 新しいエンドポイント
- `docs/README.md`: 本計画書の索引追加
- `docs/verification.md`: 実機検証の記録（新しい節を先頭に）
- リリースノート: 「実験的: ChatGPT 使用枠」

## 13. 実装状況（2026-11-01）

Docker 版の Phase 1 縦切りを実装済み。Desktop 版は未着手。

### 完了

| 領域 | 実装 |
| :--- | :--- |
| カタログ | `providers/catalog.py` に `chatgpt` プリセット（`chatgpt_responses` / `chatgpt_models`） |
| 認証 | `providers/chatgpt.py`（デバイスコード、refresh、revoke、ID トークン解析、Secret 保存） |
| 推論 | `providers/adapters.py` の `chatgpt_responses`（Responses API、`prompt_cache_key`、reasoning include） |
| モデル | `chatgpt_models`（`/models?client_version=`、`{models:[{slug}]}`） |
| 残量 | `GET /wham/usage`、`x-codex-*` ヘッダ、`codex.rate_limits` SSE、60 秒キャッシュ |
| API | `api/routes_chatgpt.py`（デバイス開始/状態/中止、quota、logout） |
| UI | `apps/web/.../ChatGptPanel.tsx`（同意チェック、デバイスコード、クォータバー） |
| 課金 | `usage.record` が `billing_mode=subscription` を $0 として記録 |
| 安全 | base URL を Codex バックエンドに固定、ログアウトで revoke、トークン非表示 |
| テスト | `tests/unit/test_chatgpt.py`、`tests/integration/test_providers.py`（ChatGPT 追加分） |

### 未完了 / 次の作業

- 実アカウントでの疎通確認（デバイスコード → `/responses` → `/wham/usage`）
- Desktop 版（Responses アダプター、PKCE、Provider 登録、UI）
- `docs/README.md` への本計画書の索引追加
- 実機検証を `docs/verification.md` の先頭に追記

### 実装上の注意

- `originator` は既定で `mix_agent`。接続が拒否される場合のみ
  `MIX_CODEX_ORIGINATOR` で上書きする（Codex CLI の値はクライアント偽装になる）。
- デバイスコードの進行状態はプロセス内のみ。`--workers 1` 前提で、再起動すると
  進行中のログインは失効する。
- `POST /providers/{key}/logout` は generic な `/{key}/{action}` より先に
  マッチさせる必要があるため、`routes_chatgpt` を `routes_providers` より先に
  include している。

## 14. 参考（実装時に読む Codex CLI の該当箇所）

- `codex-rs/login/src/server.rs` — PKCE ログイン、コールバック、コード交換
- `codex-rs/login/src/device_code_auth.rs` — デバイスコード
- `codex-rs/login/src/auth/manager.rs` — refresh、クレーム、恒久エラー分類
- `codex-rs/model-provider-info/src/lib.rs` — `CHATGPT_CODEX_BASE_URL`
- `codex-rs/codex-api/src/endpoint/responses.rs` — `/responses`
- `codex-rs/codex-api/src/endpoint/models.rs` — `/models?client_version=`
- `codex-rs/codex-api/src/rate_limits.rs` — レート制限ヘッダ
- `codex-rs/backend-client/src/client/rate_limit_resets.rs` — `/wham/usage`
- `codex-rs/protocol/src/openai_models.rs` — モデル一覧の形
