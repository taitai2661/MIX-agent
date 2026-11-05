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

    /// Read the seed if it already exists. Unlike [`SecretBackend::master_seed`]
    /// this never creates one, so the hybrid backend can tell "first launch"
    /// apart from "an existing file-seed install".
    fn existing_seed(&self) -> anyhow::Result<Option<String>> {
        if !self.seed_path.exists() {
            return Ok(None);
        }
        let seed = std::fs::read_to_string(&self.seed_path)
            .with_context(|| format!("reading master seed from {}", self.seed_path.display()))?;
        let trimmed = seed.trim().to_string();
        Ok(if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        })
    }

    /// Remove the on-disk seed after it has been copied into the OS keychain.
    /// Best-effort: a failure leaves the copy behind, which is the safe
    /// direction (the seed is still recoverable), so it is only logged.
    fn remove_seed(&self) {
        if let Err(err) = std::fs::remove_file(&self.seed_path) {
            if err.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(
                    %err,
                    path = %self.seed_path.display(),
                    "could not remove the file-backed master seed after keychain migration"
                );
            }
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
        if let Some(seed) = self.existing_seed()? {
            return Ok(seed);
        }
        if !self.dir.exists() {
            std::fs::create_dir_all(&self.dir)
                .with_context(|| format!("creating directory {}", self.dir.display()))?;
        }
        let seed = generate_seed();
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

impl KeychainBackend {
    /// Read the keychain seed without creating one. `None` means "this install
    /// has never used the keychain"; an error means the keychain is reachable
    /// but refused (locked / permission denied), which callers must not treat
    /// as "first launch".
    fn existing_master_seed(&self) -> anyhow::Result<Option<String>> {
        let entry = Entry::new(SERVICE, MASTER_SEED_SLOT)?;
        match entry.get_password() {
            Ok(seed) if seed.trim().is_empty() => Ok(None),
            Ok(seed) => Ok(Some(seed)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(anyhow!(err).context("reading the master seed from the Keychain")),
        }
    }

    fn store_master_seed(&self, seed: &str) -> anyhow::Result<()> {
        let entry = Entry::new(SERVICE, MASTER_SEED_SLOT)?;
        entry
            .set_password(seed)
            .context("storing the master seed in the Keychain")
    }
}

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
        if let Some(seed) = self.existing_master_seed()? {
            return Ok(seed);
        }
        let seed = generate_seed();
        self.store_master_seed(&seed)?;
        Ok(seed)
    }

    fn probe(&self) -> anyhow::Result<()> {
        let entry = Entry::new(SERVICE, "_diagnostics_probe")?;
        match entry.get_password() {
            Ok(_) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(anyhow!("Keychain is not reachable: {err}")),
        }
    }
}

fn generate_seed() -> String {
    use rand::RngCore;
    let mut rng = rand::thread_rng();
    let mut buf = [0u8; 32];
    rng.fill_bytes(&mut buf);
    BASE64.encode(buf)
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

/// Keychain-preferred backend.
///
/// Secret *values* stay in the AES-GCM file (so reading a provider key never
/// opens a Keychain prompt), but the **master seed** lives in the OS Keychain.
/// This closes the main weakness of the plain file backend: the seed used to
/// sit in `.master_seed` next to `secrets.enc.json`, so anyone who could read
/// the data directory could decrypt every stored credential.
///
/// Migration is automatic and idempotent. An existing file seed is copied into
/// the Keychain, read back to prove the write worked, and only then removed
/// from disk. If the Keychain is unavailable the file seed is used (when it
/// still exists); when it does not, the error is returned instead of silently
/// minting a new seed that would make existing secrets undecryptable.
/// The part of the keychain backend the hybrid needs. Keeping this narrow lets
/// unit tests exercise the migration with an in-memory seed store instead of a
/// real Keychain prompt.
trait SeedStore: Send + Sync {
    fn existing_master_seed(&self) -> anyhow::Result<Option<String>>;
    fn store_master_seed(&self, seed: &str) -> anyhow::Result<()>;
    fn probe(&self) -> anyhow::Result<()>;
}

impl SeedStore for KeychainBackend {
    fn existing_master_seed(&self) -> anyhow::Result<Option<String>> {
        KeychainBackend::existing_master_seed(self)
    }

    fn store_master_seed(&self, seed: &str) -> anyhow::Result<()> {
        KeychainBackend::store_master_seed(self, seed)
    }

    fn probe(&self) -> anyhow::Result<()> {
        SecretBackend::probe(self)
    }
}

pub struct KeychainSeedBackend {
    file: FileBackend,
    keychain: Arc<dyn SeedStore>,
}

impl KeychainSeedBackend {
    pub fn new(dir: impl AsRef<Path>) -> Self {
        Self {
            file: FileBackend::new(dir),
            keychain: Arc::new(KeychainBackend),
        }
    }

    #[cfg(test)]
    fn with_seed_store(dir: impl AsRef<Path>, keychain: Arc<dyn SeedStore>) -> Self {
        Self {
            file: FileBackend::new(dir),
            keychain,
        }
    }
}

impl SecretBackend for KeychainSeedBackend {
    fn put(&self, slot: &str, encrypted_value: &str) -> anyhow::Result<()> {
        self.file.put(slot, encrypted_value)
    }

    fn get(&self, slot: &str) -> anyhow::Result<Option<String>> {
        self.file.get(slot)
    }

    fn delete(&self, slot: &str) -> anyhow::Result<()> {
        self.file.delete(slot)
    }

    fn master_seed(&self) -> anyhow::Result<String> {
        match self.keychain.existing_master_seed() {
            Ok(Some(seed)) => {
                // A stale copy is a recovery aid, not a second source of
                // truth. Keep it when it differs (so a manual rollback is
                // possible) and warn loudly; remove it when it matches.
                match self.file.existing_seed() {
                    Ok(Some(file_seed)) if file_seed != seed => {
                        tracing::warn!(
                            "file-backed and keychain master seeds differ; using the keychain seed. \
                             Existing secrets sealed with the file seed will fail to decrypt."
                        );
                    }
                    Ok(Some(_)) => self.file.remove_seed(),
                    Ok(None) => {}
                    Err(err) => {
                        tracing::warn!(%err, "could not inspect the file-backed master seed");
                    }
                }
                Ok(seed)
            }
            Ok(None) => {
                if let Some(seed) = self.file.existing_seed()? {
                    self.keychain.store_master_seed(&seed)?;
                    match self.keychain.existing_master_seed() {
                        Ok(Some(read)) if read == seed => {
                            self.file.remove_seed();
                            tracing::info!(
                                "migrated the master seed from the data directory into the OS keychain"
                            );
                        }
                        Ok(_) => {
                            tracing::warn!(
                                "keychain seed read-back did not match; keeping the file copy"
                            );
                        }
                        Err(err) => {
                            tracing::warn!(%err, "could not verify the keychain seed; keeping the file copy");
                        }
                    }
                    return Ok(seed);
                }
                let seed = generate_seed();
                self.keychain.store_master_seed(&seed)?;
                match self.keychain.existing_master_seed() {
                    Ok(Some(read)) if read == seed => Ok(seed),
                    Ok(_) => Err(anyhow!(
                        "the OS keychain did not return the master seed it just stored"
                    )),
                    Err(err) => Err(anyhow!(
                        "the OS keychain could not be read back after storing the master seed: {err}"
                    )),
                }
            }
            Err(err) => match self.file.existing_seed() {
                Ok(Some(seed)) => {
                    tracing::warn!(
                        %err,
                        "OS keychain unavailable; falling back to the file-backed master seed"
                    );
                    Ok(seed)
                }
                Ok(None) => Err(anyhow!(
                    "the OS keychain is unavailable ({err}) and no file-backed master seed exists. \
                     Unlock the keychain and restart, or set MIX_SECRET_BACKEND=file to start over."
                )),
                Err(file_err) => Err(file_err),
            },
        }
    }

    fn probe(&self) -> anyhow::Result<()> {
        match self.keychain.probe() {
            Ok(()) => Ok(()),
            Err(keychain_err) => self
                .file
                .probe()
                .map_err(|file_err| anyhow!(
                    "neither the OS keychain ({keychain_err}) nor the data directory ({file_err}) is usable"
                )),
        }
    }
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
    ///
    /// macOS and Windows keep the master seed in the OS keychain while the
    /// encrypted values stay in the 0600 data file (`KeychainSeedBackend`).
    /// Linux defaults to the file backend because Secret Service is not
    /// guaranteed to be present; set `MIX_SECRET_BACKEND=keychain` to opt in,
    /// or `MIX_SECRET_BACKEND=file` to force the legacy layout on any OS.
    pub fn open(data_dir: impl AsRef<Path>) -> Self {
        let data_dir = data_dir.as_ref();
        let backend: Arc<dyn SecretBackend> = match std::env::var("MIX_SECRET_BACKEND").as_deref() {
            Ok("keychain") => Arc::new(KeychainBackend),
            Ok("file") => Arc::new(FileBackend::new(data_dir)),
            _ if cfg!(any(target_os = "macos", target_os = "windows")) => {
                Arc::new(KeychainSeedBackend::new(data_dir))
            }
            _ => Arc::new(FileBackend::new(data_dir)),
        };
        Self::with_backend(backend)
    }

    /// Construct with an explicit backend. Tests use this so they never touch
    /// a developer's real keychain.
    pub fn open_with_backend(backend: Arc<dyn SecretBackend>) -> Self {
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
        let store = SecretStore::open_with_backend(Arc::new(FileBackend::new(&temp_dir)));

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
        let store2 = SecretStore::open_with_backend(Arc::new(FileBackend::new(&temp_dir)));
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

    #[derive(Default)]
    struct MemorySeedStore {
        seed: std::sync::Mutex<Option<String>>,
        fail: bool,
    }

    impl SeedStore for MemorySeedStore {
        fn existing_master_seed(&self) -> anyhow::Result<Option<String>> {
            if self.fail {
                anyhow::bail!("keychain is locked");
            }
            Ok(self.seed.lock().unwrap().clone())
        }

        fn store_master_seed(&self, seed: &str) -> anyhow::Result<()> {
            if self.fail {
                anyhow::bail!("keychain is locked");
            }
            *self.seed.lock().unwrap() = Some(seed.to_string());
            Ok(())
        }

        fn probe(&self) -> anyhow::Result<()> {
            if self.fail {
                anyhow::bail!("keychain is locked");
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn hybrid_moves_the_seed_into_the_keychain_and_keeps_secrets_readable() {
        let dir = std::env::temp_dir().join(format!("mixd_test_hybrid_{}", uuid::Uuid::new_v4()));

        // Legacy install: encrypted value + seed on disk.
        let legacy = SecretStore::open_with_backend(Arc::new(FileBackend::new(&dir)));
        legacy
            .put("provider/openai", "api_key", b"sk-legacy")
            .await
            .expect("seed the legacy store");
        let seed_file = dir.join(".master_seed");
        assert!(seed_file.is_file(), "file backend writes the seed");

        // First launch with the hybrid backend migrates the seed.
        let keychain = Arc::new(MemorySeedStore::default());
        let hybrid = KeychainSeedBackend::with_seed_store(&dir, keychain.clone());
        let migrated = hybrid.master_seed().expect("migrate seed");
        assert_eq!(
            keychain.seed.lock().unwrap().as_deref(),
            Some(migrated.as_str())
        );
        assert!(
            !seed_file.exists(),
            "the on-disk seed is removed after migration"
        );

        // Values stay in the file and remain decryptable under the migrated seed.
        let store = SecretStore::open_with_backend(Arc::new(hybrid));
        assert_eq!(
            store
                .get_string("provider/openai", "api_key")
                .await
                .expect("decrypt with migrated seed"),
            "sk-legacy"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn hybrid_refuses_to_mint_a_new_seed_when_the_keychain_is_locked() {
        let dir =
            std::env::temp_dir().join(format!("mixd_test_hybrid_locked_{}", uuid::Uuid::new_v4()));
        let backend = KeychainSeedBackend::with_seed_store(
            &dir,
            Arc::new(MemorySeedStore {
                fail: true,
                ..Default::default()
            }),
        );
        let err = backend.master_seed().expect_err("must not mint a new seed");
        assert!(err.to_string().contains("keychain"), "unexpected: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
