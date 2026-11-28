# Memory共有パック v1

Memory 項目をマシン・workspace 間で持ち運ぶための単一 JSON 形式。
正文は `pack.schema.json`。設計背景は `docs/desktop/b10-knowledge-embedding-design.md` §5。

## 形式

- ファイル: UTF-8 JSON、拡張子 `.memory-pack.json`、トップレベル `format: "mixd-memory-pack/1"`。
- `meta`: `exported_at`（RFC3339）、`exporter`（表示名・任意）、`source`（`desktop` 等）、`count`。
- `items[]`: 各 Memory 1件。`id` は pack 内ローカル ID（`m1`, `m2`, …）。
  `title/content/role/scope/source_kind/lifecycle_state/verification/confidence/salience/strength/pinned`
  のみ。`owner_id/task_id/embedding/activation_count` 等の環境依存値は持たない。
- `links[]`: pack 内参照。`{source, target, kind, weight}`。`source/target` は `items[].id`。
- v1 で含めないもの: ベクトル（import 後に再埋め）、Knowledge 文書（v2 検討）、`task` scope（既定で export 対象外）。

## ルール

1. export 前に全件へ現行の認証情報チェック（`MemoryService::is_sensitive` 相当）を適用し、該当があれば除外＋件数を報告する。
2. import 時は ID を捨てて新規 UUID を振る。`lifecycle_state` は `candidate` に倒す（`--trust` 明示時のみ `established` 維持）。
3. 同一内容は `create_or_reinforce` と同義で reinforce（複製しない）。
4. `--dry-run` で「新規 N / reinforce M / 除外 K」を先に表示する。
5. 1 pack 上限 500 件（超えたら分割 export）。
