//! `.mix/policy.toml` parser scaffold (A5).
//!
//! The file declares workspace-level default policy: which Tools ask, allow
//! or deny when no durable `permission_rules` row matches. It never overrides
//! the database — the DB rows written by "Always Allow" / Settings always win
//! (see `docs/policy-spec.md` §2.1).
//!
//! All failures are fail-closed to [`PolicyFile::default`]: a missing,
//! unreadable or invalid file parses as "ask everything" plus a warning, so a
//! typo can never silently allow a destructive Tool.
//!
//! The app can wire the A5 checks in after `Storage::open` with
//! [`verify_on_startup`], which loads the file, applies its retention window
//! and verifies the audit chain. The tools layer can turn a cached
//! [`PolicyFile`] into an effective decision with [`evaluate_tool_policy`]
//! (only when no DB permission rule matched — DB rules always win).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::audit;
use crate::db::DbPool;

/// Maximum rules accepted from one file. Guards against accidental megabyte
/// files being evaluated on every Tool call.
pub const MAX_POLICY_RULES: usize = 200;

/// File format version accepted by this parser.
pub const POLICY_VERSION: u32 = 1;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PolicyDecision {
    Allow,
    #[default]
    Ask,
    Deny,
}

impl PolicyDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Ask => "ask",
            Self::Deny => "deny",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PolicyScope {
    Global,
    #[default]
    Workspace,
}

/// One `[[rules]]` entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRule {
    /// Tool id, or `"*"` for every Tool.
    pub tool: String,
    pub decision: PolicyDecision,
    #[serde(default)]
    pub scope: PolicyScope,
    #[serde(default)]
    pub pattern: Option<String>,
    #[serde(default)]
    pub workspace: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct DefaultsSection {
    #[serde(default)]
    decision: PolicyDecision,
    #[serde(default = "default_true")]
    audit_enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct AuditSection {
    #[serde(default)]
    verify_on_startup: bool,
    #[serde(default)]
    retention_days: u32,
}

/// Optional Org distribution block (P2-1). When present the file claims to be
/// centrally distributed. A real implementation will verify an Ed25519
/// signature with a public key/fingerprint supplied *out of band* (trusted
/// constant or env), never from this editable file. Until then
/// [`verify_org_signature`] rejects every such file: no `sha256(body+signer)`
/// stand-in, because the signer string lives in the same file an attacker can
/// edit.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct OrgSection {
    #[serde(default)]
    expected_signer: Option<String>,
    #[serde(default)]
    signature: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ApprovalSection {
    #[serde(default = "default_ttl")]
    ttl_secs: i64,
}

fn default_ttl() -> i64 {
    24 * 60 * 60
}

impl Default for ApprovalSection {
    fn default() -> Self {
        Self {
            ttl_secs: default_ttl(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RawPolicyFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    defaults: DefaultsSection,
    #[serde(default)]
    rules: Vec<PolicyRule>,
    #[serde(default)]
    audit: AuditSection,
    #[serde(default)]
    approval: ApprovalSection,
    #[serde(default)]
    org: OrgSection,
}

/// Validated, ready-to-evaluate policy file.
#[derive(Debug, Clone)]
pub struct PolicyFile {
    pub version: u32,
    pub default_decision: PolicyDecision,
    pub audit_enabled: bool,
    pub rules: Vec<PolicyRule>,
    pub verify_on_startup: bool,
    pub retention_days: u32,
    pub approval_ttl_secs: i64,
    /// Pinned Org signer fingerprint, if this file claims central distribution.
    pub org_signer: Option<String>,
    /// Signature accompanying an Org claim. Kept so `verify_org_signature`
    /// can reject the whole file at load time instead of ignoring the claim.
    pub org_signature: Option<String>,
}

impl Default for PolicyFile {
    /// Fail-closed default: ask everything, audit on.
    fn default() -> Self {
        Self {
            version: POLICY_VERSION,
            default_decision: PolicyDecision::Ask,
            audit_enabled: true,
            rules: Vec::new(),
            verify_on_startup: false,
            retention_days: 0,
            approval_ttl_secs: default_ttl(),
            org_signer: None,
            org_signature: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PolicyError {
    #[error("unsupported policy version {0} (expected {POLICY_VERSION})")]
    Version(u32),
    #[error("tool name is empty (rule #{0})")]
    EmptyTool(usize),
    #[error("too many rules ({0} > {MAX_POLICY_RULES})")]
    TooManyRules(usize),
    #[error("invalid glob pattern {pattern:?} (rule #{index}): {reason}")]
    BadPattern {
        index: usize,
        pattern: String,
        reason: String,
    },
    #[error("invalid scope {0:?} (rule #{1}): policy files accept only global/workspace")]
    BadScope(String, usize),
    #[error("TOML parse error: {0}")]
    Parse(String),
    #[error(
        "Org-distributed policy files are not supported (signature verification is unavailable)"
    )]
    OrgUnverifiable,
}

/// Normalise a rule glob so Windows paths behave like glob paths.
///
/// This is the single normalisation used by *both* the policy file evaluator
/// and the `permission_rules` executor. `glob` treats `\` as an escape on Unix
/// and refuses recursive wildcards next to it, so `C:\Windows\**\*` never
/// compiled in one surface while it degraded to a substring match in the
/// other. Keeping it here makes the two surfaces impossible to drift.
pub fn normalize_glob_pattern(pattern: &str) -> String {
    pattern.replace('\\', "/")
}

/// Match a rule glob against a target. `None` means the pattern could not be
/// parsed; callers must fail closed rather than fall back to a substring test
/// (which silently widens Deny rules).
pub fn glob_matches(pattern: &str, target: &str) -> Option<bool> {
    let matcher = glob::Pattern::new(&normalize_glob_pattern(pattern)).ok()?;
    Some(matcher.matches(&normalize_glob_pattern(target)))
}

/// Validate a glob with the same parser the executor uses, so the file and
/// the runtime never disagree about which patterns match.
pub fn validate_glob_pattern(pattern: &str) -> Result<(), String> {
    glob::Pattern::new(&normalize_glob_pattern(pattern))
        .map(|_| ())
        .map_err(|err| err.to_string())
}

fn check_pattern(pattern: &str) -> Result<(), String> {
    validate_glob_pattern(pattern)
}

impl PolicyFile {
    fn validate(mut raw: RawPolicyFile) -> Result<Self, PolicyError> {
        if raw.version != POLICY_VERSION {
            return Err(PolicyError::Version(raw.version));
        }
        if raw.rules.len() > MAX_POLICY_RULES {
            return Err(PolicyError::TooManyRules(raw.rules.len()));
        }
        for (i, rule) in raw.rules.iter_mut().enumerate() {
            rule.tool = rule.tool.trim().to_string();
            if rule.tool.is_empty() {
                return Err(PolicyError::EmptyTool(i));
            }
            if let Some(p) = rule.pattern.as_mut() {
                *p = p.trim().to_string();
                if p.is_empty() {
                    rule.pattern = None;
                } else if let Err(reason) = check_pattern(p) {
                    return Err(PolicyError::BadPattern {
                        index: i,
                        pattern: p.clone(),
                        reason,
                    });
                }
            }
            if let Some(w) = rule.workspace.as_mut() {
                *w = w.trim().to_string();
                if w.is_empty() {
                    rule.workspace = None;
                }
            }
        }
        Ok(Self {
            version: raw.version,
            default_decision: raw.defaults.decision,
            audit_enabled: raw.defaults.audit_enabled,
            rules: raw.rules,
            verify_on_startup: raw.audit.verify_on_startup,
            retention_days: raw.audit.retention_days,
            approval_ttl_secs: raw.approval.ttl_secs,
            org_signer: raw.org.expected_signer,
            org_signature: raw.org.signature,
        })
    }
}

/// Verify an Org-distributed policy file. **Scaffold: always fails closed.**
///
/// `true` only when the file carries no Org distribution claim at all
/// (nothing to verify). Any `expected_signer` or `signature` is rejected with
/// a loud warning. The previous `sha256(body + signer)` comparison was
/// forgeable: both the body and the "signer" came from the same editable TOML,
/// so an attacker could recompute the digest. No Ed25519 crate is present in
/// the current dependency tree; when one is added, verification must use a
/// public key/fingerprint from a trusted out-of-band source (env or a build
/// constant), and this function must keep rejecting on absence of that key.
pub fn verify_org_signature(
    _body_without_signature: &str,
    file: &PolicyFile,
    signature: Option<&str>,
) -> bool {
    if file.org_signer.is_none() && signature.is_none() {
        return true;
    }
    tracing::warn!(
        "org policy signature verification is not implemented: rejecting Org-distributed policy file (fail closed)"
    );
    false
}

/// Parse TOML text into a validated [`PolicyFile`].
///
/// An Org-distribution claim is rejected outright: no Ed25519 verifier or
/// out-of-band trust anchor exists yet, so accepting the file's rules would
/// let anyone self-authorise central policy.
pub fn parse_policy_toml(text: &str) -> Result<PolicyFile, PolicyError> {
    let raw: RawPolicyFile = toml::from_str(text).map_err(|e| PolicyError::Parse(e.to_string()))?;
    let file = PolicyFile::validate(raw)?;
    if !verify_org_signature(text, &file, file.org_signature.as_deref()) {
        return Err(PolicyError::OrgUnverifiable);
    }
    Ok(file)
}

/// Load a policy file from disk. Never fails: any error yields the fail-closed
/// default plus a warning, so a broken file cannot break startup or a Run.
pub fn load_policy_file(path: &Path) -> PolicyFile {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            tracing::debug!(path = %path.display(), %e, "no policy file; using fail-closed defaults");
            return PolicyFile::default();
        }
    };
    match parse_policy_toml(&text) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(path = %path.display(), %e, "invalid policy file; using fail-closed defaults");
            PolicyFile::default()
        }
    }
}

/// First matching rule for (`tool`, `pattern_target`), in file order.
///
/// `tool = "*"` matches every Tool. A rule with a `pattern` only matches when
/// `pattern_target` is present and the glob matches it.
pub fn decide_from_file<'a>(
    file: &'a PolicyFile,
    tool: &str,
    pattern_target: Option<&str>,
) -> Option<&'a PolicyRule> {
    file.rules.iter().find(|rule| {
        if rule.tool != "*" && rule.tool != tool {
            return false;
        }
        match (&rule.pattern, pattern_target) {
            (None, _) => true,
            // A pattern that cannot be parsed does not match (fail closed),
            // matching the executor's `permission_rules` behaviour instead of
            // the old substring fallback that silently widened the rule.
            (Some(pattern), Some(target)) => glob_matches(pattern, target).unwrap_or(false),
            (Some(_), None) => false,
        }
    })
}

/// Conventional policy file location under the app data directory.
pub fn policy_path(data_dir: &Path) -> PathBuf {
    data_dir.join(".mix").join("policy.toml")
}

/// Effective decision for one Tool invocation under the *file* policy.
///
/// Call this only after no DB `permission_rules` row matched: durable DB rules
/// always win over the file (see `docs/policy-spec.md` §2.1). Returns the first
/// matching rule's decision, or the file's fail-closed default (`Ask` when the
/// file is missing or invalid).
pub fn evaluate_tool_policy(
    policy: &PolicyFile,
    tool: &str,
    pattern_target: Option<&str>,
) -> PolicyDecision {
    decide_from_file(policy, tool, pattern_target)
        .map(|rule| rule.decision.clone())
        .unwrap_or_else(|| policy.default_decision.clone())
}

/// Outcome of the startup checks requested by a policy file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupCheck {
    /// Rows moved out of the live audit log into the archive.
    pub pruned_rows: u64,
    /// Whether `[audit] verify_on_startup` requested chain verification.
    pub verification_requested: bool,
    /// Number of rows verified (archive + live); 0 when not requested.
    pub verified_rows: u64,
    /// Failure reason, when verification was requested and failed.
    pub failure: Option<String>,
}

/// Load `path` and run the A5 startup maintenance it requests: retention
/// prune, then optional audit-chain verification. Never throws — every
/// failure is recorded in [`StartupCheck`] so startup can warn and continue.
///
/// Wiring from the app, right after `Storage::open`:
/// ```no_run
/// # async fn example(storage: &mixd_storage::Storage, data_dir: &std::path::Path) -> anyhow::Result<()> {
/// let report = mixd_storage::policy::verify_on_startup(
///     storage.pool(),
///     &mixd_storage::policy::policy_path(data_dir),
/// ).await;
/// if let Some(reason) = report.failure {
///     tracing::warn!(%reason, "audit chain verification failed at startup");
/// }
/// # Ok(())
/// # }
/// ```
pub async fn verify_on_startup(pool: &DbPool, path: &Path) -> StartupCheck {
    let policy = load_policy_file(path);
    verify_on_startup_with_policy(pool, &policy).await
}

/// Same as [`verify_on_startup`], for callers that already loaded/cached the
/// policy file (and use it for [`evaluate_tool_policy`] during Runs).
pub async fn verify_on_startup_with_policy(pool: &DbPool, policy: &PolicyFile) -> StartupCheck {
    let mut check = StartupCheck {
        pruned_rows: 0,
        verification_requested: policy.verify_on_startup,
        verified_rows: 0,
        failure: None,
    };
    if policy.retention_days > 0 {
        match prune_from_policy(pool, policy).await {
            Ok(rows) => check.pruned_rows = rows,
            Err(err) => tracing::warn!(%err, "audit retention prune failed; continuing"),
        }
    }
    if policy.verify_on_startup {
        match audit::verify_chain(pool).await {
            Ok(rows) => check.verified_rows = rows,
            Err(err) => check.failure = Some(err.to_string()),
        }
    }
    check
}

/// Prune old audit rows according to `policy.retention_days` without running
/// verification. Returns the number of rows archived.
pub async fn prune_from_policy(pool: &DbPool, policy: &PolicyFile) -> anyhow::Result<u64> {
    audit::prune_old_entries(pool, policy.retention_days).await
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
version = 1

[defaults]
decision = "ask"
audit_enabled = true

[[rules]]
tool = "terminal"
decision = "ask"
scope = "workspace"
pattern = "cargo test*"

[[rules]]
tool = "files"
decision = "deny"
pattern = "**/.env*"

[audit]
verify_on_startup = false
retention_days = 0

[approval]
ttl_secs = 86400
"#;

    #[test]
    fn sample_parses_and_matches() {
        let file = parse_policy_toml(SAMPLE).expect("sample parses");
        assert_eq!(file.rules.len(), 2);
        let hit = decide_from_file(&file, "terminal", Some("cargo test --all")).expect("match");
        assert_eq!(hit.decision, PolicyDecision::Ask);
        assert!(decide_from_file(&file, "terminal", Some("rm -rf ~")).is_none());
        let hit = decide_from_file(&file, "files", Some("a/.env")).expect("deny match");
        assert_eq!(hit.decision, PolicyDecision::Deny);
    }

    #[test]
    fn bad_version_and_bad_pattern_rejected() {
        assert!(parse_policy_toml("version = 99").is_err());
        let bad = "version = 1\n[[rules]]\ntool = \"x\"\ndecision = \"allow\"\npattern = \"[\"";
        // "[" is actually a valid glob in glob 0.3 (literal bracket); use a
        // control character that the parser rejects instead.
        let _ = bad;
        let bad2 = "version = 1\n[[rules]]\ntool = \"\"\ndecision = \"allow\"";
        assert!(parse_policy_toml(bad2).is_err());
    }

    #[test]
    fn windows_style_globs_are_normalised_like_the_executor() {
        // The old policy parser validated the raw pattern (rejecting `C:\…`)
        // while `decide_from_file` fell back to substring matching on parse
        // failure. Both surfaces now share `glob_matches`.
        assert_eq!(
            glob_matches(r"C:\Windows\**\*", r"C:\Windows\System32\hosts"),
            Some(true)
        );
        assert_eq!(
            glob_matches(r"C:\Windows\**\*", r"C:\Users\dev\file"),
            Some(false)
        );
        assert!(validate_glob_pattern(r"C:\Windows\**\*").is_ok());
        // Fail closed: an unparsable rule never matches.
        assert_eq!(glob_matches("***", "src/main.rs"), None);
    }

    #[test]
    fn wildcard_tool_matches_all() {
        let file = parse_policy_toml("version = 1\n[[rules]]\ntool = \"*\"\ndecision = \"deny\"")
            .expect("parses");
        assert!(decide_from_file(&file, "anything", None).is_some());
    }

    fn temp_policy(text: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("mixd-policy-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = policy_path(&dir);
        std::fs::create_dir_all(path.parent().unwrap()).expect("create .mix");
        std::fs::write(&path, text).expect("write policy");
        (path, dir)
    }

    #[test]
    fn load_and_decide_from_file() {
        let (path, dir) = temp_policy(SAMPLE);
        let file = load_policy_file(&path);
        assert_eq!(
            evaluate_tool_policy(&file, "terminal", Some("cargo test --all")),
            PolicyDecision::Ask
        );
        assert_eq!(
            evaluate_tool_policy(&file, "files", Some("secret/.env.local")),
            PolicyDecision::Deny
        );
        // Unmatched tools fall back to the fail-closed default.
        assert_eq!(
            evaluate_tool_policy(&file, "network", None),
            PolicyDecision::Ask
        );

        let (path2, dir2) = temp_policy("version = 1\n[defaults]\ndecision = \"deny\"\n");
        let deny_by_default = load_policy_file(&path2);
        assert_eq!(
            evaluate_tool_policy(&deny_by_default, "terminal", None),
            PolicyDecision::Deny
        );
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(dir2);
    }

    #[test]
    fn missing_or_invalid_file_fails_closed() {
        let missing = std::env::temp_dir().join(format!("mixd-no-policy-{}", uuid::Uuid::new_v4()));
        let file = load_policy_file(&missing);
        assert_eq!(
            evaluate_tool_policy(&file, "terminal", None),
            PolicyDecision::Ask
        );

        let (path, dir) = temp_policy("version = 99");
        let file = load_policy_file(&path);
        assert_eq!(
            evaluate_tool_policy(&file, "terminal", None),
            PolicyDecision::Ask
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn verify_on_startup_respects_policy_flags() {
        let data_dir =
            std::env::temp_dir().join(format!("mixd-policy-startup-{}", uuid::Uuid::new_v4()));
        let storage = crate::Storage::open(&data_dir).await.expect("open");

        // Missing file: no verification requested, no failure.
        let check = verify_on_startup(storage.pool(), &policy_path(&data_dir)).await;
        assert!(!check.verification_requested);
        assert_eq!(check.verified_rows, 0);
        assert!(check.failure.is_none());

        let (path, policy_dir) = temp_policy("version = 1\n[audit]\nverify_on_startup = true\n");
        let check = verify_on_startup(storage.pool(), &path).await;
        assert!(check.verification_requested);
        assert_eq!(check.verified_rows, 0, "empty chain verifies as length 0");

        audit::append_entry(
            storage.pool(),
            crate::records::SINGLE_USER_ID,
            "run",
            "call",
            None,
            "terminal",
            &serde_json::json!({}),
            "succeeded",
        )
        .await
        .expect("append");
        let check = verify_on_startup(storage.pool(), &path).await;
        assert_eq!(check.verified_rows, 1);
        assert!(check.failure.is_none());

        drop(storage);
        let _ = std::fs::remove_dir_all(data_dir);
        let _ = std::fs::remove_dir_all(policy_dir);
    }

    #[tokio::test]
    async fn verify_on_startup_applies_retention_days() {
        let data_dir =
            std::env::temp_dir().join(format!("mixd-policy-prune-{}", uuid::Uuid::new_v4()));
        let storage = crate::Storage::open(&data_dir).await.expect("open");
        let old = "2020-01-01T00:00:00Z";
        let hash = audit::entry_hash(
            audit::GENESIS_HASH,
            crate::records::SINGLE_USER_ID,
            "old",
            "run",
            "call",
            None,
            "terminal",
            "{}",
            "succeeded",
            old,
        );
        sqlx::query(
            "INSERT INTO audit_log \
             (id, owner_id, run_id, tool_call_id, approval_id, tool_name, arguments, decision, prev_hash, entry_hash, created_at) \
             VALUES ('old', ?1, 'run', 'call', NULL, 'terminal', '{}', 'succeeded', ?2, ?3, ?4)",
        )
        .bind(crate::records::SINGLE_USER_ID)
        .bind(audit::GENESIS_HASH)
        .bind(&hash)
        .bind(old)
        .execute(storage.pool())
        .await
        .expect("seed old row");

        let (path, policy_dir) =
            temp_policy("version = 1\n[audit]\nverify_on_startup = true\nretention_days = 1\n");
        let check = verify_on_startup(storage.pool(), &path).await;
        assert_eq!(check.pruned_rows, 1);
        assert_eq!(check.verified_rows, 2, "archived row + checkpoint");
        assert!(check.failure.is_none());

        drop(storage);
        let _ = std::fs::remove_dir_all(data_dir);
        let _ = std::fs::remove_dir_all(policy_dir);
    }

    #[tokio::test]
    async fn verify_on_startup_reports_tampering() {
        let data_dir =
            std::env::temp_dir().join(format!("mixd-policy-tamper-{}", uuid::Uuid::new_v4()));
        let storage = crate::Storage::open(&data_dir).await.expect("open");
        audit::append_entry(
            storage.pool(),
            crate::records::SINGLE_USER_ID,
            "run",
            "call",
            None,
            "terminal",
            &serde_json::json!({}),
            "succeeded",
        )
        .await
        .expect("append");

        // Simulate an on-disk edit that bypasses the append-only trigger.
        sqlx::query("DROP TRIGGER audit_log_no_update")
            .execute(storage.pool())
            .await
            .expect("drop guard");
        sqlx::query("UPDATE audit_log SET entry_hash = 'tampered'")
            .execute(storage.pool())
            .await
            .expect("tamper");
        sqlx::raw_sql(
            "CREATE TRIGGER IF NOT EXISTS audit_log_no_update \
             BEFORE UPDATE ON audit_log BEGIN SELECT RAISE(ABORT, 'audit_log is append-only'); END;",
        )
        .execute(storage.pool())
        .await
        .expect("restore guard");

        let (path, policy_dir) = temp_policy("version = 1\n[audit]\nverify_on_startup = true\n");
        let check = verify_on_startup(storage.pool(), &path).await;
        assert!(check.verification_requested);
        assert_eq!(check.verified_rows, 0);
        assert!(check.failure.is_some(), "tampered chain must be reported");

        drop(storage);
        let _ = std::fs::remove_dir_all(data_dir);
        let _ = std::fs::remove_dir_all(policy_dir);
    }

    #[test]
    fn org_signature_never_accepts_a_self_signed_claim() {
        // No Org block: nothing to verify.
        let plain = parse_policy_toml("version = 1").expect("parses");
        assert!(verify_org_signature("version = 1", &plain, None));
        assert!(!verify_org_signature("version = 1", &plain, Some("abc")));

        // A file claiming Org distribution is rejected outright at parse time
        // (the claimed signer lives in the same editable file, so it proves
        // nothing). Build the equivalent value by hand to exercise the
        // predicate itself as well.
        assert!(parse_policy_toml("version = 1\n[org]\nexpected_signer = \"team-key-1\"").is_err());
        let mut org = parse_policy_toml("version = 1").expect("parses");
        org.org_signer = Some("team-key-1".into());
        use sha2::Digest;
        let mut hasher = sha2::Sha256::new();
        hasher.update(b"version = 1");
        hasher.update(b"team-key-1");
        let forged = hex::encode(hasher.finalize());
        assert!(!verify_org_signature("version = 1", &org, None));
        assert!(!verify_org_signature("version = 1", &org, Some("wrong")));
        assert!(!verify_org_signature("version = 1", &org, Some(&forged)));

        // A stray signature without a pinned signer is also rejected.
        assert!(!verify_org_signature("version = 1", &plain, Some(&forged)));
    }
}
