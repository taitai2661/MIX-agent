//! Revision history for stored skills.
//!
//! `skill_revisions` existed since the initial schema but was never written, so
//! re-importing a SKILL.md silently replaced the previous version. These tests
//! pin the three paths that now snapshot: overwrite, delete and restore.

use mixd_core::skills::{Skill, SkillService};
use mixd_storage::Storage;

async fn temp_storage() -> Storage {
    let dir = std::env::temp_dir().join(format!("mixd-skills-test-{}", uuid::Uuid::new_v4()));
    Storage::open(&dir).await.expect("open temp storage")
}

fn skill(name: &str, body: &str) -> Skill {
    Skill {
        name: name.to_string(),
        description: "Does a thing.".to_string(),
        body_markdown: body.to_string(),
        ..Skill::default()
    }
}

#[tokio::test]
async fn overwriting_a_skill_keeps_the_previous_body() {
    let storage = temp_storage().await;
    SkillService::save(&storage, skill("demo-skill", "first body"))
        .await
        .expect("save v1");
    SkillService::save(&storage, skill("demo-skill", "second body"))
        .await
        .expect("save v2");

    let revisions = SkillService::list_revisions(&storage, "demo-skill")
        .await
        .expect("list");
    assert_eq!(revisions.len(), 1, "one snapshot per overwrite");
    assert_eq!(revisions[0].skill.body_markdown, "first body");

    let restored = SkillService::restore_revision(&storage, "demo-skill", &revisions[0].id)
        .await
        .expect("restore");
    assert_eq!(restored.body_markdown, "first body");

    let current = SkillService::get(&storage, "demo-skill")
        .await
        .expect("get")
        .expect("present");
    assert_eq!(current.body_markdown, "first body");

    // Restore is reversible too: the second body was snapshotted first.
    let revisions = SkillService::list_revisions(&storage, "demo-skill")
        .await
        .expect("list after restore");
    assert_eq!(revisions.len(), 2);
    assert_eq!(revisions[0].skill.body_markdown, "second body");
}

#[tokio::test]
async fn deleting_a_skill_can_be_undone_from_its_history() {
    let storage = temp_storage().await;
    SkillService::save(&storage, skill("gone-skill", "keep me"))
        .await
        .expect("save");
    SkillService::delete(&storage, "gone-skill")
        .await
        .expect("delete");
    assert!(SkillService::get(&storage, "gone-skill")
        .await
        .expect("get")
        .is_none());

    let revisions = SkillService::list_revisions(&storage, "gone-skill")
        .await
        .expect("list");
    assert_eq!(revisions.len(), 1);
    SkillService::restore_revision(&storage, "gone-skill", &revisions[0].id)
        .await
        .expect("restore after delete");
    let restored = SkillService::get(&storage, "gone-skill")
        .await
        .expect("get")
        .expect("restored");
    assert_eq!(restored.body_markdown, "keep me");
}

#[tokio::test]
async fn reimporting_identical_content_does_not_add_a_revision() {
    let storage = temp_storage().await;
    let first = SkillService::save(&storage, skill("same-skill", "body"))
        .await
        .expect("save v1");
    let second = SkillService::save(&storage, skill("same-skill", "body"))
        .await
        .expect("save v2");
    assert_eq!(
        second.updated_at, first.updated_at,
        "identical save is a no-op"
    );
    let revisions = SkillService::list_revisions(&storage, "same-skill")
        .await
        .expect("list");
    assert!(revisions.is_empty(), "no history for an unchanged file");
}

#[tokio::test]
async fn two_rows_with_the_same_skill_name_are_rejected() {
    let storage = temp_storage().await;
    SkillService::save(&storage, skill("unique-skill", "body"))
        .await
        .expect("save");
    let duplicate = sqlx::query("INSERT INTO skills (id, owner_id, data) VALUES (?1, ?2, ?3)")
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(serde_json::json!({"name": "unique-skill"}).to_string())
        .execute(storage.pool())
        .await;
    assert!(duplicate.is_err(), "the name is unique in SQLite too");
}

#[tokio::test]
async fn a_revision_from_another_skill_cannot_be_restored() {
    let storage = temp_storage().await;
    SkillService::save(&storage, skill("skill-a", "a"))
        .await
        .expect("save a");
    SkillService::save(&storage, skill("skill-b", "b"))
        .await
        .expect("save b");
    SkillService::save(&storage, skill("skill-b", "b2"))
        .await
        .expect("overwrite b");
    let revision = SkillService::list_revisions(&storage, "skill-b")
        .await
        .expect("list")
        .remove(0);

    assert!(
        SkillService::restore_revision(&storage, "skill-a", &revision.id)
            .await
            .is_err(),
        "a revision id must stay scoped to the skill it belongs to"
    );
}
