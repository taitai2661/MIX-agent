# 監査ログ・ポリシー仕様書（A5）

> 状態: **実装同期 (2026-11-23)**。実装は `apps/desktop/storage`（`audit_log` テーブル + `audit.rs` + `policy.rs`）と
> `apps/desktop/tools`（`execute.rs` からの記録）を参照。本書は Desktop 版の契約を定める。

---

## 0. 目的

- すべての Tool 呼び出しと承認決定を**改ざん検出可能な追記専用ログ**に残す。
- ワークスペース直下の `.mix/policy.toml` で既定ポリシー（allow / ask / deny）を宣言的に管理する。
- 既存の承認フロー（`tools/src/approval.rs` + `tools/src/execute.rs` の `ToolExecutor`）と
  既存の権限エンジン（`tools/src/permission.rs` の scope/pattern 解決）を壊さずに連携する。

非目的:

- 暗号化そのものは本仕様の範囲外。監査ログを含む SQLite DB は**平文**で、`Storage::open` が
  起動時に警告を出す。SQLCipher は未組み込み（Keychain の DB キーは将来用に staged のみ）。
  保護は 0600 のファイル権限と OS のディスク暗号化（FileVault / BitLocker / LUKS）に依存する。
  API key 等の秘密値は Keychain 管理のマスター鍵で暗号化され、`arguments` は保存前に redact
  されるが、DB 全体の at-rest 暗号化ではない。
- ネットワーク越境ポリシー（egress allowlist）は `EgressPolicy` が引き続き担当する。

---

## 1. 用語

| 用語 | 意味 |
| --- | --- |
| Tool 呼び出し | `dispatch_tool` が 1 回の model tool-call に対して行う実行単位（`tool_calls.id`） |
| 承認 | `approvals.id` に対応する Allow / Deny 決定。`remember` / `remember_session` を含む |
| 監査エントリ | `audit_log` の 1 行。Tool 呼び出し・承認解決のたびに 1 行以上追加される |
| ポリシーファイル | ワークスペース直下 `.mix/policy.toml`。TOML で既定決定と例外規則を宣言する |

---

## 2. `.mix/policy.toml`

### 2.1 配置と優先度

- 探索順: (1) `.mix/policy.toml` → (2) 規定値（すべて `ask` 相当）。
  `storage::policy::policy_path(data_dir)` は `<data_dir>/.mix/policy.toml` を返す。アプリ起動への
  組み込みは未完了（§7）のため、「実行ワークスペース直下」との対応は配線時に確定する。
- ファイルが無い・読めない・パース失敗の場合は**警告ログ + fail-closed（`ask`）**とし、起動や Run を止めない。
- DB の `permission_rules`（UI の "Always Allow" 等）が常にファイルより優先される。
  ファイルは「既定値の配布物」、DB は「ユーザーの上書き」と位置づける。

### 2.2 スキーマ（v1）

```toml
version = 1

[defaults]
# どの rule にも一致しなかった場合の決定。allow | ask | deny。既定 ask。
decision = "ask"
# 監査ログを残すか。既定 true。false は開発用であり警告が出る。
audit_enabled = true

# 例外規則は上から順に評価し、最初に一致したものが勝つ。
# DB ルール（precedence bucket, execute.rs 参照）の前段として評価される。
[[rules]]
tool = "terminal"          # 必須。Tool id（"*" で全 Tool）
decision = "ask"           # 必須。allow | ask | deny
scope = "workspace"        # 任意。global | workspace。既定 workspace
pattern = "cargo test*"    # 任意。glob。対象は Tool の pattern_target と同じ
workspace = ""             # 任意。workspace UUID。空はその scope の全域

[[rules]]
tool = "files"
decision = "deny"
pattern = "**/.env*"

[audit]
# true で起動時にチェーン検証を実行し、結果を StartupCheck で返す。既定 false。
verify_on_startup = false
# 保持期間（日）。0 = prune しない（既定）。> 0 で起動時に古い行を archive へ退避。
retention_days = 0

[approval]
# 承認プロンプトの TTL（秒）。execute.rs の DEFAULT と同じ範囲に clamp される。
ttl_secs = 86400
```

### 2.3 バリデーション規則

1. `version` は `1` のみ受領。それ以外はエラー（将来の拡張余地）。
2. `decision` は `allow | ask | deny` のみ。未知値はエラー。
3. `scope` は `global | workspace` のみ（`session` はメモリ専用のためファイルでは不可）。
4. `pattern` は `glob::Pattern` として有効であること（`is_valid_pattern` と同一基準）。
5. `tool = "*"` は全 Tool に一致するワイルドカード。`tool` 空文字は不可。
6. `rules` は最大 200 件。それ以上はエラー（巨大ファイルの誤読防止）。

### 2.4 解決順序（全体）

```
session rules (memory)
  → workspace+pattern / workspace / global+pattern / global (DB, execute.rs)
  → .mix/policy.toml rules (上から順)
  → Tool::decision_for (Tool 既定)
  → defaults.decision (既定 ask)
```

- `deny` は fail-closed: いずれかの層で `deny` が確定したら後段を見ずに拒否する。
- dangerous コマンド昇格（`risk::assess_tool_call` が dangerous かつ決定が allow → ask）は
  ポリシー解決の**後**に適用され、ファイルで allow されていても昇格する。

### 2.5 Org 配布ポリシー（署名検証は未実装）

`[org] expected_signer` / `signature` はスキーマ上受理するが、`policy::verify_org_signature` は
**fail-closed の足場**である。Ed25519 の依存クレートはなく、公開鍵／fingerprint をファイル外
（信頼できる env またはビルド定数）から与える設計もまだない。現状は Org 配布を主張する
（`expected_signer` または `signature` を持つ）ファイルをすべて警告付きで拒否する。
旧来の `sha256(body + signer)` 比較は signer 文字列も同じ編集可能な TOML にあり偽造可能なため、
正しい検証方式として扱わない（実装・本書のいずれにも有効な機構として記載しない）。

---

## 3. `audit_log` テーブル

### 3.1 DDL（`storage/src/migrations/0008_audit_log.sql`）

```sql
CREATE TABLE IF NOT EXISTS audit_log (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    owner_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    tool_call_id TEXT NOT NULL,
    approval_id TEXT,
    tool_name TEXT NOT NULL,
    arguments TEXT NOT NULL DEFAULT '{}',
    decision TEXT NOT NULL,          -- allow | deny | ask | succeeded | failed | denied | awaiting_approval | approved | denied_by_user | expired
    prev_hash TEXT NOT NULL,
    entry_hash TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
```

- `seq` は挿入順の連番（チェーン順序の基準）。
- `prev_hash` は直前行の `entry_hash`。先頭行は `"GENESIS"`。
- `entry_hash = hex(sha256(canonical_json))`。`canonical_json` は次の 10 フィールドを**この順**で
  並べた JSON 配列を `serde_json::to_vec` で符号化した UTF-8 バイト列:
  `[prev_hash, owner_id, id, run_id, tool_call_id, approval_id, tool_name, arguments, decision, created_at]`。
  `arguments` は JSON 文字列、`approval_id` は `Option<&str>` で欠損時は `null`（空文字とは区別）。
  区切り文字を使わないため、`|` を含む引数やツール名で異なる 2 行が同じハッシュになることはない。
  フィールド順は契約の一部で、変更すると既存チェーンが全て無効になる。
  実装は `storage/src/audit.rs::entry_hash` が唯一の正本。
- インデックス（`run_id` / `tool_call_id` / `tool_name` / `created_at`）と
  `audit_log_archive`（後述、append-only トリガ付き）も migration 0008 が作成する。
- 追記専用の強制はトリガで実施する:

```sql
CREATE TRIGGER IF NOT EXISTS audit_log_no_update BEFORE UPDATE ON audit_log
BEGIN SELECT RAISE(ABORT, 'audit_log is append-only'); END;
CREATE TRIGGER IF NOT EXISTS audit_log_no_delete BEFORE DELETE ON audit_log
BEGIN SELECT RAISE(ABORT, 'audit_log is append-only'); END;
```

### 3.2 不変条件

1. `INSERT` のみ許可。`UPDATE` / `DELETE` はトリガが ABORT する。
2. `entry_hash` は `prev_hash` を含めて計算されるため、1 行の改ざんは
   それ以降の全行の検証失敗として検出できる。
3. `retention_days = 0`（既定）では prune しない。`> 0` のときだけ
   `policy::prune_from_policy` → `audit::prune_old_entries` が、古い行を
   `audit_log_archive`（独自の append-only トリガ付き）へ写してから live 表から削除する。
   トランザクション中に DELETE トリガを一時的に外すのはこの経路のみ。archive → live と
   `GENESIS` から切れ目なく検証でき、全行が archive へ移った場合は
   `pruned_checkpoint` 行を残してチェーン先頭を維持する。
4. 監査書き込みの失敗は Tool 実行を止めない（warn ログのみ）。監査が無いことより
   Run が止まることの方が被害が大きいため。検証失敗も現状は `tracing::warn` のみで、
   UI 通知は未実装（§7）。
5. 同時追記はプロセス内 mutex と DB の `BEGIN IMMEDIATE` で直列化し、複数の writer が
   同じチェーン先頭を読まないようにする。Verifier は O(1) メモリで行をストリームする。

### 3.3 検証

- `audit::verify_chain(pool)` が archive → live の順に `seq` 昇順で全行をストリームし、
  各行の再計算ハッシュと `prev_hash` 連鎖を検査する。先頭は `GENESIS`、archive の末尾から
  live が続く。戻り値は `Ok(verified_count)` / 最初の破断位置（`prev_hash` または
  `entry_hash` 不一致、seq 付き）の `Err`。
- 起動時検証は既定 OFF（`verify_on_startup = false`）。`policy::verify_on_startup(pool, path)`
  は `load_policy_file` → retention prune → 任意の `verify_chain` を実行し、
  `StartupCheck { pruned_rows, verification_requested, verified_rows, failure }` を返す。
  関数は失敗を投げず `failure` に記録する。呼び出し側で警告を出す想定だが、アプリ起動への
  組み込みは未完了（§7）。既にポリシーを読んでいる場合は
  `verify_on_startup_with_policy(pool, &policy)` を使う。

---

## 4. approval.rs との連携

### 4.1 記録点（`tools/src/execute.rs`）

| タイミング | `decision` 値 | 備考 |
| --- | --- | --- |
| `record_tool_call(awaiting_approval)` | `awaiting_approval` | プロンプト表示前。`approval_id` はまだ NULL の場合あり |
| `record_tool_call(succeeded/failed/denied)` | 同値 | 実行結果の確定時 |
| `resolve_approval(Approve)` | `approved` | `approvals` 行更新と同一トランザクションではない（best-effort） |
| `resolve_approval(Deny)` | `denied_by_user` | 同上 |
| `sweep_expired` | `expired` | TTL 切れで自動失効した行のみ |

- いずれも `audit::append_entry` を best-effort で呼び、失敗時は
  `tracing::warn!(audit_write_failed)` のみで呼び出し元にエラーを返さない。
- `arguments` は保存前に `redact::redact_arguments` 済みの表示用コピーを記録する
  （秘密値が監査ログに平文で残らないようにする）。

### 4.2 シーケンス図

```
dispatch_tool
  ├─ resolve_permission (DB → policy.toml → Tool既定)
  ├─ risk 昇格 (dangerous + allow → ask)
  ├─ [Ask の場合] record_tool_call(awaiting_approval) + audit(awaiting_approval)
  ├─ request_approval_graded → UI prompt
  ├─ resolve_approval → approvals更新 + audit(approved/denied_by_user)
  ├─ Tool dispatch
  └─ record_tool_call(succeeded/failed/denied) + audit(同値)
```

---

## 5. API（Rust）

```rust
// storage/src/audit.rs
pub struct AuditEntry { /* seq, id, owner_id, run_id, tool_call_id, approval_id, tool_name, arguments: Value, decision, prev_hash, entry_hash, created_at: DateTime<Utc> */ }
pub fn entry_hash(prev_hash: &str, owner_id: &str, id: &str, run_id: &str, tool_call_id: &str, approval_id: Option<&str>, tool_name: &str, arguments: &str, decision: &str, created_at: &str) -> String;
pub async fn append_entry(pool: &DbPool, owner_id: &str, run_id: &str, tool_call_id: &str, approval_id: Option<&str>, tool_name: &str, arguments: &serde_json::Value, decision: &str) -> anyhow::Result<AuditEntry>;
pub async fn verify_chain(pool: &DbPool) -> anyhow::Result<u64>;
pub async fn entries_for_run(pool: &DbPool, run_id: &str, limit: i64) -> anyhow::Result<Vec<AuditEntry>>;
pub async fn prune_old_entries(pool: &DbPool, retention_days: u32) -> anyhow::Result<u64>;

// storage/src/policy.rs
pub struct PolicyFile { /* version, default_decision, audit_enabled, rules, verify_on_startup, retention_days, approval_ttl_secs, org_signer */ }
pub fn parse_policy_toml(text: &str) -> Result<PolicyFile, PolicyError>;
pub fn load_policy_file(path: &Path) -> PolicyFile; // 失敗時は既定 + warn
pub fn decide_from_file<'a>(file: &'a PolicyFile, tool: &str, pattern_target: Option<&str>) -> Option<&'a PolicyRule>;
pub fn policy_path(data_dir: &Path) -> PathBuf;         // <data_dir>/.mix/policy.toml
pub fn evaluate_tool_policy(policy: &PolicyFile, tool: &str, pattern_target: Option<&str>) -> PolicyDecision;
pub fn verify_org_signature(body_without_signature: &str, file: &PolicyFile, signature: Option<&str>) -> bool; // 未実装: Org 主張は常に拒否
pub struct StartupCheck { /* pruned_rows, verification_requested, verified_rows, failure: Option<String> */ }
pub async fn verify_on_startup(pool: &DbPool, path: &Path) -> StartupCheck;
pub async fn verify_on_startup_with_policy(pool: &DbPool, policy: &PolicyFile) -> StartupCheck;
pub async fn prune_from_policy(pool: &DbPool, policy: &PolicyFile) -> anyhow::Result<u64>;
```

---

## 6. テスト

- `storage/tests/audit_chain.rs`:
  - 2 行 append → `verify_chain` が 2 を返す。同時 append も断鎖しない。
  - `UPDATE` / `DELETE` が ABORT される。保存済み行の再計算で改ざんを検出する。
  - `retention_days > 0` の prune で archive へ退避しても検証が続くこと、全行を移しても
    `pruned_checkpoint` でチェーン先頭が維持されること。
- `audit.rs` 単体: ハッシュが owner_id / approval_id / decision の違いで変わること、
  `approval_id` の `null` と空文字が別であること、`|` を含むフィールドの衝突しないこと。
- `storage` 単体: `policy::parse_policy_toml` の正常系・異常系（未知 decision、壊れた glob、
  version 不一致）、`verify_org_signature` が自己署名風の Org 主張を拒否すること。
- `tools/tests/permissions.rs` 既存ケースはそのまま通る（監査追加は best-effort のため）。

---

## 7. 今後の課題

- [ ] `verify_on_startup` のアプリ起動シーケンスへの組み込みと検証失敗の UI 通知。
      storage 側の関数は実装済みだが、アプリからはまだ呼ばれていない（現状は `tracing::warn` のみ）。
- [ ] archive の長期保管・エクスポート方針。`retention_days > 0` の prune 関数自体は実装済み
      （checkpoint 行でチェーン先頭を維持）。
- [ ] DB の at-rest 暗号化（SQLCipher）。現状は平文 + `Storage::open` の起動時警告 + 0600。
- [ ] entry 署名（秘密鍵による否認防止）。現行はハッシュチェーンのみ。
- [ ] Org 配布の真正性検証（Ed25519 + 信頼できる out-of-band 公開鍵）。現状
      `verify_org_signature` は fail-closed の足場（§2.5）。
- [ ] `.mix/policy.toml` の workspace スコープ解決（`workspace = "<uuid>"` 指定時の照合テスト）。
