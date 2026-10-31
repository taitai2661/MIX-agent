# 記憶機能 大幅強化ロードマップ

対象: **Docker 版 + Desktop 版**（コード共有は禁止のため、仕様書と契約テストで同期する）
主軸: ①意味検索（embedding） ②記憶の統合・矛盾解消 ③自動抽出の品質向上 ④関連付け・知識グラフ ⑤管理 UI・可視化

作成日: 2026-10-09

---

## 進捗サマリ

| Phase | 状態 | 計画からの逸脱 |
| --- | --- | --- |
| 0 基盤と仕様書 | **完了** | なし |
| 1 意味検索 | **完了**（両版） | 候補取得は SQL へ移さず（下記 1-c' を参照）。専用テーブルを設けず `data` へ格納（下記 1-b' を参照） |
| 2 自動抽出の品質向上 | **完了**（両版） | 抽出プロンプトを仕様 §6 に固定し、parity で逐語一致を CI 検出する契約に変更 |
| 3 統合・矛盾解消・寿命 | **完了**（両版） | 0024 で 8 列を復元（案 B）。`forget` は §7.2 どおりハード削除のままとし、`supersede` との区別を §3 に明記。Desktop は列が既存のため**マイグレーション不要**、挙動のみ |
| 4 関連付け・知識グラフ | **完了**（両版） | 計画の `kind` 語形を §2 `relation_kinds` に訂正。生成源 (1) を §6 プロンプト増やさず抽出後の機械生成に変更。**鏡の 2 種は `link()` が拒否する**契約を §4 に確定（下記 4-a' を参照） |
| 5 管理 UI・可視化 | **完了**（両版） | 計画の `AssociationsView` / `KnowledgeView` / `ChipsAutocomplete` は**名指しだけで根拠が無い**ため、§9 #12（エッジが UI から見えない）に絞った |
| 6 検索品質ベンチ | **完了**（両版） | synthetic スイートは両版で数値が一致しないため、**CI ゲートは共有 fixture スイートに限定**した（下記 6'' を参照） |

### 6' 共有 fixture の配置（計画からの逸脱）

計画は共有 fixture を `tests/fixtures/memory_recall_cases.json` の一箇所に置く想定だった。
実際には **`docker/MIX-agent/tests/fixtures/` と `desktop/mix-agent-desktop/core/tests/fixtures/` の
2 コピー**を置き、`check-memory-parity.py` が**バイト一致**を検査する方式にした。

理由: ローカルツリーは `docker/.git` と `desktop/.git` の**別リポジトリ**であり、Desktop 版だけを
clone した環境では `docker/` 配下を参照できない。仕様書とまったく同じ構造（各リポジトリに 1 本、
parity が同期）にすることで、単体 clone でも `cargo test` が通るようになった。
CI は `.github/workflows/ci.yml` がルートを一括 checkout するため、両コピーが揃い parity が働く。

> **サブエージェントの報告は必ずツリーと突き合わせること。**
> Phase 2 の Desktop 版について、ファイル一覧・テスト名・検証結果まで記した詳細な完了報告が
> 送られたが、`find -newermt` と `grep` で確認したところ**その内容は実在の変更と一致しなかった**
> （`core/src/scheduler.rs` も `core/tests/memory_extraction.rs` も存在せず、
> `EXTRACTION_PROMPT` も `observe_turn` も定義されていない）。
> 報告を受けたら `find` / `grep` / `cargo test` で裏を取る、をこの作業の手順に加える。


**Phase 0 の成果**: `docs/memory-spec.md`（両 edition に同一内容・バイト一致）、
`scripts/check-memory-parity.py`（語彙 49 項目 + ランキング定数 9 項目を両版の実装定数と突き合わせ）、
`.github/workflows/ci.yml` に `memory-parity` ジョブ。決断 A = 案 B、決断 B = アプリ内 cos sim を確定。

**Phase 1 の成果**:
- `Adapter.embed` / `Provider::embed`（Anthropic は `EmbeddingNotSupported`）
- ベクトルは両版とも**行の `data` JSON** に `embedding` + `embedding_model` として保存
- ハイブリッドランキング `rank()`（Docker）/ `retrieve_with_query_vector()`（Desktop）、
  ベクトルが使えない組は**必ず `without_vector` の重み**に戻す
- 共有 fixture `tests/fixtures/memory_recall_cases.json`（7 ケース × 実測スコア）が両版のテストで読まれる
- 書き込み時に embedding（API・ツール・自動抽出ジョブ）、`/memories-debug/reembed` で backfill

### 計画からの主な逸脱とその理由

**1-b' 専用テーブルを持たない** — 計画は Docker に `0023` の embedding 列、Desktop に
`memory_embeddings` テーブルを置く想定だった。実装は**両版とも行の `data` JSON** に
`embedding` を持たせた。仕様 §1.1 が「物理的な列の持ち方は各エディションが決められる」と
定めており、これを採るだけでマイグレーションが 1 つも要らず、既存デプロイの移行リスクが消える。
pgvector を入れる話も、テーブルを分ける話も、件数が数千件を超えてからでよい。

**1-c' 候補取得を SQL に移さない** — 計画は `pg_trgm` の `memory_content_trgm` で候補を絞る
想定だった。だが**ベクトルランキングは候補を全件見なければ成立しない**。trigram で絞った時点で
意味的に一致する行が候補から落ちるので、この最適化はむしろ意味検索の recall を下げる。
正解は逆で、アプリ内 cos sim ならば全件走査が正しい設計になる。件数上限は
`service.CANDIDATE_LIMIT = 2000`（`embeddings.BACKFILL_SCAN_LIMIT` も同値）とし、
パーソナルな記憶ストアとしては十分な余裕をもって見込む。

**ranking を 9 定数に固定** — 仕様の `ranking` ブロックはフラットな 9 キーとし、
`check-memory-parity.py` が**両版の実装定数と数値一致**を検査するようにした。
`context_budget_ratio` は Docker 固有の実装詳細なので JSON から外し §5.3 の本文に残した
（Desktop は固定文字数予算のため対応する定数が存在しない）。

---

## 0. 前提と検証手段

### 開発方針の制約（`README.md:21-25` より）

- `desktop/` ↔ `docker/` のコード共有は**禁止**。両者を結ぶのは `docs/` の仕様書と契約のみ。
- したがって「記憶機能を強化する」場合は **仕様書 → 両版で別実装 → 契約テストで乖離を検出** の順になる。

### 検証コマンド（`.github/workflows/ci.yml` と同一）

| エディション | コマンド |
| --- | --- |
| Docker | `docker compose --profile test run --build --rm test` |
| Docker | `uv run ruff check` / `uv run ruff format --check` |
| Desktop | `cargo fmt --all -- --check` |
| Desktop | `cargo clippy --workspace --all-targets -- -D warnings` |
| Desktop | `cargo test --workspace` |
| Desktop | `pnpm ui:typecheck` / `pnpm ui:lint` / `pnpm ui:test` |
| 共通 | `python3 scripts/check-greeting-parity.py`（挨拶版の parity。記憶にも同型のものを追加する → Phase 0） |

---

## 1. 現状（着手前のベースライン）

> [!NOTE]
> この節は **2026-10-09 の着手前ベースライン** を記録したものです。Phase 1〜6 は
> 上の進捗サマリのとおり完了しており、embedding / Knowledge 検索 / 関連付けは
> すでに実装されています。現在の実装を確認するときは、この節ではなく
> 「進捗サマリ」と各 Phase の成果、および実コードを参照してください。

### 1.1 両版の対応表

| 項目 | Docker 版（Python / FastAPI / PostgreSQL） | Desktop 版（Rust / SQLite） |
| --- | --- | --- |
| 本体 | `apps/server/mix_agent/memory/`（`service` 236 / `runtime` 180 / `jobs` 171 / `tools` 52 行、計 1017 行） | `core/src/memory.rs`（353 行） |
| データモデル | **flat**。`memories.content` + `data={pinned, source, source_run}` のみ。taxonomy 列と graph 4 テーブルは `0022` で削除 | **taxonomy 保持**。`lifecycle_state` / `role` / `scope` / `confidence` / `salience` / `strength` / `verification` / `task_id` … + `memory_associations` + `memory_features` テーブルが現存（`storage/src/migrations/0001_initial.sql:196-255`） |
| 検索 | `terms()` の語彙一致 0.85 + recency 0.15。**直近 200 件を Python で全件スキャン**（`service.py:139,189`）。`memory_content_trgm` インデックスは存在するが未使用 | CJK unigram/bigram の lexical。スコア 0 除外 + lifecycle フィルタ + ヒット時 `reinforce`（`core/src/retrieval.rs`） |
| 意味検索 | **なし**（`Adapter` に `embed` が無い） | **なし**（`providers/src/lib.rs:249` の `Provider` は `stream` のみ） |
| 自動抽出 | LLM による post-response job。最大 3 件・200 字・安定事実のみ・再試行付きスケジューラ（`jobs.py`） | **未実装**（`desktop/docs/architecture.md:365` で明示） |
| 明示的記憶 | 「覚えて：」「〜を忘れて」の正規表現検出（`service.explicit_candidate`）+ `memory` ツール | `memory` ツール（`recall` / `remember` / `forget` / `search_knowledge`、`tools/src/memory.rs`） |
| 統合・矛盾 | 完全一致 dedupe のみ。`supersede` は**ハード削除**、`dispute` / `resolve_conflict` / `decay` は実質ノーオプ（`runtime.py:120-138`, `service.py:235`） | `verify` / `reinforce` は実装済み。矛盾の**検出**は未着手 |
| 関連付け | `associations_for()` は常に `[]`（`service.py:217`）、`GET /memories/{key}/associations` も常に `[]` | `memory_associations` テーブルはあるが、検索での展開は未確認 |
| context 注入 | `context/builder.py`。memory 予算 **10%**（`context/types.py:26`）、フラット箇条書き・1 件 600 字。`memory_ids` / `memory_groups` を debug 出力に載せる | `core/src/retrieval.rs`。`<retrieved_context>` ブロックとして注入、全体 8000 字・1 件 1200 字 |
| Knowledge 注入 | **未接続**。`context/retrievers.py:82` の `search_knowledge` は `Phase 1: interface only` で `[]` を返す | lexical 検索は実装済み |
| 管理 UI | `apps/web/src/features/memory/`（`Memories.tsx` 225 行 / `TraceView.tsx` 262 行） | `ui/src/features/memory/MemoryPane.tsx`（280 行） |

### 1.2 現状のボトルネック（5 軸すべてに関わる）

1. **検索が Python 上の 200 件全件スキャン** — スケールしない。PostgreSQL に `pg_trgm` インデックスがあるのに使えていない。
2. **embedding が両版とも無い** — 言い換え・同義語に弱い。これが「記憶しても思い出せない」の主因。
3. **Docker は flat 化で ライフサイクル／矛盾／関連をすべて失い**、Desktop はそれを保持している → 仕様書同期の前提が崩れている。
4. **Desktop に自動抽出が無い** → Desktop の記憶は人手入力に依存。
5. **「何が注入されたか」が UI から見えない** → 記憶を増やすほど誤想起を検証できない。

---

## 2. 着手前に確定が必要な 3 点（Phase 0 の判断事項）

### 2.1 未コミットの WIP を先に退避する

| リポジトリ | 状態 |
| --- | --- |
| `docker/`（トップレベルが git ルート） | **7 ファイルが未コミット**（`adapters.py` / `usage.py` / `context/summary.py` / `runs/engine.py` / `context/builder.py` / `api/routes.py` / `api/schemas.py`、+103 / -24）。記憶と無関係な作業の可能性が高い |
| `desktop/` | **46 ファイルが未コミット**（+2239 / -343）。`ProvidersPane.tsx` など記憶と無関係な大規模変更を含む |

スキーマ変更（Alembic マイグレーション / SQLite マイグレーション）の前に、**少なくとも記憶まわりのファイルだけ独立コミットに分離**する。
混在したまま進めると、失敗時の切り戻しが効かない。

### 2.2 データモデルの正を 1 つに決める（最重要）

**衝突**: `docker/.../db/models.py:200-202` に次が書かれている。

> `NOTE: MemoryAssociation / MemoryFeature / MemoryConflict / MemoryEvidence were removed in the ChatGPT-style simplification (migration 0022 drops the tables). Do not re-add them; use plain Memory rows only.`

一方で今回の依頼は「統合・矛盾解消」と「関連付け・知識グラフ」であり、**この注記と正面から衝突する**。黙って壊さず、判断の変更として明示的に書き換える必要がある。

| 案 | 内容 | 評価 |
| --- | --- | --- |
| **A: flat 維持** | 関連も矛盾も `Memory.data` JSONB に `links[]` / `conflict_with` として持つ。テーブルは増やさない | 0022 の判断を尊重、実装最小。ただしリンク数・双方向参照・部分インデックスが扱いにくい。**グラフ拡張検索の実装が複雑になる** |
| **B: flat の外皮 + 必要 8 列 + `memory_links` 単一テーブル**（推奨） | `content` 列と UI 向けフラット構造は維持。統合に必要な `lifecycle_state` / `superseded_by_id` / `disputed_by_id` / `confidence` / `salience` / `role` / `scope` / `source_kind` を戻し、関連と上書き関係を **`memory_links` 1 テーブル**に集約 | 0022 が削除した「4 テーブル（associations / features / conflicts / evidences）の復活」とは区別できる。Desktop は既存 `memory_associations` を `memory_links` 相当に寄せるだけで差分が小さい。**UI / 検索 / 矛盾解消の全軸に必要な語彙が揃う** |
| **C: 0022 の全面撤回** | `0021` の taxonomy と 4 テーブルをそのまま復元 | 両版の差が縮まるが、フラット UI（ChatGPT 風）と 4 テーブルの複雑さを同時に抱える。管理 UI の設計が二重になる |

**推奨は B。** 採用する場合、次の 2 点を必ず同時に行う:

1. `db/models.py:200-202` の「Do not re-add them」注記を、**何を残し何を捨てたか**が分かる文に書き換える（`0022` の docstring も同様）。
2. `desktop/docs/` と `docker/MIX-agent/docs/` の両方に **Memory 仕様書**を置き、列名・ライフサイクル語彙・リンク種別・tool 契約を 1 か所で定義する。

### 2.3 embedding の実装方式

`compose.yaml:33` は `image: postgres:17.5-bookworm` — **pgvector を同梱していない**。`deploy/docker`・`deploy/egress`・`scripts/render_zimaos_compose.py` まで image 変更が波及する。

| 方式 | 利点 | 代償 |
| --- | --- | --- |
| pgvector 導入（`pgvector/pgvector:pg17` へ image 切替） | 部分一致・近傍検索が SQL で完結、スケールする | compose / ZimaOS render / CI の `postgres-test` まで image を揃える必要。既存ボリュームでの image 差し替え要検証 |
| **ベクトルは Python / Rust 内で全件 cos sim**（推奨はセーフバックとして併用） | image 変更ゼロ、既存デプロイを壊さない | 件数が増えたら線形に遅くなる。**数百〜数千件までは実用** |

→ **Phase 0 では「pgvector を入れるか」を決めずとも、`embed` が無い場合に lexical のみへ自動フォールバックする契約を先に固定する**。これにより Phase 1 を段階的に出荷できる。

---

## 3. フェーズ計画

```
Phase 0  基盤と仕様書の確定            [小]  … 両版の前提を揃える。以降の全フェーズの土台
Phase 1  意味検索（embedding）          [大]  … 軸①
Phase 2  自動抽出の品質向上            [中]  … 軸③（Desktop は新規実装）
Phase 3  統合・矛盾解消・寿命           [中]  … 軸②
Phase 4  関連付け・知識グラフ           [中]  … 軸④
Phase 5  管理 UI・可視化                [中]  … 軸⑤
Phase 6  検索品質ベンチと回帰防止       [小]  … 横断
```

Phase 1 と 2 は並行可能（担当ファイルが重ならない）。Phase 3 は Phase 1・2 の成果（語彙付き候補とベクトル）に依存する。

---

### Phase 0 — 基盤と仕様書の確定（小）

**目標**: 以降の全フェーズが「どちらか片方だけ直して忘れる」事なく進める状態を作る。動作変更は最小。

| 項目 | 内容 |
| --- | --- |
| 作業 1 | 未コミット WIP の分離（2.1）。記憶以外は `wip:` 系コミットへ退避 |
| 作業 2 | データモデルの決定（2.2 の A / B / C）。**推奨 B** |
| 作業 3 | Memory 仕様書を新規作成。両 edition の `docs/` に同一内容を置く<br> - 列 / タイプ / 既定値、ライフサイクル語彙（`candidate` / `active` / `established` / `superseded` / `expired` / `archived` / `disputed`）<br> - `memory_links` の `kind`（`related` / `supersedes` / `conflicts_with` / `derived_from`）と `weight` の意味<br> - tool 契約（`memory_search` / `memory_add` / `memory_update` / `memory_delete` の入出力 JSON）<br> - 検索のランキング式（lexical / vector / recency の重み）と既定値 |
| 作業 4 | `scripts/check-memory-parity.py` を新規作成し、`ci.yml` にジョブ追加<br>現行 `check-greeting-parity.py`（`scripts/`）と同じ方式で、仕様書の各節が両 edition で本文一致すること・tool 契約 JSON が一致することを検査する |
| 作業 5 | `context/retrievers.py:82` の `search_knowledge` が `[]` を返している点を「未実装」と明記し、Knowledge 注入を Phase 5 の後続タスクとして整理（記憶の「など」に該当） |

**変更対象**: `docker/MIX-agent/docs/memory-spec.md`（新規）、`desktop/docs/memory-spec.md`（新規）、`scripts/check-memory-parity.py`（新規）、`.github/workflows/ci.yml`、`docker/.../db/models.py` と `0022` の docstring（判断変更の明記）

**検証**: `python3 scripts/check-memory-parity.py` が成功すること。既存テストが green のままであること。

**完了条件**: 仕様書 1 本で両版の記憶データモデル・tool 契約・ランキング式を説明でき、CI が乖離を検出する。

---

### Phase 1 — 意味検索（embedding）（大）／軸①

**目標**: 「同じ意味で別の言い方」を思い出せるようにする。**embedding が無い環境では必ず現状の lexical 検索へ落ちる**こと。

#### 1-a. Provider 契約への `embed` 追加（両版）

| 版 | 変更 |
| --- | --- |
| Docker | `apps/server/mix_agent/providers/adapters.py` の `Adapter` に `async def embed(self, model, texts) -> list[list[float]]` を追加。`_openai` / `_compatible` は `POST /embeddings`、`_gemini` は `embedContent`、`_ollama` / `_lmstudio` は `/api/embeddings`。**Anthropic は embedding を提供しないため明示的に `NotSupported`** を返し、呼び出し側をフォールバックさせる |
| Desktop | `providers/src/lib.rs:249` の `Provider` trait に `async fn embed(&self, ...) -> anyhow::Result<Vec<Vec<f32>>>` を追加し、既定実装で `NotSupported`。実装は OpenAI / Gemini / Ollama / 各 wrapper に追加（Anthropic は既定のまま） |

> trait 変更は実装全数に波及する。**`not_supported` の既定実装を先に用意してから**各実装へ広げる。

#### 1-b. 格納とインデックス

- **Docker**: `0023` マイグレーションで embedding 列を追加。**pgvector を採用する場合**は compose / `deploy/` / `render_zimaos_compose.py` / `postgres-test` の image を `pgvector/pgvector:pg17` に統一し `CREATE EXTENSION vector`。**採用しない場合**は `memories.data.embedding`（JSONB 配列）に保存し、件数上限付きでアプリ側 cos sim。
- **Desktop**: `storage/src/migrations/` に `memory_embeddings`（`memory_id` / `dim` / `vector` BLOB）を追加。SQLite はトランザクション内で全件 cos sim。`run.rs` のデータ移行関数に載せる（同ファイル docstring: JSON 形状の変更は `run_data_migrations` で一括、が方針）。

#### 1-c. ハイブリッド検索への置き換え

- Docker `memory/service.py:181 search()` の**200 件 Python スキャンを SQL へ移す**（`pg_trgm` の `memory_content_trgm` が既にあるので `content % :q` / `ILIKE` で候補を絞る）→ その上で vector / recency を合成。
- Desktop `core/src/retrieval.rs` も同じ合成式を使う。
- **ランキング式は Phase 0 の仕様書に書いた式を両版とも参照**する。`service.py` の `"weights": {"lexical": 0.85, "recency": 0.15}` の debug 出力は、合成後の内訳（`lexical` / `vector` / `recency`）に拡張する。
- ベクトルが無い項目・`embed` 非対応プロバイダ → `vector` 項みを 0 とし、挙動は Phase 1 以前と同一。

**変更対象**: `providers/adapters.py`、`providers/src/lib.rs`（+ 各実装）、`memory/service.py`、`memory/runtime.py`、`context/retrievers.py`、`migrations/versions/0023_*.py`、`storage/src/migrations/`、`core/src/retrieval.rs`、両版の仕様書

**検証**:
- Docker: `tests/integration/` に `test_memory_hybrid_search.py`（embedding 非対応の provider で lexical のみに戻ること、同義語クエリでヒットすること、200 件超でも SQL で完結すること）
- Desktop: `core/tests/` に同趣旨の `cargo test`
- 両版で同じ fixture（クエリ → 期待する memory id）を **共有する JSON を `tests/fixtures/memory_recall_cases.json` に置き**、両版のテストがそれを読む

**完了条件**: 「ユーザーの好み」→「パーソナルな設定」のような言い換えで両版ともヒットする。embedding が使えない環境でも既存テストが green。

**リスク**: image 切替が既存ボリュームに与える影響。**Phase 1 の最初のスパイクで pgvector 版 compose を 1 回だけ起動・移行・起動確認する**。

---

### Phase 2 — 自動抽出の品質向上（中）／軸③

**目標**: 「記憶すべきことが残らない／逆にノイズが残る」を減らす。**Desktop に自動抽出を新規実装する**。

#### 2-a. Docker の抽出品質改善（`memory/jobs.py`）

現状: 最大 3 件・各 200 字・**安定した事実のみ**・`user_content` / `assistant_content` の 2 断片のみを渡す。

| 改善 | 内容 |
| --- | --- |
| 入力の拡張 | 直近 N ターン + 会話 summary を渡す。`context/summary.py` / `tiered_summary.py` が既にあるので再利用できる |
| **統合判定の LLM 化** | 現状 dedupe は**完全一致のみ**（`service.py:137-147`）。抽出時に既存記憶の上位 10 件を候補として一緒に渡し、「既存と重複 / 既存を置換 / 新規」を **1 回の呼び出しで判定**させる。件数削減と矛盾防止を同時に達成 |
| 役割の付与 | 値に `role`（`fact` / `preference` / `goal` / `constraint`）と `confidence` を持たせる（Phase 2 の案 B 採用時はこの列が使える） |
| 契機の拡張 | ユーザ発言のみでなく、run の決定事項・成果物の要約も候補にする。`runs/engine.py:71 _collect_memory_observations` は現在 "no synchronous task-memory harvesting" で空 → 実験的に有効化できるか検証 |
| 明示コマンドの強化 | `service.explicit_candidate` の正規表現は 2 パターンのみ。命令形・敬語・英語の揺れを追加し、**モデルに 1 回判定させるフォールバック**を用意 |

#### 2-b. Desktop の自動抽出（新規）

- `core/src/runs/engine.rs` の完了フックで `MemoryProcessingJob` 相当のジョブを積む（テーブルは `0001_initial.sql:257` に既にある）。
- 抽出呼び出しは `Provider::stream` で行い、Docker 版 `jobs.py` と同じプロンプト・同じ JSON スキーマを使う（**プロンプトは仕様書に書き、両版で本文一致させる** → Phase 0 の parity スクリプトの検査対象）。
- スケジューラ: `app/src/scheduler.rs`（既存）にフックする。Docker 版と同じく **リトライ 3 回・バックオフ**。
- 設定: `memory_auto_formation` 相当のトグルを UI に置く（`GeneralSettings.tsx` に既存パターンがある）。

**変更対象**: `memory/jobs.py`、`context/summary.py`（再利用）、`runs/engine.py`、`core/src/runs/engine.rs`、`app/src/scheduler.rs`、`storage`（ジョブの永続化確認）、仕様書の抽出プロンプト節

**検証**:
- Docker: `tests/integration/test_memory_auto_trace.py` が既にある → 件数上限・スキャンダラス語の除外・重複統合のケースを追加
- Desktop: `core/tests/` に抽出ジョブのテスト（provider をモックし、JSON パース失敗でリトライすること）
- **両版で同一プロンプトであることを parity スクリプトで検査**

**完了条件**: 「これは覚えておいて」以外の形で話したときにも記憶が残る。Desktop で会話後に Memory が自動増える。秘密情報は両版とも `SENSITIVE` で弾かれる。

**リスク**: 抽出ノイズの増加。**Phase 3 の統合とセットで出荷しないと重複が増える**ため、Phase 2 単独では feature flag を既定オフにしておく。

---

### Phase 3 — 統合・矛盾解消・寿命（中）／軸②

**目標**: 記憶が「増え続ける一方向のログ」ではなく、**最新の事実に更新され、古ければ失効する**状態にする。

| 機能 | 実装内容 | 現状との差 |
| --- | --- | --- |
| 意味的 dedupe | Phase 1 のベクトルで既存候補を取得し、しきい値超えたら `reinforce`（同一視）、近くなければ `create` | Docker は完全一致のみ |
| **supersede** | 「新しい事実が古いを置き換える」。**ハード削除せず** `lifecycle_state='superseded'` + `superseded_by_id` を立て、revision を必ず残す | Docker `runtime.supersede` は現在 **`db.delete(old)`**。復元は revision 経由のみで、リンクが切れる |
| conflict 検出 | 同一 subject で異なる value を持つ候補を `disputed` にし、**解決待ちとして UI に出す** | `dispute` / `resolve_conflict` はノーオプ（`runtime.py:132-138`） |
| lifecycle | `candidate` → `active` / `established` → `superseded` / `expired` / `archived`。検索対象を絞る | Docker は列ごと削除済み。Desktop は列はあるが遷移が `verify` しか無い |
| **decay** | 長期未激活の `strength` / `salience` を減衰、下限で `archived`。`pinned` は除外。`service.decay()` を実装し、`jobs` スケジューラに日次 1 回のフックを追加 | Docker `decay()` は `return 0`（`service.py:235`） |
| reinforce | 検索でヒットしたら強化 | Docker 未実装、Desktop は実装済み（`memory.rs:329`） |

**統合と矛盾の判定も Phase 2-a と同様に LLM を 1 回使い、`plan` を返して `apply_plan` で永続する**構造（`runtime.evaluate` / `apply_plan`）は既にあるので、**アクションを `supersede` / `dispute` に接続する**だけで済む。現状 `supersede` は `update` にマッピングされ（`runtime.py:96`）、`dispute` は `create` に落ちているので、ここを実装に置き換える。

**変更対象**: `memory/runtime.py`、`memory/service.py`、`migrations/versions/0024_*.py`（案 B の列復元）、`core/src/memory.rs`（`superseded_by_id` / `disputed_by_id` の遷移）、`core/src/retrieval.rs`（lifecycle フィルタの更新）、仕様書のライフサイクル節

**検証**:
- Docker: `tests/integration/test_memory_role_taxonomy.py` が既にある → supersede / dispute / decay / reinforce の遷移テストに再利用・拡張
- 破壊的操作: supersede で **revision が必ず残ること**、削除が発生しないこと
- Desktop: `core/tests/` に lifecycle 遷移と revision 復元の所有チェック（`core/src/memory.rs:296` で既に実装済みのガード）の回帰テスト

**完了条件**: 「重複した記憶が 5 件並ぶ」状態が解消し、矛盾は UI から解決できる。`decay` 後も `pinned` の記憶は失われない。

**リスク**: 既存データの移行（Docker の flat 行に `lifecycle_state` を既定値で付与）。`0022` の backfill が `try/except` 吞みなので、**移行テストを明示的に作る**こと。

---

### Phase 4 — 関連付け・知識グラフ（中）／軸④

**目標**: `associations_for()` の空実装を埋め、**想起の段階で関連する記憶を展開できる**ようにする。

#### 4-a. スキーマ（案 B 採用時）

```
memory_links (
  id, owner_id,
  source_memory_id, target_memory_id,
  kind   -- related | supersedes | conflicts_with | derived_from
  weight REAL, coactivation_count INTEGER,
  created_at, updated_at
)
UNIQUE (owner_id, source_memory_id, target_memory_id, kind)
```

Desktop の `memory_associations`（`0001_initial.sql:226-241`）はこの形に寄せる。`kind` は当初 `related` のみでもよい（`supersedes` / `conflicts_with` は Phase 3 の状態遷移を表現するために必要）。

#### 4-b. リンクの生成源（3 経路）

| 経路 | 実装 |
| --- | --- |
| (1) 抽出時に LLM が提案 | Phase 2 の 1 回の呼び出しで `links: [{to, kind}]` を返させる。既存記憶 id を候補として渡す |
| (2) coactivation | 同一 run でヒットした記憶ペアの `coactivation_count` を加算（Docker `context/builder.py` の `memory_ids` を入力にできる） |
| (3) ベクトル近傍 | Phase 1 の embedding で top-k 近傍を `related` として自動生成（しきい値と最大度数で暴走を防ぐ） |

#### 4-c. 検索での展開

- `service.associations_for()`（現在 `return []`）を実装し、`runtime.recall()` の後段で **seed → 1 ホップのみ**展開する。`debug` 出力の `association_expansion` / `budget_exhausted` は既に枠がある（`service.py:205`）ので、そこに実測値を載せる。
- 展開分は Phase 1 のハイブリッド結果と同様に予算（`context/types.py` の memory 10%）で切る。
- API `GET /memories/{key}/associations`（現在 `return []`、`routes.py` ）を実装。Desktop は `MemoryDispatch` に `associations` アクションを追加。

**変更対象**: `migrations/versions/0025_memory_links.py`（案）、`memory/service.py`、`memory/runtime.py`、`api/routes.py`、`storage/src/migrations/`、`core/src/memory.rs`、`tools/src/memory.rs`（`MemoryDispatch`）、仕様書のリンク節

**検証**: 展開が必ず 1 ホップで止まること、度数上限を超えないこと、予算超過時に `budget_exhausted=true` になることのテスト（両版）。

**完了条件**: 1 件の記憶から関連を辿れて API / ツールで取得でき、context 注入が予算を超えない。

**リスク**: リンク密度の爆発。**自動生成 (3) は既定オフ**にしてから (1)(2) の効果を見積もる。

#### Phase 4 実施メモ（両版）

計画からの逸脱と判断:

- **`4-a` の `kind` は `related` ではなく仕様 §2 `relation_kinds` の語彙に従う。**
  計画に書いた `related | supersedes | conflicts_with | derived_from` は古い想定で、
  spec §2 の `supersedes` / `contradicts` が正。
- **生成源 (1) を `EXTRACTION_PROMPT` に足さない** — §6 のプロンプト本文は
  `check-memory-parity.py` が両版の**逐語一致**を検査する契約になっている。
  プロンプトを増やすと §6・spec・Desktop の `memory.rs` の 3 か所を同時に改稿する
  契約変更が必要になり、Phase 4 の範囲を超える。代わりに **`jobs._apply` の後に機械生成**
  （抽出行 vs 既存ストアの語彙一致 → `terms()` + `rank()` で上位 3 件を `relates_to`）した。
  spec §4 に同じ方針を書いたので Desktop 版も追随する。
- **リンクの向きを仕様に明記した** — `source` = §3 の列が**指す側**（値）、
  `target` = 列を**持つ行**。§1 の定義（`superseded_by_id` = 「これを置き換えた Memory」）と
  §3 の遷移（「旧側に `superseded_by_id` を立てる」）、§2 の能動態語形から導いた。
  逆に読むと「旧側が新しい側を置き換えた」になり事実に反する。**暗黙のままだと Desktop 版が
  逆向きに実装しうる**ので §4 に根拠つきで書き下した。
- **生成源 (2) を経由する本番経路が無い**（`search(activate=True)` は実装したが、
  context 注入・tool recall・debug 検索のいずれも §3 どおり非激活）。
  明示検索への激活は recency を書き換え既存の順位テストに影響しうるので**判断保留**。
- **context 予算での打ち切りは展開側に持ち込まない** — 硬い打ち切りは §4 の
  度数 5 / 追加 8 の 2 上限、予算は §5.3 の 10% バジェットが結果全体に効く。

#### 4-a' 鏡の 2 種を `link()` が拒否する（仕様へ確定した契約）

**起きたこと**: Docker 版の `service.link()` は `kind=supersedes` / `contradicts` を受けたとき
**§3 の列（`superseded_by_id` / `disputed_by_id`）だけを書き、`lifecycle_state` と revision を
一切触らない**実装だった。Desktop 版も同じで、`link(a, b, "supersedes")` の直後に
`b` は `superseded_by_id` を持ちながら `established` のまま、つまり
`retrievable_lifecycles` に残る。Desktop 側はこれを「エッジは関係についての主張であり、
状態遷移は §3 の仕事。遷移が欲しければ遷移を呼べ」と doc comment に明記していた。

**なぜそうしなかったか**: §3 の図は `supersede ──▶ superseded（superseded_by_id を立てる）`
であり、`supersede` は **`lifecycle_state` と `superseded_by_id` の両方を立てる**操作だと
書かれている。「列が立つ ⇒ 状態も superseded」という不変条件は §3 に明文化されていないが、
**列だけが立つ侵入経路を残した限り、その不変条件は調で保たれている**にすぎない。

**採った解決**: むしろ侵入経路ごと塞いだ。§4 に次を明記し、両版の `link()` が
`ValueError` / `Err` で拒否するようにした。

> §3 の列を書きうるのは §3 の遷移だけ。鏡の 2 種は §3 の遷移が列を書き、`sync_links`
> （§3 → §4 の一方向）がそこからエッジを導出してのみ生まれる。逆向き（§4 → §3）を
> 許すと中間状態が作れる。`link()` が張るのは §4 自身のエッジ
> （`relates_to` / `supports` / `derived_from` / `caused_by`）である。

効果は 3 つ: (i) 鏡の不変条件が「呼び出し側が守る約束」から「構造的に不可能」に変わる、
(ii) `link()` の引数が §2 の 6 語のうち 4 語に絞られ、§3 との境界が 1 行で説明できる、
(iii) エンドポイントのテストが `link()` でエッジを作る代わりに **本物の `supersede` 経路**
で作るようになり、実運用と同一の経路を検査するようになった。

**入れた理由**: Phase 4 の Desktop 実装が「§4 は `link()` が §3 を実行すべきか、
鏡 2 種を禁じるべきか、§4 に明記されていない」と**親に判断を仰いだ**ため。
暗黙のまま 2 版に実装が増えると、後から合わせるコストが跳ね上がる。

**Phase 4 の検証済み数値（両版とも親が再実行して確認）**

| 検証 | Docker | Desktop |
| --- | --- | --- |
| テスト | `5 failed, 570 passed, 1 skipped` | `559 passed / 0 failed` |
| lint / fmt | ruff **14 件**（ベースライン 15 から 1 減、新規 0） | `cargo fmt --check` exit 0 / clippy **既知 4 件のみ** |
| spec | `cmp` で**バイト一致** | 同左 |
| parity | OK | 同左 |

Docker の通過数が `569` → `570` に増えたのは、鏡 2 種の拒否検査を 1 件足した分。
Desktop が `559` のままなのは、拒否検査を既存テスト内に追記し、`link()` で鏡 kind を
作っていた 2 箇所を `MemoryService::supersede` に**置き換えた**（= テスト数は不変）ため。
**両版とも、鏡エッジの検査は `link()` ではなく本物の §3 遷移経路で行う**ようにした。

#### Phase 4 の残課題（§9 に記載）

- **#12** §4 のエッジが **UI から見えない**（参照画面が無い、
  `memory_link_embedding_neighbors` の設定 UI も無い）→ Phase 5
- **#13** §5.3 の `memory_groups` / `weights` / `budget_exhausted` /
  `excluded(reason=budget)` と 10% 予算・600 字切りが Desktop 版に無い
  （Desktop は既存の文字数上限で切っている）→ Phase 5。**Phase 0〜4 のどのフェーズでも
  触っていない既存の差分**なので、§9 に載せたことで初めて可視化した。
- **生成源 (2) を経由する本番経路が無い**（`search(activate=True)` を渡す呼び出しが
  context 注入・tool recall・debug 検索のいずれにも無い）。明示検索への激活は recency を
  書き換え既存の順位テストに影響しうるので**判断保留のまま**。

---

### Phase 5 — 管理 UI・可視化（中）／軸⑤

**目標**: 記憶が増えた今、**「何が保存され、何が context に入ったのか」を人が確認・訂正できる**ようにする。

| 機能 | Docker（`apps/web/src/features/memory/`） | Desktop（`ui/src/features/memory/MemoryPane.tsx`） |
| --- | --- | --- |
| 一覧の装飾 | 出所（`user` / `auto`）、鮮度、役割、リンク数、`superseded` / `disputed` バッジ | 同（既に `role` / `scope` / `lifecycle_state` 列があるため表示側の追加が中心） |
| **今回注入された記憶** | `builder.py:288` が debug に `memory_ids` / `memory_groups` を載せている → `GET /memories-debug/search` 経由で表示。`TraceView.tsx`（262 行）に追記するのが自然 | `retrieval.rs` の `RetrievedBlock` に採用 id を残し、Run の debug イベントに載せて `MemoryPane` に表示 |
| 改訂履歴 | `GET /memories/{key}/revisions` と `POST .../restore/{revision}` は既にある → **diff 表示**を足す | `MemoryService::revisions` / `restore` は既にある → 同 |
| 矛盾解決 | `disputed` 一覧 → 片方を選んで採用（Phase 3 の `resolve_conflict` を実装） | 同 |
| 抽出候補の承認 | `MemoryProcessingJob` の pending を一覧し、採用 / 捨て（Phase 2 の feature flag と接続） | 同 |
| 一括操作 | 全削除（`DELETE /memories` 相当）、エクスポート / インポート（`README.md:24` の将来方針と整合） | 同 |

**UI 言語の一貫性**: `docker/.../app/strings.ts:4` は `memory: "Associative Memory"` のまま（flat 化後の名乗りと噛み合っていない）。**Phase 0 の仕様書で名称と日本語訳を決め、両版で揃える。**

**検証**: Docker は `TraceView.test.tsx` の方式で追加。Desktop は `pnpm ui:typecheck` / `ui:lint` / `ui:test`。

**完了条件**: 会話 1 件について「どの記憶が使われ、なぜそれに当たったか」を UI で追跡できる。

---

#### Phase 5 実施メモ（両版）

**計画からの逸脱** — 計画の機能表には `AssociationsView` / `KnowledgeView` /
`ChipsAutocomplete` が名指しだけで載っていたが、**どれも現状のコードに存在せず、
「何をどう表示するか」の根拠が計画に書かれていない**。名前だけ追うと中身を私が
発明することになるので、**§9 #12（エッジが UI から見えない＋設定 UI も無い）に絞って**
実装した。それ以外の項目（改訂履歴 diff・矛盾解決・一括操作）のうち、API が既に在った
改訂履歴と矛盾解決を先に載せた。

**やったこと（両版とも同じ性質）**

- `TraceView` / `MemoryPane` に §1 の列を**実値で**出す。`superseded` / `disputed` /
  `archived` / `expired` は §2 の語のまま（訳し替えなし）。
- **AssociationsView（新規）**: 両方向のエッジを §4 の形のまま。**`source` / `target` を
  位置ではなくラベルで示す** — §4 の「向き」は仕様に根拠つきで書いてあるので、UI で逆に
  描くとその根拠そのものが UI 上で否定される。
- **矛盾の解決**: Docker は `POST /memories/{key}/resolve` を新設、Desktop は
  `resolve_conflict` command を新設。**どちらも `prefer` を送らない操作を 1 つ用意し、
  §3 の「既定は保存済み側が勝つ」を UI で上書きしない**。却下は `dismiss` → `expired`。
- **設定 UI**: `memory_link_embedding_neighbors` のトグル（**既定 `false` のまま**）。
- UI 名称を `Associative Memory` → **`Memory`** に統一（§1 の呼称と一致）。

**親が Phase 5 の報告を受けて自分で直したもの（サブエージェントの作業外）**

1. **`GET /memories?state=` が受理されるのに無視されていた** — `list_traces(db, owner,
   query, state)` は `state` を受け取って一度も使っていなかった。**フィルタが効かない
   フィルタは無いのより悪い**（「何も無かった」と「絞り方が間違っていた」を呼び出し側が
   区別できない）ので、§2 の語で厳密に絞る + 未知の値は **422** にした。§9 #14。
   - **`pinned` 豁免は効かせない** — §3 の `pinned` は **§2 の検索ゲート**からの救済であって、
     一覧の絞り込みからの救済ではない。`state=archived` と聞かれたら pinned な `archived`
     行も返すのが正解。
2. **`ranking_mode` が context 注入経路の debug に出ていなかった** — `service.search(debug=True)`
   は返しているのに `runtime.recall(debug=True)` が forward していなかったため、
   **1 エディションの中で 2 つの debug 面が食い違っていた**（プローブは答え、注入は答えない）。
   forward する 1 行を足した。§5.3 には載せていない — `weights`（`vector` が `0.0` なら
   lexical のみ）が同じ情報を担っており、両版が持つのはそちらのため。

**報告から採った仕様の追記**

- **§5.3 の `weights` はクエリ側の 1 組**であって候補ごとの内訳ではない、と明記
  （§2.2 が「埋め込みの無い候補は `without_vector` に降格」と決めているため）。
  Desktop が debug に出す実装を聞き取ったところ、**両版ともクエリ側の集合を出していた**
  ので一致。粒度が未規定だと片方が候補単位で出し始める事故が起きるので規定した。
- **§4 に `link_count` を明記**（触れるエッジ数・両端で数える・**0 のときはキー自体を出さない**）。
  両版が同じ数え方をしていたが、仕様に項目が無かった。

**§9 #13 の残り（Phase 5 の範囲外として明示的に残した）**

`memory_groups` / `budget_exhausted` / `excluded(reason=budget)` と §5.3 の 10% 予算・
600 字切りは **Desktop 版に無い**。予算を導入すると Desktop の既存の文字数上限方式が変わり、
`recall_fixture.rs` の順位に波及しうるため、**UI の Phase 5 で予算モデルを変えるのは得策でない**
と判断して触らなかった。`weights` と `memory_ids` は既に計算している値の再掲なので足した。

---

### Phase 6 — 検索品質ベンチと回帰防止（小・横断）

**目標**: 以降の変更で「想起が劣化した」を数値で検出する。

1. **共有 fixture**: `tests/fixtures/memory_recall_cases.json` — `query` / `expected_memory_ids` / `must_not_match` を両版のテストが読む（Phase 1 で作るものを完成させる）。
2. **指標**: recall@5 / recall@8、MRR、ノイズ率（期待外のヒット）。CI では**既知の劣化のみ fail**、改善は情報表示。
3. **規模テスト**: 1 万件での検索レイテンシを Docker で計測（`service.search` の Python 全件スキャンが SQL に移ったことを数値で確認）。Desktop は `cargo test -- --ignored` で heavy テスト扱い。
4. **parity スクリプトの拡張**: ランキング式の重み・抽出プロンプト・tool 契約 JSON の一致を CI で継続検査。

> **状態**: ①②は Phase 1 で作成済み、④は Phase 0 で達成で達成済み。③を含め全項目が Phase 6 で完了。
>
> **計画からの主な逸脱**: synthetic スイートの数値は**両版で一致しない**（下記の横断比較）。
> 共有 fixture 7 ケースの契約値だけが両版で同一なので、**CI ゲートは fixture スイートに限定した**。
> また Desktop は heavy テストを `#[ignore]` ではなく**独立バイナリ**（`cargo run --bin retrieval-bench`）にした —
> どちらも「`cargo test --workspace` を遅くしない」を満たすが、バイナリのほうが手元でそのまま回せて計測しやすい。

#### Phase 6 実施メモ（Docker 版）

`experiments/run_retrieval_bench.py`（新規・実行可）+ `tests/unit/test_retrieval_bench.py`
（3 テスト・DB 不要）。**ネットワーク・LLM API 不要**で 1 コマンドで走る。test ステージの
Dockerfile に `COPY experiments/` を追加したので、契約テストと同じ環境で測れる。

**測り方**: ①共有 fixture 7 ケース（`tests/unit/test_memory_ranking.py` と同じ読み方、
パーサは 1 つ）②シード固定の合成コーパス（`--size` 指定・既定 1000）。指標は
`recall_at_5` / `recall_at_8` / `mrr` / `noise_rate` / `latency_p50_ms` / `latency_p95_ms`
の 6 個で、**定義そのものを JSON の `definitions` に同梱**する（Desktop 版が同じ指標を
移植するときの正）。hybrid モードは**外部 API を呼ばない決定的なスタブ embedding**
（語彙由来の BoW + トピック質量・L2 正規化）で §5 の `vector` 項を通す。

**実測（親が自分の手で再現・報告値と一致を確認）**

| mode | size | synth recall@5 | recall@8 | MRR | noise | p50 | p95 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| lexical | 1000 | 0.6000 | 1.0000 | 1.0000 | 0.3750 | 8.33 ms | 18.10 ms |
| hybrid | 1000 | 0.7000 | 1.0000 | 1.0000 | 0.3750 | 20.79 ms | 31.04 ms |
| lexical | 10000 | 0.6000 | 1.0000 | 1.0000 | 0.3750 | 16.66 ms | 26.43 ms |
| hybrid | 10000 | 0.7000 | 1.0000 | 1.0000 | 0.3750 | 42.31 ms | 54.54 ms |

fixture スイートは 4 実行とも `recall@5/@8 = 1.0000`, `MRR = 1.0000`, `noise = 0.4167`。

**このベンチが答えたこと**

1. **hybrid は recall@5 で 0.60 → 0.70。** §5 の `vector` 項が実際に効いている数値。
2. **一方 noise は両モード 0.3750 で差が付かなかった。** 1 クエリあたり
   `5 expected + 3 distractor = 8 行`で top-8 がちょうど埋まる設計のためフィラーの
   混入余地が無い。**ノイズでの優劣はこのコーパスでは未観測**（recall 側で判定でき、
   `|expected| ≤ 5` で recall@5 の上限が 1.0 になることを優先した結果）。
3. **10k 件で p50 8.33 → 16.66 ms（lexical）。** ただし `CANDIDATE_LIMIT = 2000` の
   ため評価対象は**最新 2000 行**。ベンチは期待セットを「1〜150 日」、フィラーを
   「180〜899 日」に置いて**どの `--size` でも評価対象が窓内に入る**よう調整した
   （＝ recall はランキングの能力を、レイテンシは行数とソートを測る）。
   **コーパス側で調整した事実はここで明記する** — 挙動変更ではない。

**CI に入れるなら（採否は判断保留）**

時間は入れない。品質指標は決定的だがレイテンシはランナーに依存し、締めると赤が飛ぶ。
代わりに 2 行構成にする:

```bash
python experiments/run_retrieval_bench.py --suite fixture \
  --min-recall5 1.0 --min-recall8 1.0 --min-mrr 1.0 --max-noise 0.5
python experiments/run_retrieval_bench.py --suite synthetic --size 1000 \
  --min-recall5 0.6 --max-noise 0.4
```

`--max-noise 0.4` が実測 0.3750（= 3/8）からの**実用上の境界**なのは、返却が常に 8 件で
次の離散値が 4/8 = 0.5 だから — **ノイズが 1 行でも漏れれば 0.5 > 0.4 で落ちる**。
0.5 にすると最初の漏れを見逃す。しきい値は**走った suite 全部**に掛かるので、
fixture 用と synthetic 用は 2 コマンドに分ける（上の構成どおり）。

**ベンチ側で自己検証して直した欠陥**: `--max-p95-ms` が「ストップウォッチを持たない
fixture スイート」にも適用され、正当な CI コマンドが誤って exit 2 になっていた。
**測れない指標は gate から外す**ようにした（`--suite fixture` で `--max-p95-ms` を
付けると該当 suite に測定が無いためスキップされる）。

**既知の実装差（ベンチは測っていないもの）**

- `must_not_match` は **fixture に存在しない**（fixture 変更禁止のため導出 =
  そのケースの全候補 − expected）。よって fixture の `noise = 0.4167` は
  **定義の導出由来の数値**であって品質の絶対値ではない。比較は同スイート内でのみ有効。
- 合成コーパスの語彙は latin のみで、`terms()` の **CJK bigram 経路は exercise しない**
  （unit テスト側で担保）。意図は `|Q ∩ T| / |Q|` を算術として確定させること。
- **スタブ embedding は語彙由来**なので、hybrid vs lexical の差は「§5 の `vector` 項の
  効き方」の証明であって**実プロバイダの品質の証明ではない**。

**仕様側の欠落（報告のみ・未対応）**: spec に「1 万件での検索レイテンシ」に対応する行が
**無い**（`grep -E '万件|レイテンシ|ベンチ|bench'` が 0 件）。§6 は自動形成の節なので
ここには書かれていない。エンジニアリングの観測値であって両版の挙動契約ではないので、
**仕様ではなくこのロードマップに記載する**のが筋と判断した。

#### Phase 6 実施メモ（Desktop 版）

`core/src/bin/retrieval_bench.rs`（1366 行・独立バイナリ）。呼び出しは
`cargo run -p mixd-core --bin retrieval-bench -- --size 1000 --mode lexical`。

**読む側を 1 つにした**: `core/src/recall_fixture.rs`（新規 lib モジュール）を追加し、
`core/tests/recall_fixture.rs` とベンチの**両方**がそこから読む形にした。**重要**:
そのモジュールは **`crate::retrieval::rank_score`（本番のスコアラー）を呼んでいて**、
§5 の式をテスト/ベンチ用に再実装していない。二重実装は「1 つの契約に対する 2 つの解釈」
になり、最初のフィールド改名で乖離する（この意図はモジュールの docstring にも書かれた）。
フリーズ指定の `core/tests/recall_fixture.rs` は**主張を一切弱めていない**ことを親が中身で確認 —
全ケースを `raw["cases"]` として走査し、スコアを小数 4 桁で完全一致、候補数を宣言的に検査。

**測る経路が Docker 版と違う**: Desktop は `retrieve_memories` /
`retrieve_memories_hybrid`（**`activate=true`・実稼働経路**）を対称に測る。
Docker 版は非活性の debug search 経路。指標の名前と定義は同じだが、**経路が違うので
synthetic の数値が一致する保証は無い**（下記の横断比較がその実例）。

`cargo test --workspace` は **563 passed / 0 failed / 0 ignored** のまま、ベンチは
`running 0 tests` の独立バイナリ — 計測をテストに混ぜていない。

**実測（`--size 1000` を親が再現・報告値と一致）**

| mode | size | recall@5 | recall@8 | mrr | noise | p50 | p95 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| lexical | 1000 | 0.6000 | 0.9250 | 1.0000 | 0.5793 | 6.97 ms | 8.76 ms |
| lexical | 10000 | 0.6000 | 0.9250 | 1.0000 | 0.5793 | 57.68 ms | 59.86 ms |
| hybrid | 1000 | 0.7000 | 1.0000 | 1.0000 | 0.3750 | 22.32 ms | 23.03 ms |
| hybrid | 10000 | 0.7000 | 1.0000 | 1.0000 | 0.3750 | 209.77 ms | 275.04 ms |

しきい値の退出コードも実測: `--min-recall5 0.65` は lexical で **exit 1**（regression）、
hybrid で **exit 0**。`--suite fixture --max-p95-ms 1000` は **exit 2**（not-measured）。
Docker 版と同じく「測れない指標は gate から外す」挙動。

#### 横断比較（Phase 6 の最大の収穫）

**fixture スイートは両版で完全一致**する: `recall@5/@8 = 1.0000`、`mrr = 1.0000`、
`noise = 0.4167`（4 実行すべて）。**共有 fixture が本当に契約として働いている証拠。**

一方 synthetic は**一致しない**:

| synthetic 1000 | Docker lex | Desktop lex | Docker hyb | Desktop hyb |
| --- | --- | --- | --- | --- |
| recall@5 | 0.6000 | 0.6000 | 0.7000 | 0.7000 |
| recall@8 | 1.0000 | **0.9250** | 1.0000 | 1.0000 |
| mrr | 1.0000 | 1.0000 | 1.0000 | 1.0000 |
| noise | 0.3750 | **0.5793** | 0.3750 | 0.3750 |

**読み方（ここで結論を誤ると危険）**

1. `recall@5` と `mrr` が両版で一致するのは、**合成コーパスの構造が偶然そろった**から
   （1 クエリあたり `5 expected + 3 distractor`、`|expected| ≤ 5`）であって、
   **製品同士が数値で一致している証明ではない。**
2. 差が出たのは **lexical のみ**。Desktop のコーパスは **§4 の `memory_links` を持つ**ため、
   lexical 検索が 1 ホップ展開で **cross-topic のデコイ行を取り込む**。Desktop 側が
   **DB のリンク表で実証**した。Docker 版のコーパスはリンクを作らない。
3. つまり **Desktop は hybrid が 4 指標すべてで lexical を上回る**が、Docker では
   `recall@5` のみ改善する。**これは製品の優劣ではなくコーパスの差**である。
   「Desktop の lexical が劣っている」と読んではならない。
4. **レイテンシの横断比較は無意味**: Desktop は **debug ビルド**（10〜50 倍遅い）かつ
   10k 件を**全部ロード**（`embedded_rows 10000`）、Docker は `CANDIDATE_LIMIT = 2000`
   で候補窓が止まる。Desktop の `57.68 ms` / `209.77 ms` は debug の値であり、
   リリースビルドなら桁が変わる。

**CI に採用した（親の決定・`.github/workflows/ci.yml` 編集）**

共有 fixture ゲートを**両 job に追加**した。**コマンドは一字一句同じ**:

```bash
--suite fixture --min-recall5 1.0 --min-recall8 1.0 --min-mrr 1.0 --max-noise 0.5
```

- **Docker** → `docker-backend` にステップ追加。test ステージに `COPY experiments/` 済み。
  **DB 不要**（`alembic upgrade head` なしで exit 0 を親が実測確認）。
- **Desktop** → `desktop-rust` にステップ追加、**`if: matrix.os == 'ubuntu-latest'` に限定**。
  seed 固定なので合成コーパスの値は OS で変わらず、3 OS マトリクスで買うものは無い。
  `cargo test --workspace` が既にビルド済みなので増分実行で済む。
- **時間は gate に入れない** — 品質指標は決定的だがレイテンシはマシン依存。
  合成コーパスのゲートも入れていない（**両版で数値が一致しない**のが理由）。
  あれを横断ゲートにすると、片方だけ追う形になる。

#### Phase 6 の残課題

- **合成コーパスに §4 リンクを持たせるかどうかを統一する**: 現状 Docker はリンク無しだが、
  Desktop は持つ。展開がノイズにどう効くかを両版で同じ条件で測りたければ、
  **コーパス生成側を仕様に書く**必要がある（`--size` 以外の生成規則が両版で独立発明されているため）。
- **CJK の合成コーパス**: 両版とも latin のみで、`terms()` / unigram-bigram の
  日本語経路をベンチで exercise していない（unit テスト側で担保）。
- **実 embedding プロバイダの品質は未測定**: ハイブリッドは語彙由来のスタブなので、
  §5 の `vector` 項の**効き方**を示すにとどまる。

---

## 4. 横断リスク

| リスク | 対処 |
| --- | --- |
| **`models.py:200` の "Do not re-add them" と依頼の矛盾** | Phase 0 で明示的に判断を記録し、注記を書き換える。黙って置き換えない |
| 両版のスキーマ乖離が深まる | 仕様書 1 本 + parity スクリプト + 共有 fixture。`greeting-parity` の成功例をそのまま踏襲 |
| 未コミット WIP との混在 | Phase 0 でコミット分離。以降は記憶まわりを独立コミットに |
| pgvector image 切替の影響 | Phase 1 の冒頭にスパイクを置き、移行 → 起動を 1 回実証してから本実装 |
| 抽出ノイズの急増（Phase 2） | feature flag 既定オフ。Phase 3 の統合とセットで有効化 |
| `embed` 非対応プロバイダ（Anthropic 等） | `NotSupported` → lexical の自動フォールバックを契約として固定 |
| Desktop の `Provider` trait 変更が全実装に波及 | 既定実装を先に置き、段階的に実装を追加 |
| Knowledge 注入が Docker で未接続（`retrievers.py:82`） | 「記憶のほか」の範囲として Phase 0 で明記し、後続タスク化 |

---

## 5. 推奨する着手順（要決断ポイントつき）

1. **決断 A**: データモデルを A / B / C のどれにするか（**B 推奨**）→ これが決まらないと Phase 1 以降のマイグレーションが書けない
2. **決断 B**: pgvector を採るか、アプリ内 cos sim で始めるか（**まずはアプリ内 cos sim、実測後に pgvector** が安全）
3. Phase 0 実行（WIP 分離 → 仕様書 → parity スクリプト）
4. Phase 1 と Phase 2 を並行着手
5. Phase 3 → Phase 4 → Phase 5 の順に統合
6. Phase 6 を最後にまとめて適用（ただし共有 fixture は Phase 1 で先に作る）
