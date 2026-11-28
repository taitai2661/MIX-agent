//! Audit chain integration tests (A5).
//!
//! - Two appends verify as a chain of length 2.
//! - `UPDATE` / `DELETE` on `audit_log` are rejected (append-only triggers).
//! - Concurrent appends form one unbroken chain (no duplicate chain heads).
//! - Retention pruning archives old rows and keeps the chain verifiable.
//! - A tampered recomputation is detected by `entry_hash`.

use mixd_storage::audit;
use mixd_storage::records::SINGLE_USER_ID;
use mixd_storage::Storage;

/// Fresh database per test, following the pattern of `db::tests`: a UUID
/// directory under the system temp dir, removed at the end of the test.
async fn open() -> (Storage, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("mixd-audit-{}", uuid::Uuid::new_v4()));
    let storage = Storage::open(&dir).await.expect("open");
    (storage, dir)
}

async fn close(storage: Storage, dir: std::path::PathBuf) {
    drop(storage);
    let _ = tokio::fs::remove_dir_all(&dir).await;
}

#[tokio::test]
async fn two_appends_verify_as_a_chain_of_two() {
    let (storage, dir) = open().await;
    let pool = storage.pool();
    audit::append_entry(
        pool,
        SINGLE_USER_ID,
        "run-1",
        "call-1",
        None,
        "terminal",
        &serde_json::json!({"command": "cargo test"}),
        "awaiting_approval",
    )
    .await
    .expect("first append");
    audit::append_entry(
        pool,
        SINGLE_USER_ID,
        "run-1",
        "call-1",
        Some("approval-1"),
        "terminal",
        &serde_json::json!({"command": "cargo test"}),
        "approved",
    )
    .await
    .expect("second append");
    assert_eq!(audit::verify_chain(pool).await.expect("verify"), 2);
    let entries = audit::entries_for_run(pool, "run-1", 10)
        .await
        .expect("entries");
    assert_eq!(entries.len(), 2);
    // Newest first, chained: second row's prev is the first row's hash.
    assert_eq!(entries[0].decision, "approved");
    assert_eq!(entries[0].prev_hash, entries[1].entry_hash);
    assert_eq!(entries[1].prev_hash, audit::GENESIS_HASH);
    close(storage, dir).await;
}

#[tokio::test]
async fn concurrent_appends_form_one_unbroken_chain() {
    let (storage, dir) = open().await;
    let pool = storage.pool();
    const N: i64 = 16;

    let mut tasks = Vec::new();
    for i in 0..N {
        let pool = pool.clone();
        tasks.push(tokio::spawn(async move {
            audit::append_entry(
                &pool,
                SINGLE_USER_ID,
                "run-concurrent",
                &format!("call-{i}"),
                None,
                "terminal",
                &serde_json::json!({"i": i}),
                "succeeded",
            )
            .await
            .expect("append")
        }));
    }
    let mut seqs = Vec::with_capacity(N as usize);
    for task in tasks {
        seqs.push(task.await.expect("join").seq);
    }
    seqs.sort_unstable();
    assert_eq!(
        seqs,
        (1..=N).collect::<Vec<_>>(),
        "no duplicate chain heads"
    );

    // The head must be a single line: every row links to the one before it.
    assert_eq!(
        audit::verify_chain(pool).await.expect("chain verifies"),
        N as u64
    );
    close(storage, dir).await;
}

#[tokio::test]
async fn update_and_delete_are_rejected() {
    let (storage, dir) = open().await;
    let pool = storage.pool();
    audit::append_entry(
        pool,
        SINGLE_USER_ID,
        "run-1",
        "call-1",
        None,
        "files",
        &serde_json::json!({"action": "read"}),
        "succeeded",
    )
    .await
    .expect("append");
    let updated = sqlx::query("UPDATE audit_log SET decision = 'x'")
        .execute(pool)
        .await;
    assert!(updated.is_err(), "UPDATE must be rejected by the trigger");
    let deleted = sqlx::query("DELETE FROM audit_log").execute(pool).await;
    assert!(deleted.is_err(), "DELETE must be rejected by the trigger");
    // The chain still verifies afterwards.
    assert_eq!(audit::verify_chain(pool).await.expect("verify"), 1);
    close(storage, dir).await;
}

#[tokio::test]
async fn prune_archives_old_rows_and_keeps_chain_verifiable() {
    let (storage, dir) = open().await;
    let pool = storage.pool();

    // Insert rows with controlled timestamps (direct SQL so retention can be
    // exercised without waiting a day). Each hash covers owner + approval.
    let old = "2020-01-01T00:00:00Z";
    let recent = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%fZ").to_string();
    let h1 = audit::entry_hash(
        audit::GENESIS_HASH,
        SINGLE_USER_ID,
        "id1",
        "run",
        "call1",
        None,
        "terminal",
        "{}",
        "succeeded",
        old,
    );
    let h2 = audit::entry_hash(
        &h1,
        SINGLE_USER_ID,
        "id2",
        "run",
        "call2",
        Some("approval-2"),
        "terminal",
        "{}",
        "approved",
        old,
    );
    let h3 = audit::entry_hash(
        &h2,
        SINGLE_USER_ID,
        "id3",
        "run",
        "call3",
        None,
        "terminal",
        "{}",
        "succeeded",
        &recent,
    );
    for (id, call, approval, decision, hash, ts) in [
        ("id1", "call1", None, "succeeded", &h1, old),
        ("id2", "call2", Some("approval-2"), "approved", &h2, old),
        ("id3", "call3", None, "succeeded", &h3, recent.as_str()),
    ] {
        sqlx::query(
            "INSERT INTO audit_log \
             (id, owner_id, run_id, tool_call_id, approval_id, tool_name, arguments, decision, prev_hash, entry_hash, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, 'terminal', '{}', ?6, ?7, ?8, ?9)",
        )
        .bind(id)
        .bind(SINGLE_USER_ID)
        .bind("run")
        .bind(call)
        .bind(approval)
        .bind(decision)
        .bind(if id == "id1" {
            audit::GENESIS_HASH
        } else if id == "id2" {
            h1.as_str()
        } else {
            h2.as_str()
        })
        .bind(hash)
        .bind(ts)
        .execute(pool)
        .await
        .expect("seed row");
    }
    assert_eq!(
        audit::verify_chain(pool).await.expect("pre-prune verify"),
        3
    );

    // 0 days disables pruning entirely (the fail-closed default).
    assert_eq!(
        audit::prune_old_entries(pool, 0).await.expect("no prune"),
        0
    );
    assert_eq!(audit::prune_old_entries(pool, 1).await.expect("prune"), 2);

    let live: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_log")
        .fetch_one(pool)
        .await
        .expect("count live");
    let archived: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_log_archive")
        .fetch_one(pool)
        .await
        .expect("count archive");
    assert_eq!(
        live, 1,
        "the recent row remains; a checkpoint is only needed when all rows are pruned"
    );
    assert_eq!(archived, 2);

    // Archive first, then live: one continuous chain, still fully verified.
    assert_eq!(
        audit::verify_chain(pool).await.expect("post-prune verify"),
        3
    );

    // A later append chains off the checkpoint, not GENESIS.
    let appended = audit::append_entry(
        pool,
        SINGLE_USER_ID,
        "run",
        "call4",
        None,
        "terminal",
        &serde_json::json!({}),
        "succeeded",
    )
    .await
    .expect("append after prune");
    assert_ne!(appended.prev_hash, audit::GENESIS_HASH);
    assert_eq!(
        audit::verify_chain(pool)
            .await
            .expect("verify after append"),
        4
    );

    // Append-only protection was restored after pruning.
    let deleted = sqlx::query("DELETE FROM audit_log").execute(pool).await;
    assert!(deleted.is_err(), "DELETE trigger must be restored");
    close(storage, dir).await;
}

#[tokio::test]
async fn pruning_every_row_keeps_a_checkpoint_head() {
    let (storage, dir) = open().await;
    let pool = storage.pool();
    let old = "2020-01-01T00:00:00Z";
    let h1 = audit::entry_hash(
        audit::GENESIS_HASH,
        SINGLE_USER_ID,
        "id1",
        "run",
        "call1",
        None,
        "terminal",
        "{}",
        "succeeded",
        old,
    );
    sqlx::query(
        "INSERT INTO audit_log \
         (id, owner_id, run_id, tool_call_id, approval_id, tool_name, arguments, decision, prev_hash, entry_hash, created_at) \
         VALUES ('id1', ?1, 'run', 'call1', NULL, 'terminal', '{}', 'succeeded', ?2, ?3, ?4)",
    )
    .bind(SINGLE_USER_ID)
    .bind(audit::GENESIS_HASH)
    .bind(&h1)
    .bind(old)
    .execute(pool)
    .await
    .expect("seed old row");

    assert_eq!(audit::prune_old_entries(pool, 1).await.expect("prune"), 1);
    let live: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_log")
        .fetch_one(pool)
        .await
        .expect("count");
    assert_eq!(live, 1, "checkpoint row preserved");
    let head = audit::entries_for_run(pool, "audit", 1)
        .await
        .expect("checkpoint");
    assert_eq!(head.len(), 1);
    assert_eq!(head[0].decision, audit::PRUNED_CHECKPOINT_DECISION);
    assert_eq!(head[0].prev_hash, h1);
    assert_eq!(audit::verify_chain(pool).await.expect("verify"), 2);

    let appended = audit::append_entry(
        pool,
        SINGLE_USER_ID,
        "run",
        "call2",
        None,
        "terminal",
        &serde_json::json!({}),
        "succeeded",
    )
    .await
    .expect("append");
    assert_eq!(appended.prev_hash, head[0].entry_hash);
    assert_eq!(audit::verify_chain(pool).await.expect("verify"), 3);
    close(storage, dir).await;
}

#[tokio::test]
async fn a_second_prune_archives_only_new_rows_and_keeps_the_chain() {
    let (storage, dir) = open().await;
    let pool = storage.pool();
    let old = "2020-01-01T00:00:00Z";
    // Two old rows, chained from GENESIS.
    let h1 = audit::entry_hash(
        audit::GENESIS_HASH,
        SINGLE_USER_ID,
        "id1",
        "run",
        "call1",
        None,
        "terminal",
        "{}",
        "succeeded",
        old,
    );
    let h2 = audit::entry_hash(
        &h1,
        SINGLE_USER_ID,
        "id2",
        "run",
        "call2",
        None,
        "terminal",
        "{}",
        "succeeded",
        old,
    );
    for (id, call, prev, hash) in [
        ("id1", "call1", audit::GENESIS_HASH, &h1),
        ("id2", "call2", h1.as_str(), &h2),
    ] {
        sqlx::query(
            "INSERT INTO audit_log \
             (id, owner_id, run_id, tool_call_id, approval_id, tool_name, arguments, decision, prev_hash, entry_hash, created_at) \
             VALUES (?1, ?2, 'run', ?3, NULL, 'terminal', '{}', 'succeeded', ?4, ?5, ?6)",
        )
        .bind(id)
        .bind(SINGLE_USER_ID)
        .bind(call)
        .bind(prev)
        .bind(hash)
        .bind(old)
        .execute(pool)
        .await
        .expect("seed old row");
    }

    // First prune: the prefix moves to the archive and a checkpoint is written.
    assert_eq!(
        audit::prune_old_entries(pool, 1)
            .await
            .expect("first prune"),
        2
    );
    assert_eq!(
        audit::verify_chain(pool)
            .await
            .expect("verify after first prune"),
        3
    );

    // Simulate more history ageing out: append an old row chained off the
    // checkpoint head. A second prune previously re-inserted id1/id2 into the
    // archive and failed forever with a UNIQUE(id) violation.
    let checkpoint: (String,) = sqlx::query_as(
        "SELECT entry_hash FROM audit_log WHERE decision = ? ORDER BY seq DESC LIMIT 1",
    )
    .bind(audit::PRUNED_CHECKPOINT_DECISION)
    .fetch_one(pool)
    .await
    .expect("checkpoint head");
    let h3 = audit::entry_hash(
        &checkpoint.0,
        SINGLE_USER_ID,
        "id3",
        "run",
        "call3",
        None,
        "terminal",
        "{}",
        "succeeded",
        old,
    );
    sqlx::query(
        "INSERT INTO audit_log \
         (id, owner_id, run_id, tool_call_id, approval_id, tool_name, arguments, decision, prev_hash, entry_hash, created_at) \
         VALUES ('id3', ?1, 'run', 'call3', NULL, 'terminal', '{}', 'succeeded', ?2, ?3, ?4)",
    )
    .bind(SINGLE_USER_ID)
    .bind(&checkpoint.0)
    .bind(&h3)
    .bind(old)
    .execute(pool)
    .await
    .expect("seed second-era old row");

    assert_eq!(
        audit::prune_old_entries(pool, 1)
            .await
            .expect("second prune"),
        2
    );
    let archived: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_log_archive")
        .fetch_one(pool)
        .await
        .expect("archive count");
    assert_eq!(archived, 4, "each row is archived exactly once");
    assert_eq!(
        audit::verify_chain(pool)
            .await
            .expect("verify after second prune"),
        5
    );
    close(storage, dir).await;
}

#[tokio::test]
async fn truncating_the_chain_tail_is_detected() {
    let (storage, dir) = open().await;
    let pool = storage.pool();
    for i in 0..3 {
        audit::append_entry(
            pool,
            SINGLE_USER_ID,
            "run",
            &format!("call-{i}"),
            None,
            "terminal",
            &serde_json::json!({"i": i}),
            "succeeded",
        )
        .await
        .expect("append");
    }
    assert_eq!(audit::verify_chain(pool).await.expect("verify"), 3);

    // Simulate an attacker who can edit the DB file directly (bypassing the
    // append-only triggers) and cut off the tail. A forward-only hash walk
    // cannot see this; the persistent head checkpoint can.
    sqlx::query("DROP TRIGGER audit_log_no_delete")
        .execute(pool)
        .await
        .expect("drop guard");
    sqlx::query("DELETE FROM audit_log WHERE seq = 3")
        .execute(pool)
        .await
        .expect("truncate tail");
    sqlx::raw_sql(
        "CREATE TRIGGER IF NOT EXISTS audit_log_no_delete \
         BEFORE DELETE ON audit_log BEGIN SELECT RAISE(ABORT, 'audit_log is append-only'); END;",
    )
    .execute(pool)
    .await
    .expect("restore guard");

    let verify = audit::verify_chain(pool).await;
    assert!(verify.is_err(), "a truncated tail must fail verification");
    assert!(
        verify.unwrap_err().to_string().contains("tail truncated"),
        "expected a head mismatch"
    );
    close(storage, dir).await;
}

#[tokio::test]
async fn recomputation_detects_tampering() {
    // The triggers make in-DB tampering impossible, so tamper detection is
    // covered at the hash-function level: any field change alters the digest.
    let base = audit::entry_hash(
        audit::GENESIS_HASH,
        SINGLE_USER_ID,
        "id",
        "run",
        "call",
        None,
        "terminal",
        "{}",
        "approved",
        "t",
    );
    let tampered = audit::entry_hash(
        audit::GENESIS_HASH,
        SINGLE_USER_ID,
        "id",
        "run",
        "call",
        Some("approval"),
        "terminal",
        "{}",
        "denied_by_user",
        "t",
    );
    assert_ne!(base, tampered);
}
