# MIX-agent Desktop — Auto ルーティング強化計画

> 状態: 実装中（Phase 0〜1 実装済み、Phase 2 以降は未着手）
> 対象: Desktop 版の Auto モデル選択（`core/src/auto.rs` / `app/src/commands/auto.rs`）
> 前提: 選択は**ローカル・決定論的**のままにする（プロバイダへ問い合わせない）。
> 品質は [`coding-eval.md`](coding-eval.md) の合格率・虚偽完了・コストで判定する。
> 関連: [`agent-speed-plan.md`](agent-speed-plan.md) §6.1（速度の実測）、
> [`local-runtime-plan.md`](local-runtime-plan.md)（ローカル優先）、
> [`architecture.md`](architecture.md)（現行仕様）。

---

## 0. 結論 — 着手順

Auto の弱点は「候補の絞り込み」ではない。**判断材料が実測されていないこと**と、
**選んだモデルが失敗したときに次の手がないこと**である。効果の大きい順に次を実施する。

| # | 施策 | 主な効果 | 期待 | リスク | 工数 |
| :-- | :--- | :--- | :--- | :--- | :--- |
| 1 | ターン単位の計測基盤（TTFT / tokens/s / 成否 / コスト） | 速度・信頼性を学習できるようにする | 以降の全施策の前提 | 低（加算のみ） | 2〜3日 |
| 2 | 既存シグナルの実装（priority / speed / 失敗分類 / フィードバック鮮度） | 現在は固定値・未使用の材料を有効化 | 選択精度と復旧率が向上 | 低〜中 | 4〜5日 |
| 3 | プリ・ファーストトークンのフォールバック | 429/5xx/タイムアウトで Run が死なない | 復旧率 +、体感停止の削減 | 中（重複出力） | 4〜5日 |
| 4 | ルーティング品質（profile / 品質事前分布 / 予算 / スティッキー） | タスクに合うモデルを選ぶ | 品質・コスト・キャッシュ効率 | 中（評価必須） | 1〜2週間 |
| 5 | 説明と設定 UI | 「なぜこのモデルか」を出し、制御可能にする | 納得感・問い合わせ削減 | 低 | 1週間 |
| 6 | 評価・シャドウ・ロールアウト | 変更を安全に出す | 回帰検出 | 低 | 継続 |

**最初の PR は 1 + 2**（計測と分類の修正）。ユーザーに見える選択はほぼ変えず、
以降の施策が測れる状態にする。3 は評価ハーネスの準備後に着手する。

### 実装状況（2026-11-08）

| フェーズ | 状態 | 主な着地点 |
| :--- | :--- | :--- |
| 0. 計測基盤 | 実装済み | `0006_auto_stats.sql`、`TurnPerformance`、`record_turn_performance` |
| 1. 既存シグナル | 実装済み | `auto_priority` 設定、実測速度、`ProviderErrorKind` 分類、フィードバック鮮度、Context 見積もり、安定ハッシュ |
| 2. フォールバック | 一部実装 | 最初の応答前のモデル切り替え、`auto_fallback_attempts` 設定、Run への記録。回路遮断器とローカル起動待ちは未着手 |
| 3. ルーティング品質 | 未着手 | — |
| 4. 説明・設定 UI | 一部実装 | 優先度の設定 UI のみ。選択理由の表示は未着手 |
| 5. 評価・ロールアウト | 未着手 | — |

Phase 1 の暗黙フィードバック（再生成・モデル切り替え）は Phase 2 に送った。
計測は Run 終了後にまとめて書き込む（ターン単位の値は保持する）。

### Phase 2 の実装メモ

- フォールバック候補は Run 開始前にアプリ層で最大 2 件まで解決する
  （`resolve_auto_model_excluding`）。エンジンはモデルカタログを持たない。
- エンジンは **最初のテキスト・推論・ツール呼び出しより前**の失敗だけを
  切り替え対象にする。部分出力後の失敗は重複を避けるため再試行しない。
- 対象は `rate_limit` / `provider_5xx` / `transport` / `auth` /
  `context_overflow`。Context 超過は次の窓がより大きい場合だけ切り替える。
- 設定 `auto_fallback_attempts`（0〜2、既定 1）。
- 月次予算が上限に達している場合、ローカルモデルからクラウドへの
  フォールバックは行わない（ローカル primary が予算チェックを素通りするため）。
- 実際に切り替えた場合は `RunOutcome.auto_fallbacks` に記録し、
  `runs.data.modelId` / `providerId` / `autoFallbacks` と
  `autoSelection` を最終モデルに更新する。
- 回路遮断器の共有状態とローカルランタイムの起動待ちは Phase 2 の残り。

---

## 1. 現状（コード根拠）

### 1.1 選択の流れ

`mixd_core::auto::select_auto_model` は次の順で動く。

1. `models` テーブルの `data.auto != 0` を候補にする（`fetch_auto_candidates`）。
2. 非チャット既知モデル、Context Window、Vision、Tool Calling で足切りする。
3. 生存者をスコアリングする。
   - フィードバックの Beta 平均（`message_feedback`）
   - UCB の探索項
   - 静的な品質事前分布（`quality_prior`）とパラメータ数
   - 静的な価格表によるコスト効率（`pricing`）
   - 信頼性ペナルティ（`auto_reliability_events`）
   - 推論・継続・ローカルの小さなボーナス
   - Context 使用率、プロバイダ再試行のペナルティ
4. 完全同点は `request_key` のハッシュで決める。

### 1.2 既にあるが使われていない材料

| 材料 | 現状 | 根拠 |
| :--- | :--- | :--- |
| `AutoPriority`（`speed` / `balanced` / `quality` / `cost`） | 常に `Balanced` | `app/src/commands/auto.rs` の `AutoSelectOptions` |
| `speed_score` | 常に `0.0` | 同上。`RunPerf` 相当の記録がない |
| `excluded_ids` | 常に空 | 同上。フォールバックがない |
| `prefer_other_provider_than` | 常に `None` | 同上 |
| `_total_feedback` | 常に `0.0` | UCB は候補内の合計で代用している |
| `performance_events` テーブル | 未使用 | 計測の受け皿だけがある |
| `AutoSelection.reason` | 汎用文のみ。UI 非表示 | `ui/src` に `autoSelection` の参照がない |
| 失敗の分類 | 文字列一致。`provider_5xx` と `incomplete` は**生成されない** | `conversations.rs` の `record_auto_outcome` 呼び出し |

### 1.3 具体的な不整合（修正対象）

1. **5xx が学習されない。** 失敗分類は `auth` / `rate_limit` / `timeout` / `other` しか
   作らないが、`reliability_penalties` は `rate_limit` / `provider_5xx` / `timeout` /
   `incomplete` だけを数える。HTTP 500 は `other` になり、ペナルティが付かない。
2. **認証失敗も学習されない。** 401/403 は `auth` になるが、ペナルティ対象外。
   同じ壊れたキーのプロバイダを選び続ける。
3. **失敗が Run 単位で 1 件しか記録されない。** 複数ターンのうち 1 回失敗しても、
   最終的に成功すれば `success` だけが残る。ターン単位の可用性が失われる。
4. **フォールバックがない。** 選択は Run 作成時に 1 回だけ。最初のモデルが 429 でも
   Run は失敗する（transport の再試行は同一モデル内に限られる）。
5. **コンテキスト見積もりが狭い。** `estimate_required_tokens` は会話本文だけを見て、
   システムプロンプト、ツール定義、添付画像、プロジェクト指示を含めない。
6. **`context_window == 0` は「無制限」として通る。** 不明なモデルが大きな
   コンテキストを要求するターンで選ばれ得る。
7. **フィードバックが全件走査。** `message_feedback` を全件読み、Rust 側で profile を
   絞る。履歴が増えるほど Auto の解決が遅くなる。
8. **フィードバックに鮮度がない。** 半年前の 1 票と昨日の 1 票が同じ重み。
9. **profile の変更が履歴を孤立させる。** キーワード判定を変えると、既存の
   `message_feedback.data.profile` と一致しなくなる。バージョン付けがない。
10. **同点処理がプロセス・バージョン依存。** `DefaultHasher` は安定性が保証されず、
    永続化した `request_key` の再現性に使うには弱い。

---

## 2. 設計方針

- **core は純粋なまま。** スコア計算は `core/src/auto.rs`、DB・設定・計測は
  `app/src/commands/auto.rs` に置く。core に sqlx を持ち込まない。
- **判断根拠を残す。** `AutoSelection` に候補ごとのスコア内訳を追加し、UI と
  診断で同じデータを見せる。会話本文は保存しない（モデル ID と数値だけ）。
- **決定論を壊さない。** 乱数は使わない。同点は固定シードの安定ハッシュで決める。
- **フェイルオープン。** 統計テーブルが空・壊れている・読めない場合は、現在の
  事前分布とフィードバックだけで動く。Auto を止めない。
- **既存インストールの挙動を急に変えない。** 履歴がない状態では現行とほぼ同じ
  選択になることをゴールデンテストで保証する。
- **ロールアウトできる。** 新しいスコアは内部フラグ（`auto_routing_v2`）で切り替え、
  既定 OFF → 評価 → ON の順にする。

---

## 3. Phase 0 — 計測基盤（2〜3日）

### 3.1 何を測るか

1 ターンごとに次を記録する。Run 全体ではなくターン単位にするのが重要。

| 項目 | 用途 |
| :--- | :--- |
| `model_id` / `provider_id` / `profile` / `mode` | 集計キー |
| `ttft_ms` | 最初の `TextDelta` / `ReasoningDelta` / `ToolCallStart` まで |
| `duration_ms` | `stream()` 開始から `Finished` まで |
| `output_tokens` / `output_tps` | `Usage` と `duration_ms` から算出 |
| `input_tokens` / `context_tokens` | 予算・Context 余裕の学習 |
| `outcome` / `failure_kind` | 可用性 |
| `fallback_used` / `attempt` | 復旧の効果測定（Phase 2 以降） |

### 3.2 保存先

`performance_events` は生ログとして残しつつ、読み取りは集計テーブルにする。
Auto の解決は毎ターン走るため、全イベント走査は避ける。

```sql
-- storage/src/migrations/0006_auto_stats.sql（追加のみ）
CREATE TABLE IF NOT EXISTS auto_model_stats (
    model_id        TEXT NOT NULL,
    profile         TEXT NOT NULL,          -- '*' は全 profile の集計
    success_count   INTEGER NOT NULL DEFAULT 0,
    failure_count   INTEGER NOT NULL DEFAULT 0,
    ewma_ttft_ms    REAL,
    ewma_output_tps REAL,
    ewma_duration_ms REAL,
    last_failure_kind TEXT,
    last_failure_at TEXT,
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    PRIMARY KEY (model_id, profile)
);
CREATE INDEX IF NOT EXISTS auto_model_stats_profile_idx
    ON auto_model_stats(profile, updated_at);
```

- 更新は UPSERT 1 回。`ewma = ewma * (1 - α) + sample * α`、`α = 0.2`。
- 外れ値を抑えるため、TTFT は `0..120_000ms`、tokens/s は `0..1_000` に clamp する。
- 速度は成功ターンだけで更新する。失敗は `failure_count` と
  `last_failure_kind` に残す（失敗の速さは速度の指標ではない）。
- 90 日更新のない行は起動時ではなく、書き込み時に確率的に削除する
  （スケジューラを増やさない）。

### 3.3 計測点

- `core/src/runs/engine/execute_turn.rs` の `run_turn` で `Instant` を取り、
  最初のデルタ時刻と終了時刻を `TurnResult` に載せる。
- `Usage.output_tokens` と `duration_ms` から `output_tps` を出す。
  `Usage` がないプロバイダは `None` とし、速度学習に使わない。
- 保存は Run のクリティカルパスから外す。`conversations.rs` のバックグラウンド
  タスク、または `tokio::spawn` で書き、失敗は `log_write` に留める。
- 既存の `record_auto_outcome` は Run 単位のまま残し、ターン単位の失敗は
  新しい `record_turn_outcome` で記録する（二重計上を避ける）。

### 3.4 受け入れ条件

- 各ターン後に `auto_model_stats` が更新される。
- 計測追加による Run のクリティカルパスへの加算が 1ms 未満（保存は非同期）。
- 統計が空の DB で、現行と同じ選択になるゴールデンテストが通る。
- `cargo test -p mixd-core -p mix-agent-desktop auto` が通る。

---

## 4. Phase 1 — 既存シグナルの実装（4〜5日）

### 4.1 `AutoPriority` を設定可能にする

- `SettingsView` に `auto_priority`（`balanced` / `speed` / `quality` / `cost`）を追加。
  既定は `balanced`。`validate_settings` で列挙を検証する。
- `resolve_auto_model` は固定 `Balanced` ではなく設定値を渡す。
- UI は「設定 ▸ 一般」または「設定 ▸ モデル」にラジオを置く。説明文は
  「速度を優先すると品質が下がる場合があります」を明記する。
- `cost` は月次予算が設定されているときだけ意味が強い。未設定でも動く。

### 4.2 実測速度を `speed_score` に入れる

- `auto_model_stats` の `ewma_ttft_ms` と `ewma_output_tps` を候補ごとに読む。
- 正規化は候補集合内の相対値にする（絶対値はプロバイダ間で比較できない）。
  - TTFT が中央値より速い → 正、遅い → 負。
  - tokens/s が中央値より高い → 正。
  - `[-0.16, 0.16]` に clamp（現行の契約を維持）。
- データがないモデルは加点も減点もしない（`0`）。静的なベンチマークで速度を
  推測すると、実測前のモデルが不当に勝つため。速度は「観測されたら効く」
  シグナルとして扱う。
- `Speed` のときは `speed_scale` を上げ、`Quality` のときは下げる（既存の重み）。

### 4.3 失敗分類を型で運ぶ

現在の文字列一致をやめ、プロバイダ層の `ProviderErrorKind` をそのまま使う。
アダプタは SSE の `error` 文字列しか持たない場合があるため、
`StreamEvent::Error(String)` の形は変えず、**エンジン境界**で
`ProviderErrorKind::from_message` / `from_error` に変換する。全アダプタに
分類を強制するより変更範囲が小さく、未知の文字列は `Other` に落ちる。

- `ProviderErrorKind` に `as_str()`（DB の安定名）と `from_message()` を追加する。
- `transport::send_with_retry` の最終エラーは HTTP status を文字列に含むため、
  `from_error` がその文字列から status を復元する。
- `run_turn` は接続エラーと `StreamEvent::Error` の両方を分類し、
  `TurnPerformance.failure_kind` に載せる。
- `RunOutcome.turn_performances` を介してアプリ層へ運び、Run 終了後に
  `auto_model_stats` と `auto_reliability_events` へ書く。
- マッピング:

| `ProviderErrorKind` | 記録する `failure_kind` | Auto の扱い |
| :--- | :--- | :--- |
| `RateLimited` | `rate_limit` | モデルを短期クールダウン |
| `Unavailable` | `provider_5xx` | モデル + プロバイダをクールダウン |
| `Transport`（timeout 含む） | `timeout` | モデルを短期クールダウン |
| `Unauthorized` | `auth` | プロバイダをクールダウンし、UI で設定を促す |
| `TooLarge` | `context_overflow` | そのリクエストだけ除外（モデル評価は下げない） |
| `BadRequest` | `bad_request` | 原則ペナルティなし。ログのみ |
| キャンセル | `cancelled` | 記録するがペナルティなし |
| その他 | `other` | ログのみ。未知の分類は推測しない |

- 失敗イベントは**ターン単位**で記録する。成功は Run 終了時に 1 回だけ。
  同じ Run で成功と失敗が混ざる場合は、両方を別スコープで持つ。
- `auth` は「モデルが悪い」ではなく「プロバイダ設定が悪い」。プロバイダ単位の
  クールダウンにし、Auto が永久に全候補を失わないよう 30 分で解除する。

### 4.4 フィードバックを軽く・新しくする

- `message_feedback` に式インデックスを追加する。

```sql
CREATE INDEX IF NOT EXISTS message_feedback_profile_idx
    ON message_feedback(json_extract(data, '$.profile'));
```

- 読み取りは `WHERE json_extract(data, '$.profile') = ?` にし、全件走査をやめる。
  さらに `created_at >= datetime('now', '-180 days')` で上限を切る。
- 重みは鮮度で減衰させる。半減期 60 日。`weight = 0.5 ^ (age_days / 60)`。
- `_total_feedback` を実際に渡し、UCB の探索項を全 profile の合計で計算する。
- 暗黙フィードバックは Phase 1 では**未実装**（Phase 2 で追加する）。実装する
  場合は次の重みを初期値にする。明示より必ず弱くする。
  - 同じ回答の再生成: 直前の Auto モデルに `down` 0.5。
  - Auto の直後に手動で別モデルへ切り替え: `down` 0.3。
  - 同じ会話が 3 ターン以上続く: `up` 0.2（メッセージごとに 1 回まで）。
  - 停止（Stop）: どちらでもない（コスト・時間の都合を品質と混同しない）。
- 明示 `up` / `down` は 1.0。暗黙は 0.2〜0.5 に抑え、自己強化ループを防ぐ。

### 4.5 コンテキスト見積もりを実測に寄せる

- `resolve_auto_model` に次を渡す。
  - システムプロンプト（プロジェクト指示を含む）
  - ツール定義の JSON（`serde_json::to_string(&tools).len()`）
  - 添付の概算トークン（画像は固定 1,000 など、種類別）
  - 会話履歴（既存の直近 20 件）
- `context_window == 0` は「不明」として扱い、`required_tokens` が安全上限
  （例: 32K）を超えるなら候補から外す。小さい値ではなく、不明を無制限にしない。
- 予約出力はモードごとに変える（`chat` 2K、`agent` 8K など）。既存の
  `AUTO_RESERVED_OUTPUT_TOKENS` をモード別の関数にする。

### 4.6 同点処理を安定化する

- `DefaultHasher` を FNV-1a（固定 offset basis）に置き換える。
- `request_key` と候補 ID の組で決める。テストに固定の期待値を置き、
  Rust のバージョンで変わらないことを保証する。

### 4.7 受け入れ条件

- 5xx を返すモデルに実際にクールダウンが付く。
- 429 / timeout / auth / cancelled の扱いが表どおり。
- `speed` 優先で TTFT の速いモデルが選ばれ、`quality` 優先では能力事前分布が
  勝つ（合成データのテスト）。
- 履歴がない DB では現行と同じ選択（ゴールデン）。
- Auto 解決の p95 が 20ms 未満（ローカル DB、1,000 候補・10,000 フィードバック）。

---

## 5. Phase 2 — フォールバックと耐障害性（4〜5日）

### 5.1 プリ・ファーストトークンのフォールバック

Run 作成時の 1 回解決を、**最初の出力前だけ**やり直せるようにする。

- トリガー: `provider.stream()` の接続失敗、または最初の
  `TextDelta` / `ReasoningDelta` / `ToolCallStart` より前の `StreamEvent::Error`。
- 再解決の入力:
  - `excluded_ids` に失敗モデルを追加
  - `prefer_other_provider_than` に失敗プロバイダを指定
  - `request_key` から派生した決定的なキー（同じ候補順を保証）
- 上限は 2 回（初回 + 2 フォールバック）。同一モデルへは戻らない。
- 予算超過を避けるため、フォールバック先が現在のモデルより高価な場合は
  `Cost` / 予算逼迫時に限り許可しない選択肢を設ける。
- 出力が始まった後の失敗は再試行しない。部分テキストとツール呼び出しの
  重複を避け、Run を失敗として返す（UI に「別モデルで再試行」を出す）。
- `Run` の `data` に `autoFallbacks: [{modelId, failureKind}]` を残し、
  `AutoSelection` にも試行回数を持たせる。

### 5.2 サーキットブレーカー

- 失敗のクールダウンを Auto 選択だけでなく、プロバイダ呼び出し前の
  共通判定でも使う（`providers` か `app` の小さな状態）。
- 成功でクールダウンを半減し、連続成功で解除する。
- UI のプロバイダ一覧に「一時停止中（あと N 分）」を出す。

### 5.3 Context 超過時の再解決

- `TooLarge` / `context_length_exceeded` は、モデルの品質ではなく
  **このリクエストの Context 不足**として扱う。
- 出力前に検出したら、より大きな Context の候補で 1 回だけ再解決する。
  それでも足りなければ既存の圧縮（compaction）へ倒す。
- 同じモデルを profile 全体で不当に下げない。

### 5.4 ローカルランタイムの可用性

- `is_local` の候補は、選択前にランタイムの起動状態を確認する。
- 停止中 + `autoStart` なら起動を待つ（タイムアウト付き）。待てない場合は
  その候補をこのリクエストだけ除外し、クラウドへフォールバックする。
- 起動待ちの時間は TTFT に含めず、`startup_ms` として別に記録する。

### 5.5 受け入れ条件

- 1 台目のモデルが 429/503 を返し、2 台目で Run が成功する統合テスト。
- 最初のトークン後の失敗では、assistant メッセージが重複しない。
- フォールバックは最大 2 回で、無限ループしない。
- キャンセルは失敗として学習されない。
- ローカル停止中の Auto が、起動待ちで Run を止めない。

---

## 6. Phase 3 — ルーティング品質（1〜2週間）

### 6.1 profile を粗く、バージョン付きにする

現行の profile は `task` と能力フラグが混ざり、`coding:lang:ja:task:coding:diff:mid`
のように次元が増える。フィードバックが疎になり、少しの変更で一致しなくなる。

- `v2:` プレフィックスを付け、互換のない履歴を混ぜない。
- 次元は `task`（coding / reasoning / math / translation / summarize /
  extraction / vision / general）、`lang`、`mode` の 3 つに絞る。
- Vision / tools は**フィルタ**であって profile ではない。能力要件で足切りし、
  スコアの profile には入れない。
- 判定材料を増やす。
  - モード（`agent` / `plan` / `thinking`）
  - ツールの有無と種類（読み取り中心 / 編集 / ブラウザ）
  - 添付（画像・PDF・コード）
  - プロジェクトの言語・`verify_command` の有無
  - 直前ターンのツール結果（失敗ログなら coding/reasoning）
  - 日本語キーワードの拡充（「直して」「テスト」「設計」「比較」など）
- 判定は決定的なまま。LLM による分類は行わない（オフライン性と再現性）。
- 旧 profile の履歴は 90 日だけ低重みで読む移行期間を設ける（設定で切れる）。

### 6.2 品質事前分布をデータ駆動にする

- 現在の `quality_prior` は固定の短い tier 表。モデル名の family / version を
  解析し、次を合成する。
  - 既知 family の基本値（例: `claude-opus` > `claude-sonnet` > `claude-haiku`）
  - バージョン（新しいほど僅かに上、ただし上限付き）
  - パラメータ数（ローカルのみ）
  - reasoning / tools / vision の有無
- 表は 1 か所（`core/src/auto.rs` の const か `docs` ではなくコード内）に置き、
  provider 名ではなくモデル family で引く。
- 未知モデルは `None` のまま。推測で強い/弱いを決めない。
- 評価で事前分布の寄与を確認し、過剰なら `quality_strength` を下げる。

### 6.3 コストと予算

- 価格はモデルのメタデータ（プロバイダが返す場合）を優先し、静的表は
  フォールバックにする。ローカルは「既知の 0 円」として `None` と区別する。
- `monthly_budget_usd` の残量比を `AutoSelectOptions` に渡す。
  - 残量 50% 以上: 現行どおり。
  - 20〜50%: コストペナルティを少し強める。
  - 20% 未満: `balanced` でも安い候補を優先し、高価なモデルには
    「予算残量が少ない」理由を付ける。
- 予算超過は現在どおり Run 作成時にハード失敗させるが、Auto はその前に
  安い候補へ寄せる。
- 価格が不明なモデルは「安い」と誤認しない。中央値より高く見積もる。

### 6.4 会話内スティッキー

- 同じ会話では、明確な理由がない限り直前のモデルを維持する。
  現在の `continuity_bonus` 0.03 は弱く、毎ターン入れ替わり得る。
- 切り替えにはスコア差のしきい値（例: 0.08）を要求する。
- リセット条件: 失敗、モード変更、優先度変更、タスク分類の変化、
  ユーザーが明示的にモデルを変更。
- 切り替えたときは `AutoSelection.reason` に「会話の途中で切り替え」と残す。
- 目的はプロンプトキャッシュの維持と、回答スタイルの安定。

### 6.5 能力情報の扱い

- `context_window` の 0 を「不明」と「0」で区別する。不明は安全上限で判定。
- `is_known_non_chat` は prefix 列挙をやめ、discovery の `ModelCapabilities.chat`
  を優先する。不明な場合だけ現行の除外リストを使う。
- モデルごとのユーザー上書き（context / vision / tools）を許可する。
  誤検出した discovery を Auto のために直せるようにする。

### 6.6 受け入れ条件

- 合成シナリオで単調性が成り立つ。
  - フィードバックが多いモデルが、他の条件が同じなら選ばれる。
  - 5xx が多いモデルが、他が同じなら避けられる。
  - 予算残量が少ないと、安いモデルが選ばれる。
  - タスク分類が変わると profile が変わる。
- 同じ会話で、入力がほぼ同じならモデルが頻繁に変わらない。
- profile 変更後も、旧履歴が新 profile に混ざらない。

---

## 7. Phase 4 — 説明と設定 UI（1週間）

### 7.1 選択理由を出す

`AutoSelection` を拡張する（保存するのはモデル ID と数値だけ）。

```rust
pub struct AutoCandidateScore {
    pub id: String,
    pub score: f64,
    pub feedback: f64,
    pub exploration: f64,
    pub quality: f64,
    pub speed: f64,
    pub cost: f64,
    pub penalty: f64,
}
```

- `AutoSelection` に `scores: Vec<AutoCandidateScore>` と
  `fallbacks: Vec<AutoFallback>` を追加する。
- Chat の Run 詳細に「Auto: <解決モデル>」を出し、ポップオーバーで
  「候補数 / 選定理由 / 次点 / フォールバック有無」を表示する。
- 現在 `UsagePanel` は Run のモデルを出すが、Auto の理由は出していない。
  既存の `CapabilityBadge` / `StatRow` の表現に合わせる。
- 会話本文やプロンプトは理由に含めない。モデル ID・スコア・分類だけ。

### 7.2 設定画面

「設定 ▸ モデル」に Auto セクションを追加する。

- 優先度（balanced / speed / quality / cost）
- フォールバック（OFF / 1 回 / 2 回）
- ローカル優先（既存 `local_prefer` をここへ移すか、二重に置かない）
- Auto 候補の一括 ON/OFF と、候補ごとの能力・実測 TTFT・直近の失敗
- 「ルーティングを試す」: サンプル文を入れて、選ばれるモデルと理由を見る
  （`resolve_auto` のプレビューを利用。Run は作らない）
- 候補が 0 件のときの警告と導線（現行のエラーメッセージと同じ文言）

### 7.3 診断

- 設定 ▸ 診断に「Auto の直近の選択」を JSON で出せるようにする。
- 個人情報を含まないこと、モデル ID と時刻だけであることをテストする。

### 7.4 受け入れ条件

- Auto の各 Run で、解決モデルと理由が UI から確認できる。
- 候補 0 件・全滅時のエラーが、どの条件で落ちたか具体的に出す。
- ja / en の両方で文言が揃う（`scripts/check-*-parity.py` の対象に追加）。
- 新しい設定が保存・再起動後も保持される。

---

## 8. Phase 5 — 評価とロールアウト

### 8.1 テスト

| 層 | 内容 |
| :--- | :--- |
| `core/src/auto.rs` | 足切り、スコアの単調性、同点の安定性、profile v2 |
| `app/tests/auto.rs` | 統計・フィードバック・失敗分類・予算・フォールバックの統合 |
| `providers` | `ProviderErrorKind` → `failure_kind` の変換 |
| `ui` | 理由の表示、設定の保存、候補 0 件の警告 |
| 回帰 | 履歴なし DB で現行と同じ選択（ゴールデン） |

### 8.2 オフライン・リプレイ

- 候補集合・フィードバック履歴・失敗履歴・予算を固定した JSON シナリオを
  用意し、選択結果をスナップショットで比較する。
- シナリオは「初回インストール」「コーディング中心」「画像中心」
  「ローカルのみ」「予算残り 10%」「5xx 多発」の 6 種から始める。
- スコアの内訳もスナップショットに含め、意図しない重み変更を検出する。

### 8.3 ライブ評価

- [`coding-eval.md`](coding-eval.md) の scripted / live を使い、
  Auto と固定モデル（品質・速度・コストの代表）を比較する。
- 指標:
  - タスク成功率
  - 虚偽完了率
  - フォールバック率と復旧率
  - ユーザーのモデル上書き率
  - p50 / p95 TTFT
  - 成功 1 件あたりの推定コスト
  - 明示フィードバックの up 率
- シャドウモード: 実際には固定モデルで実行し、Auto が何を選んだかだけを
  ログする。選択の分布と、選ばれたモデルの実測品質を比較する。

### 8.4 ロールアウト

- 内部フラグ `auto_routing_v2`（既定 OFF）で新しいスコアを有効化する。
- 期間: シャドウ 1 週間 → 10% ロールアウト → 50% → 全員。
- 中止条件:
  - scripted eval の成功率が 2pt 以上低下
  - 虚偽完了率が増加
  - フォールバック率が 20% を超える
  - Auto 解決の p95 が 50ms を超える
- 中止時はフラグを戻すだけで旧経路に戻る（DB は加算のみ）。

---

## 9. 変更するファイル

| ファイル | 変更 |
| :--- | :--- |
| `core/src/auto.rs` | スコア、`AutoSelection` の内訳、profile v2、安定ハッシュ、テスト |
| `core/src/runs/engine/execute_turn.rs` | TTFT / duration / 失敗種別を `TurnResult` に追加 |
| `core/src/runs/engine/finish.rs` / `execute.rs` | 計測値と失敗種別を Run まで運ぶ |
| `providers/src/lib.rs` / `reliability.rs` | `StreamEvent::Error` に型付き分類、変換 |
| `providers/src/transport/mod.rs` | 最終エラーに `ProviderErrorKind` を付与 |
| `app/src/commands/auto.rs` | 統計読み書き、優先度、分類、フォールバック、暗黙フィードバック |
| `app/src/commands/conversations.rs` | 解決の再試行、計測の保存、分類の受け渡し |
| `app/src/commands/settings.rs` | `auto_priority` / フォールバック / 探索の設定 |
| `storage/src/migrations/0006_auto_stats.sql` | `auto_model_stats`、式インデックス |
| `ui/src/features/chat/*` | Auto の理由、フォールバック表示 |
| `ui/src/features/settings/*` | Auto セクション、プレビュー |
| `docs/desktop/architecture.md` | 実装後に現行仕様を更新（計画書ではなく仕様に書く） |

---

## 10. リスクと対策

| リスク | 対策 |
| :--- | :--- |
| フィードバックが少なく、ノイズで選択が暴れる | 事前分布を常に混ぜる。探索項に上限。暗黙は低重み。 |
| 自己強化ループ（選ばれたモデルだけ評価が上がる） | UCB の探索を維持。profile を粗く保つ。明示と暗黙を分ける。 |
| 速度優先で品質が落ちる | 既定は balanced。`quality` / `speed` は明示時だけ。coding-eval でゲート。 |
| フォールバックで回答が重複する | 最初のトークン前だけ再試行。部分出力後は再試行しない。 |
| Context 超過をモデルのせいにする | `context_overflow` はリクエスト単位の除外。モデル評価を下げない。 |
| 統計の読み取りが Auto を遅くする | 集計テーブル + インデックス。p95 20ms のテスト。 |
| migration で既存 DB が壊れる | 追加のみ。既定値で動く。ダウングレード時も無視できる。 |
| UI が「Auto が勝手に変えた」と感じさせる | 理由と変更点を表示。会話内スティッキー。 |
| プロバイダのエラー分類が不完全 | 未知は `other` としてログのみ。推測でペナルティしない。 |

---

## 11. 決定事項（推奨）

実装時に迷わないよう、次を初期値とする。評価で覆す場合は計画書に追記する。

1. **優先度の既定は `balanced`。** `speed` はユーザーが選んだときだけ。
2. **フォールバックは既定 1 回。** 2 回は設定で許可。最初のトークン前だけ。
3. **失敗分類は `ProviderErrorKind` を正とする。** 文字列一致を残さない。
4. **暗黙フィードバックは重み 0.2〜0.5。** 明示 1.0 より必ず弱い。
5. **profile は v2 で 3 次元（task / lang / mode）。** 能力はフィルタ。
6. **速度は成功ターンの EWMA のみ。** 失敗の速さは使わない。
7. **価格不明は「高い」側に倒す。** 安いと誤認して予算を壊さない。
8. **`performance_events` は監査用に残し、読み取りは `auto_model_stats`。**
9. **ロールアウトはフラグで戻せるようにする。** DB は加算のみ。
10. **UI にはモデル ID・スコア・分類だけ。** 会話本文は出さない。

---

## 12. マイルストーン

| 週 | 内容 | 完了条件 |
| :-- | :--- | :--- |
| 1 | Phase 0 + 4.1〜4.3 | 計測、優先度、失敗分類。ゴールデン維持 |
| 2 | 4.4〜4.6 | フィードバック、Context 見積もり、安定ハッシュ |
| 3 | Phase 2 | フォールバック、サーキットブレーカー、Context 再解決 |
| 4〜5 | Phase 3 | profile v2、品質事前分布、予算、スティッキー |
| 6 | Phase 4 | 理由表示、設定、プレビュー、診断 |
| 7〜 | Phase 5 | オフライン・リプレイ、シャドウ、段階ロールアウト |

最初の 2 週間で「壊れない・測れる・直せる」状態にし、その後の品質改善を
数字で判断できるようにする。
