//! B10: Memory共有パック (`mixd-memory-pack/1`) の型と export/import 骨格.
//!
//! `lib.rs` で `pub mod memory_pack;` として配線済み。永続化は
//! `MemoryService::{list/get/create_or_reinforce}` と `memory_links` を通じて
//! 行う呼び出し側の責務。本文は
//! `packs/memory-share-pack/README.md` と `pack.schema.json`。

use serde::{Deserialize, Serialize};

pub const PACK_FORMAT: &str = "mixd-memory-pack/1";
pub const PACK_ITEM_LIMIT: usize = 500;
/// Links grow quadratically with items; cap them explicitly too. A pack with
/// 500 items but 500k links is a denial-of-service on import, not a memory set.
pub const PACK_LINK_LIMIT: usize = PACK_ITEM_LIMIT * 4;
/// A title is a short label, not another content body.
const PACK_TITLE_CHAR_LIMIT: usize = 500;

/// Validate a probability-like field: finite and inside `0.0..=1.0`.
fn valid_unit(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackMeta {
    pub exported_at: chrono::DateTime<chrono::Utc>,
    #[serde(default)]
    pub exporter: String,
    #[serde(default)]
    pub source: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackItem {
    pub id: String,
    #[serde(default)]
    pub title: String,
    pub content: String,
    #[serde(default = "default_role")]
    pub role: String,
    #[serde(default = "default_scope")]
    pub scope: String,
    #[serde(default = "default_source_kind")]
    pub source_kind: String,
    #[serde(default = "default_lifecycle")]
    pub lifecycle_state: String,
    #[serde(default = "default_verification")]
    pub verification: String,
    #[serde(default = "default_confidence")]
    pub confidence: f64,
    #[serde(default = "default_salience")]
    pub salience: f64,
    #[serde(default = "default_strength")]
    pub strength: f64,
    #[serde(default)]
    pub pinned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackLink {
    pub source: String,
    pub target: String,
    pub kind: String,
    #[serde(default = "default_weight")]
    pub weight: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryPack {
    pub format: String,
    pub meta: PackMeta,
    pub items: Vec<PackItem>,
    #[serde(default)]
    pub links: Vec<PackLink>,
}

fn default_role() -> String {
    "fact".to_string()
}
fn default_scope() -> String {
    "user".to_string()
}
fn default_source_kind() -> String {
    "user".to_string()
}
fn default_lifecycle() -> String {
    "established".to_string()
}
fn default_verification() -> String {
    "unverified".to_string()
}
fn default_confidence() -> f64 {
    0.7
}
fn default_salience() -> f64 {
    0.5
}
fn default_strength() -> f64 {
    0.6
}
fn default_weight() -> f64 {
    0.2
}

#[derive(Debug, Clone, Default)]
pub struct ImportOptions {
    /// false のとき lifecycle_state を candidate に倒す。
    pub trust: bool,
    /// true のとき書き込まず件数だけ数える。
    pub dry_run: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ImportReport {
    pub created: usize,
    pub reinforced: usize,
    pub skipped_sensitive: usize,
    pub links_restored: usize,
}

impl MemoryPack {
    /// pack として妥当か検査する。I/O や DB は触らない。
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.format == PACK_FORMAT,
            "unsupported pack format: {}",
            self.format
        );
        anyhow::ensure!(
            self.items.len() <= PACK_ITEM_LIMIT,
            "pack exceeds {} items",
            PACK_ITEM_LIMIT
        );
        anyhow::ensure!(
            self.meta.count == self.items.len(),
            "meta.count ({}) != items.len ({})",
            self.meta.count,
            self.items.len()
        );
        anyhow::ensure!(
            self.links.len() <= PACK_LINK_LIMIT,
            "pack has {} links, over the {PACK_LINK_LIMIT} limit",
            self.links.len()
        );
        // Allow a small clock-skew window; a far-future stamp is a corrupt or
        // forged pack and must not be replayed as a fresh export.
        anyhow::ensure!(
            self.meta.exported_at <= chrono::Utc::now() + chrono::Duration::minutes(5),
            "exported_at is in the future"
        );
        let ids: std::collections::HashSet<&str> =
            self.items.iter().map(|i| i.id.as_str()).collect();
        anyhow::ensure!(ids.len() == self.items.len(), "duplicate item id");
        for item in &self.items {
            anyhow::ensure!(!item.content.trim().is_empty(), "empty content");
            anyhow::ensure!(
                item.title.chars().count() <= PACK_TITLE_CHAR_LIMIT,
                "title too long: {}",
                item.id
            );
            anyhow::ensure!(
                crate::memory::ROLE_NAMES.contains(&item.role.as_str()),
                "unknown role: {} ({})",
                item.id,
                item.role
            );
            anyhow::ensure!(
                crate::memory::SOURCE_KINDS.contains(&item.source_kind.as_str()),
                "unknown source_kind: {} ({})",
                item.id,
                item.source_kind
            );
            anyhow::ensure!(
                crate::memory::LIFECYCLE_STATES.contains(&item.lifecycle_state.as_str()),
                "unknown lifecycle_state: {} ({})",
                item.id,
                item.lifecycle_state
            );
            anyhow::ensure!(
                crate::memory::VERIFICATION_STATES.contains(&item.verification.as_str()),
                "unknown verification: {} ({})",
                item.id,
                item.verification
            );
            for (field, value) in [
                ("confidence", item.confidence),
                ("salience", item.salience),
                ("strength", item.strength),
            ] {
                anyhow::ensure!(
                    valid_unit(value),
                    "{field} must be finite and in 0..=1: {} ({value})",
                    item.id
                );
            }
            // §8: a pack is just another way into the store, so it must refuse
            // the same secrets the normal intake path does.
            anyhow::ensure!(
                !crate::memory::is_sensitive(&item.content)
                    && !crate::memory::is_sensitive(&item.title),
                "sensitive content is not importable (§8): {}",
                item.id
            );
            anyhow::ensure!(
                item.content.chars().count() <= 10_000,
                "content too long: {}",
                item.id
            );
            anyhow::ensure!(
                matches!(item.scope.as_str(), "user" | "project"),
                "scope must be user|project: {}",
                item.id
            );
        }
        for link in &self.links {
            anyhow::ensure!(ids.contains(link.source.as_str()), "unknown link source");
            anyhow::ensure!(ids.contains(link.target.as_str()), "unknown link target");
            anyhow::ensure!(
                crate::memory::RELATION_KINDS.contains(&link.kind.as_str()),
                "unknown link kind: {}",
                link.kind
            );
            anyhow::ensure!(
                valid_unit(link.weight),
                "link weight must be finite and in 0..=1: {}",
                link.weight
            );
        }
        Ok(())
    }

    /// import 時に付与する lifecycle を決める。
    ///
    /// `trust = true` のときは pack が持つ lifecycle を保持する。常に
    /// `established` を返すのは、`candidate` / `superseded` / `archived` の
    /// 項目まで「確立済み」として復活させ、§3 の lifecycle 意味論を壊していた。
    /// 未知の値は信頼できる初期値 (explicit human input) に倒す。
    pub fn import_lifecycle(item: &PackItem, options: &ImportOptions) -> &'static str {
        if options.trust {
            crate::memory::LIFECYCLE_STATES
                .iter()
                .find(|state| **state == item.lifecycle_state)
                .copied()
                .unwrap_or(crate::memory::EXPLICIT_INITIAL_LIFECYCLE)
        } else {
            // 信頼しない pack は自動形成と同じく candidate から始める。
            crate::memory::AUTO_INITIAL_LIFECYCLE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example() -> MemoryPack {
        let raw = include_str!("../../../../packs/memory-share-pack/example.memory-pack.json");
        serde_json::from_str(raw).expect("example pack parses")
    }

    #[test]
    fn example_pack_validates() {
        let pack = example();
        pack.validate().expect("example is valid");
        assert_eq!(pack.items.len(), 2);
    }

    #[test]
    fn a_pack_carrying_a_secret_is_rejected() {
        let mut pack = example();
        pack.items[0].content = "my api_key: sk-live-1234567890".to_string();
        let err = pack
            .validate()
            .expect_err("§8 must reject sensitive content");
        assert!(err.to_string().contains("sensitive"), "{err}");
    }

    #[test]
    fn untrusted_import_demotes_to_candidate() {
        let pack = example();
        let options = ImportOptions::default();
        assert_eq!(
            MemoryPack::import_lifecycle(&pack.items[0], &options),
            "candidate"
        );
    }

    #[test]
    fn a_trusted_import_keeps_the_packs_lifecycle() {
        let mut pack = example();
        let options = ImportOptions {
            trust: true,
            dry_run: false,
        };
        assert_eq!(
            MemoryPack::import_lifecycle(&pack.items[0], &options),
            "established",
            "a valid pack value is preserved"
        );
        // A trusted pack must not revive an archived item as established.
        pack.items[0].lifecycle_state = "archived".to_string();
        assert_eq!(
            MemoryPack::import_lifecycle(&pack.items[0], &options),
            "archived"
        );
        // Unknown values fall back to the explicit-input initial state.
        pack.items[0].lifecycle_state = "nonsense".to_string();
        assert_eq!(
            MemoryPack::import_lifecycle(&pack.items[0], &options),
            crate::memory::EXPLICIT_INITIAL_LIFECYCLE
        );
    }

    #[test]
    fn packs_with_invalid_numbers_or_enums_are_rejected() {
        let mut pack = example();
        pack.items[0].confidence = 1.5;
        assert!(pack.validate().is_err(), "confidence out of range");
        pack.items[0].confidence = f64::NAN;
        assert!(pack.validate().is_err(), "NaN confidence");
        pack.items[0].confidence = 0.8;
        pack.items[0].role = "../../etc/passwd".to_string();
        assert!(pack.validate().is_err(), "unknown role");
        pack.items[0].role = "fact".to_string();
        pack.links[0].weight = -0.1;
        assert!(pack.validate().is_err(), "negative link weight");
        pack.links[0].weight = 0.2;
        pack.links[0].kind = "unknown".to_string();
        assert!(pack.validate().is_err(), "unknown link kind");
    }

    #[test]
    fn packs_with_oversized_titles_or_link_counts_are_rejected() {
        let mut pack = example();
        pack.items[0].title = "x".repeat(PACK_TITLE_CHAR_LIMIT + 1);
        assert!(pack.validate().is_err(), "title too long");
        pack.items[0].title = "short".to_string();
        pack.links = vec![
            PackLink {
                source: "m1".to_string(),
                target: "m2".to_string(),
                kind: "relates_to".to_string(),
                weight: 0.2,
            };
            PACK_LINK_LIMIT + 1
        ];
        assert!(pack.validate().is_err(), "too many links");
    }
}
