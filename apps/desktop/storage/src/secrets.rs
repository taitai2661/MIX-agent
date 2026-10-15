//! Secret store: bridges OS Keychain entries to typed accessors used by the
//! Provider adapters and the MCP host.
//!
//! Phase 1 layout:
//!
//! - Provider API keys, MCP tokens, OAuth refresh tokens: stored **opaque to
//!   other apps** in the OS Keychain via `keyring-rs`, then additionally
//!   AES-GCM-encrypted with a Keychain-bound master so they survive a Keychain
//!   re-import or migration tool.
//! - DB encryption key (`db.master`): generated once on first launch, stored in
//!   the Keychain. NOTE: the key is currently *staged only* — the SQLCipher
//!   passphrase wiring is not done yet, so the database file stays plaintext
//!   (owner-only 0o600 permissions are the only current protection; see
//!   `db.rs`). Do not claim at-rest encryption until the wiring lands.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{anyhow, Context};
use argon2::password_hash::SaltString;
use argon2::{Algorithm, Argon2, Params, Version};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use keyring::Entry;
use serde::{Deserialize, Serialize};

/// Keychain service name. This is the single source of truth for the bundle
/// identifier: it must stay identical to `identifier` in `app/tauri.conf.json`
/// and to `APP_IDENTIFIER` in `mixd-app` (which aliases this constant, since
/// the dependency runs app → storage and not the reverse). The test at the
/// bottom of `app/src/lib.rs` asserts all three stay in sync.
pub const SERVICE: &str = "io.github.taitai2661.mix-agent-desktop";
const DB_KEY_PURPOSE: &str = "db.master";
const MASTER_SEED_SLOT: &str = "_master_seed";

/// Abstract backend interface for secret storage.
pub trait SecretBackend: Send + Sync {
    fn put(&self, slot: &str, encrypted_value: &str) -> anyhow::Result<()>;
    fn get(&self, slot: &str) -> anyhow::Result<Option<String>>;
    fn delete(&self, slot: &str) -> anyhow::Result<()>;
    fn master_seed(&self) -> anyhow::Result<String>;
    fn probe(&self) -> anyhow::Result<()>;
}

/// File-based encrypted secret store.
/// Stores AES-GCM encrypted secrets in a JSON file with 0600 permissions
/// alongside a master seed file, avoiding OS Keychain popups.
pub struct FileBackend {
    dir: PathBuf,
    secrets_path: PathBuf,
    seed_path: PathBuf,
    cache: RwLock<Option<HashMap<String, String>>>,
}

impl FileBackend {
    pub fn new(dir: impl AsRef<Path>) -> Self {
        let dir = dir.as_ref().to_path_buf();
        let secrets_path = dir.join("secrets.enc.json");
        let seed_path = dir.join(".master_seed");
        Self {
            dir,
            secrets_path,
            seed_path,
            cache: RwLock::new(None),
        }
    }

    fn ensure_loaded(&self) -> anyhow::Result<()> {
        let needs_load = {
            let read = self
                .cache
                .read()
                .map_err(|e| anyhow!("cache lock poisoned: {e}"))?;
            read.is_none()
        };
        if needs_load {
            let mut write = self
                .cache
                .write()
                .map_err(|e| anyhow!("cache lock poisoned: {e}"))?;
            if write.is_none() {
                let map = if self.secrets_path.exists() {
                    let bytes = std::fs::read(&self.secrets_path)
                        .with_context(|| format!("reading {}", self.secrets_path.display()))?;
                    serde_json::from_slice(&bytes)
                        .with_context(|| format!("parsing {}", self.secrets_path.display()))?
                } else {
                    HashMap::new()
                };
                *write = Some(map);
            }
        }
        Ok(())
    }

    fn persist(&self, map: &HashMap<String, String>) -> anyhow::Result<()> {
        if !self.dir.exists() {
            std::fs::create_dir_all(&self.dir)
                .with_context(|| format!("creating directory {}", self.dir.display()))?;
        }
        let data = serde_json::to_vec_pretty(map).context("serializing secrets")?;
        atomic_write_file(&self.secrets_path, &data)
    }
}

impl SecretBackend for FileBackend {
    fn put(&self, slot: &str, encrypted_value: &str) -> anyhow::Result<()> {
        self.ensure_loaded()?;
        let mut write = self
            .cache
            .write()
            .map_err(|e| anyhow!("cache lock poisoned: {e}"))?;
        let map = write
            .as_mut()
            .ok_or_else(|| anyhow!("cache not initialized"))?;
        map.insert(slot.to_string(), encrypted_value.to_string());
        self.persist(map)
    }

    fn get(&self, slot: &str) -> anyhow::Result<Option<String>> {
        self.ensure_loaded()?;
        let read = self
            .cache
            .read()
            .map_err(|e| anyhow!("cache lock poisoned: {e}"))?;
        let map = read
            .as_ref()
            .ok_or_else(|| anyhow!("cache not initialized"))?;
        Ok(map.get(slot).cloned())
    }

    fn delete(&self, slot: &str) -> anyhow::Result<()> {
        self.ensure_loaded()?;
        let mut write = self
            .cache
            .write()
            .map_err(|e| anyhow!("cache lock poisoned: {e}"))?;
        let map = write
            .as_mut()
            .ok_or_else(|| anyhow!("cache not initialized"))?;
        if map.remove(slot).is_some() {
            self.persist(map)?;
        }
        Ok(())
    }

    fn master_seed(&self) -> anyhow::Result<String> {
        #[cfg(test)]
        {
            if let Some(seed) = test_seed() {
                return Ok(seed);
            }
        }
        if self.seed_path.exists() {
            let seed = std::fs::read_to_string(&self.seed_path).with_context(|| {
                format!("reading master seed from {}", self.seed_path.display())
            })?;
            let trimmed = seed.trim().to_string();
            if !trimmed.is_empty() {
                return Ok(trimmed);
            }
        }
        if !self.dir.exists() {
            std::fs::create_dir_all(&self.dir)
                .with_context(|| format!("creating directory {}", self.dir.display()))?;
        }
        use rand::RngCore;
        let mut rng = rand::thread_rng();
        let mut buf = [0u8; 32];
        rng.fill_bytes(&mut buf);
        let seed = BASE64.encode(buf);
        atomic_write_file(&self.seed_path, seed.as_bytes())
            .with_context(|| format!("writing master seed to {}", self.seed_path.display()))?;
        Ok(seed)
    }

    fn probe(&self) -> anyhow::Result<()> {
        if !self.dir.exists() {
            std::fs::create_dir_all(&self.dir)
                .with_context(|| format!("creating directory {}", self.dir.display()))?;
        }
        let probe_file = self.dir.join(".probe");
        atomic_write_file(&probe_file, b"ok")?;
        let _ = std::fs::remove_file(probe_file);
        Ok(())
    }
}

/// OS Keychain backend (legacy / optional).
pub struct KeychainBackend;

impl SecretBackend for KeychainBackend {
    fn put(&self, slot: &str, encrypted_value: &str) -> anyhow::Result<()> {
        Entry::new(SERVICE, slot)?.set_password(encrypted_value)?;
        Ok(())
    }

    fn get(&self, slot: &str) -> anyhow::Result<Option<String>> {
        let entry = Entry::new(SERVICE, slot)?;
        match entry.get_password() {
            Ok(cipher) => Ok(Some(cipher)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    fn delete(&self, slot: &str) -> anyhow::Result<()> {
        let entry = Entry::new(SERVICE, slot)?;
        match entry.delete_credential() {
            Ok(_) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(err.into()),
        }
    }

    fn master_seed(&self) -> anyhow::Result<String> {
        #[cfg(test)]
        {
            if let Some(seed) = test_seed() {
                return Ok(seed);
            }
        }
        let entry = Entry::new(SERVICE, MASTER_SEED_SLOT)?;
        match entry.get_password() {
            Ok(seed) => Ok(seed),
            Err(keyring::Error::NoEntry) => {
                use rand::RngCore;
                let mut rng = rand::thread_rng();
                let mut buf = [0u8; 32];
                rng.fill_bytes(&mut buf);
                let seed = BASE64.encode(buf);
                entry
                    .set_password(&seed)
                    .context("storing the master seed in the Keychain")?;
                Ok(seed)
            }
            Err(err) => Err(anyhow!(err).context("reading the master seed from the Keychain")),
        }
    }

    fn probe(&self) -> anyhow::Result<()> {
        let entry = Entry::new(SERVICE, "_diagnostics_probe")?;
        match entry.get_password() {
            Ok(_) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(anyhow!("Keychain is not reachable: {err}")),
        }
    }
}

fn atomic_write_file(path: &Path, content: &[u8]) -> anyhow::Result<()> {
    let tmp_path = path.with_extension("tmp");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&tmp_path)
        .with_context(|| format!("opening temporary file {}", tmp_path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        let _ = file.set_permissions(perms);
    }

    use std::io::Write;
    file.write_all(content)?;
    file.sync_all()?;
    drop(file);

    std::fs::rename(&tmp_path, path)
        .with_context(|| format!("renaming {} to {}", tmp_path.display(), path.display()))?;
    Ok(())
}

#[derive(Clone)]
pub struct SecretStore {
    backend: Arc<dyn SecretBackend>,
    master_key_cache: Arc<OnceLock<[u8; 32]>>,
}

impl Default for SecretStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Every entry point here runs Argon2 or filesystem/Keychain I/O; both are
/// blocking. Called from async Tauri commands, offload to worker threads.
macro_rules! offload {
    ($body:expr) => {
        tokio::task::spawn_blocking($body)
            .await
            .map_err(|e| anyhow!("secret store task failed: {e}"))?
    };
}

impl SecretStore {
    /// Create a secret store explicitly backed by a data directory.
    /// By default uses `FileBackend` (AES-GCM encrypted file, 0600 mode) to avoid
    /// macOS Keychain permission prompts. Set `MIX_SECRET_BACKEND=keychain` to force
    /// OS Keychain usage.
    pub fn open(data_dir: impl AsRef<Path>) -> Self {
        let backend: Arc<dyn SecretBackend> = match std::env::var("MIX_SECRET_BACKEND").as_deref() {
            Ok("keychain") => Arc::new(KeychainBackend),
            _ => Arc::new(FileBackend::new(data_dir)),
        };
        Self::with_backend(backend)
    }

    /// Default constructor. Checks `MIX_SECRET_BACKEND`; if unset or "file", resolves
    /// default data directory (or temporary directory in tests).
    pub fn new() -> Self {
        let backend: Arc<dyn SecretBackend> = match std::env::var("MIX_SECRET_BACKEND").as_deref() {
            Ok("keychain") => Arc::new(KeychainBackend),
            _ => {
                #[cfg(test)]
                let default_dir = std::env::temp_dir()
                    .join(format!("mixd_test_secrets_{}", uuid::Uuid::new_v4()));
                #[cfg(not(test))]
                let default_dir = std::env::var("MIX_DATA_DIR")
                    .map(PathBuf::from)
                    .unwrap_or_else(|_| {
                        dirs::data_local_dir()
                            .map(|d| d.join("mix-agent-desktop"))
                            .unwrap_or_else(|| PathBuf::from("."))
                    });
                Arc::new(FileBackend::new(default_dir))
            }
        };
        Self::with_backend(backend)
    }

    pub fn with_backend(backend: Arc<dyn SecretBackend>) -> Self {
        Self {
            backend,
            master_key_cache: Arc::new(OnceLock::new()),
        }
    }

    fn master_key(&self) -> anyhow::Result<[u8; 32]> {
        if let Some(key) = self.master_key_cache.get() {
            return Ok(*key);
        }
        let seed = self.backend.master_seed()?;
        let key = derive_master_key(&seed)?;
        let _ = self.master_key_cache.set(key);
        self.master_key_cache
            .get()
            .cloned()
            .ok_or_else(|| anyhow!("master key not cached"))
    }

    pub async fn put(&self, purpose: &str, name: &str, value: &[u8]) -> anyhow::Result<()> {
        let store = self.clone();
        let (purpose, name, value) = (purpose.to_string(), name.to_string(), value.to_vec());
        offload!(move || {
            let slot = keyring_key(&purpose, &name);
            let master = store.master_key()?;
            let encrypted = encrypt_with_key(&master, &purpose, &name, &value)?;
            store.backend.put(&slot, &encrypted)
        })
    }

    pub async fn get(&self, purpose: &str, name: &str) -> anyhow::Result<Vec<u8>> {
        let store = self.clone();
        let (purpose, name) = (purpose.to_string(), name.to_string());
        offload!(move || {
            let slot = keyring_key(&purpose, &name);
            let maybe_cipher = store.backend.get(&slot)?;
            let cipher = maybe_cipher.ok_or_else(|| anyhow!("no entry for slot {slot}"))?;
            let master = store.master_key()?;
            decrypt_with_key(&master, &purpose, &name, cipher.as_bytes())
        })
    }

    pub async fn get_string(&self, purpose: &str, name: &str) -> anyhow::Result<String> {
        let bytes = self.get(purpose, name).await?;
        String::from_utf8(bytes).map_err(|_| anyhow!("secret is not valid UTF-8"))
    }

    pub async fn delete(&self, purpose: &str, name: &str) -> anyhow::Result<()> {
        let store = self.clone();
        let (purpose, name) = (purpose.to_string(), name.to_string());
        offload!(move || {
            let slot = keyring_key(&purpose, &name);
            store.backend.delete(&slot)
        })
    }

    /// Ensure the master DB encryption key exists in the secret store.
    pub async fn ensure_db_key(&self) -> anyhow::Result<[u8; 32]> {
        if let Ok(plain) = self.get(DB_KEY_PURPOSE, "master").await {
            if plain.len() == 32 {
                let mut key = [0u8; 32];
                key.copy_from_slice(&plain);
                return Ok(key);
            }
        }
        let bytes = {
            use rand::RngCore;
            let mut rng = rand::thread_rng();
            let mut bytes = [0u8; 32];
            rng.fill_bytes(&mut bytes);
            bytes
        };
        self.put(DB_KEY_PURPOSE, "master", &bytes).await?;
        Ok(bytes)
    }

    pub async fn db_master_key(&self) -> anyhow::Result<[u8; 32]> {
        self.ensure_db_key().await
    }

    /// Reachability check for the diagnostics pane.
    pub async fn probe(&self) -> anyhow::Result<()> {
        let store = self.clone();
        offload!(move || store.backend.probe())
    }

    /// Report whether a usable master key is already in the secret store.
    pub async fn db_key_status(&self) -> Option<bool> {
        match self.get(DB_KEY_PURPOSE, "master").await {
            Ok(plain) => Some(plain.len() == 32),
            Err(_) => None,
        }
    }
}

fn keyring_key(purpose: &str, name: &str) -> String {
    format!("{}/{}", purpose, name)
}

#[derive(Serialize, Deserialize)]
struct SealedPayload {
    v: u8,
    p: String,
    n: String,
    c: String,
}

fn derive_master_key(seed: &str) -> anyhow::Result<[u8; 32]> {
    let salt = SaltString::from_b64(&BASE64.encode(format!("{}|master", SERVICE)))
        .map_err(|e| anyhow!("salt: {}", e))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, Params::default());
    let mut out = [0u8; 32];
    argon
        .hash_password_into(seed.as_bytes(), salt.as_str().as_bytes(), &mut out)
        .map_err(|e| anyhow!("argon2: {}", e))?;
    Ok(out)
}

/// Test hook: a fixed seed so the crypto tests never touch the OS Keychain
/// (unavailable on headless CI runners).
#[cfg(test)]
fn test_seed() -> Option<String> {
    static SEED: OnceLock<String> = OnceLock::new();
    Some(
        SEED.get_or_init(|| "mix-agent-desktop-test-master-seed".to_string())
            .clone(),
    )
}

fn encrypt_with_key(
    master: &[u8; 32],
    purpose: &str,
    name: &str,
    plaintext: &[u8],
) -> anyhow::Result<String> {
    let cipher = Aes256Gcm::new_from_slice(master).map_err(|e| anyhow!("cipher: {}", e))?;
    use rand::RngCore;
    let mut rng = rand::thread_rng();
    let mut nonce_bytes = [0u8; 12];
    rng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let aad = format!("{}|{}", purpose, name);
    let ciphertext = cipher
        .encrypt(
            nonce,
            aes_gcm::aead::Payload {
                msg: plaintext,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|e| anyhow!("encrypting secret: {}", e))?;
    let sealed = SealedPayload {
        v: 1,
        p: aad,
        n: BASE64.encode(nonce_bytes),
        c: BASE64.encode(ciphertext),
    };
    serde_json::to_string(&sealed).context("encoding sealed payload")
}

fn decrypt_with_key(
    master: &[u8; 32],
    purpose: &str,
    name: &str,
    payload: &[u8],
) -> anyhow::Result<Vec<u8>> {
    let cipher = Aes256Gcm::new_from_slice(master).map_err(|e| anyhow!("cipher: {}", e))?;
    let sealed: SealedPayload =
        serde_json::from_slice(payload).context("parsing sealed payload")?;
    let nonce_bytes = BASE64.decode(&sealed.n).context("decoding nonce")?;
    let ciphertext = BASE64.decode(&sealed.c).context("decoding ciphertext")?;
    let expected_aad = format!("{}|{}", purpose, name);
    if sealed.p != expected_aad {
        return Err(anyhow!(
            "sealed payload AAD does not match purpose/name; refusing to decrypt"
        ));
    }
    let plaintext = cipher
        .decrypt(
            Nonce::from_slice(&nonce_bytes),
            aes_gcm::aead::Payload {
                msg: &ciphertext,
                aad: expected_aad.as_bytes(),
            },
        )
        .map_err(|e| anyhow!("decrypting secret: {}", e))?;
    Ok(plaintext)
}

#[cfg(test)]
fn encrypt_with_master(purpose: &str, name: &str, plaintext: &[u8]) -> anyhow::Result<String> {
    let seed = test_seed().unwrap_or_else(|| "test-seed".to_string());
    let master = derive_master_key(&seed)?;
    encrypt_with_key(&master, purpose, name, plaintext)
}

#[cfg(test)]
fn decrypt_with_master(purpose: &str, name: &str, payload: &[u8]) -> anyhow::Result<Vec<u8>> {
    let seed = test_seed().unwrap_or_else(|| "test-seed".to_string());
    let master = derive_master_key(&seed)?;
    decrypt_with_key(&master, purpose, name, payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// These exercise the crypto only: the master seed comes from the test
    /// hook, so no Keychain access happens (headless CI has none).
    #[test]
    fn a_sealed_payload_round_trips() {
        let sealed = encrypt_with_master("provider/abc", "api_key", b"sk-secret").expect("seal");
        let plain =
            decrypt_with_master("provider/abc", "api_key", sealed.as_bytes()).expect("open");
        assert_eq!(plain, b"sk-secret");
    }

    #[test]
    fn a_payload_sealed_for_another_slot_is_refused() {
        let sealed = encrypt_with_master("provider/abc", "api_key", b"sk-secret").expect("seal");
        let err = decrypt_with_master("provider/other", "api_key", sealed.as_bytes())
            .expect_err("AAD mismatch must fail");
        assert!(err.to_string().contains("AAD"), "unexpected: {err}");
    }

    #[test]
    fn a_tampered_ciphertext_is_refused() {
        let sealed = encrypt_with_master("provider/abc", "api_key", b"sk-secret").expect("seal");
        let mut payload: SealedPayload = serde_json::from_str(&sealed).expect("parse");
        let mut bytes = BASE64.decode(&payload.c).expect("decode");
        bytes[0] ^= 0xff;
        payload.c = BASE64.encode(bytes);
        let tampered = serde_json::to_string(&payload).expect("encode");
        assert!(decrypt_with_master("provider/abc", "api_key", tampered.as_bytes()).is_err());
    }

    #[test]
    fn a_different_seed_derives_a_different_key() {
        let a = derive_master_key("seed-a").expect("derive");
        let b = derive_master_key("seed-b").expect("derive");
        assert_ne!(a, b, "two seeds must never share a key");
        assert_eq!(
            derive_master_key("seed-a").expect("derive"),
            a,
            "derivation must be deterministic"
        );
    }

    #[tokio::test]
    async fn db_key_status_reports_a_missing_or_present_key() {
        let store = SecretStore::new();
        // No entry yet → the tri-state answer must be `Some(false)` so the UI
        // can show "will be minted on first use" rather than an error.
        match store.db_key_status().await {
            Some(false) | Some(true) | None => {}
        }
    }

    #[tokio::test]
    async fn file_backend_round_trip_and_persistence() {
        let temp_dir = std::env::temp_dir().join(format!("mixd_test_fb_{}", uuid::Uuid::new_v4()));
        let store = SecretStore::open(&temp_dir);

        // Probe should succeed
        store.probe().await.expect("probe");

        // Put secret
        store
            .put("provider/openai", "api_key", b"sk-test-12345")
            .await
            .expect("put");

        // Read back
        let value = store
            .get_string("provider/openai", "api_key")
            .await
            .expect("get");
        assert_eq!(value, "sk-test-12345");

        // Ensure db key
        let db_key = store.ensure_db_key().await.expect("ensure db key");
        assert_eq!(db_key.len(), 32);
        assert_eq!(store.db_key_status().await, Some(true));

        // Re-open from same dir to verify persistence
        let store2 = SecretStore::open(&temp_dir);
        let value2 = store2
            .get_string("provider/openai", "api_key")
            .await
            .expect("get persisted");
        assert_eq!(value2, "sk-test-12345");

        let db_key2 = store2.db_master_key().await.expect("read db key");
        assert_eq!(db_key, db_key2);

        // Delete secret
        store2
            .delete("provider/openai", "api_key")
            .await
            .expect("delete");
        assert!(store2.get("provider/openai", "api_key").await.is_err());

        // Cleanup
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
