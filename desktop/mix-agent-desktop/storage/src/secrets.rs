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
//!   the Keychain. Phase 6 hooks the actual SQLCipher passphrase wiring.

use std::sync::OnceLock;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{anyhow, Context};
use argon2::password_hash::SaltString;
use argon2::{Algorithm, Argon2, Params, Version};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use keyring::Entry;
use serde::{Deserialize, Serialize};

const SERVICE: &str = "io.github.taitai2661.mix-agent-desktop";
const DB_KEY_PURPOSE: &str = "db.master";
const MASTER_SEED_SLOT: &str = "_master_seed";

#[derive(Default)]
pub struct SecretStore {}

/// Every entry point here talks to the OS Keychain and runs Argon2; both are
/// blocking. Called from async Tauri commands (once per provider row, even),
/// they used to occupy a runtime worker thread for tens of milliseconds each.
macro_rules! offload {
    ($body:expr) => {
        tokio::task::spawn_blocking($body)
            .await
            .map_err(|e| anyhow!("secret store task failed: {e}"))?
    };
}

impl SecretStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn put(&self, purpose: &str, name: &str, value: &[u8]) -> anyhow::Result<()> {
        let (purpose, name, value) = (purpose.to_string(), name.to_string(), value.to_vec());
        offload!(move || put_blocking(&purpose, &name, &value))
    }

    pub async fn get(&self, purpose: &str, name: &str) -> anyhow::Result<Vec<u8>> {
        let (purpose, name) = (purpose.to_string(), name.to_string());
        offload!(move || get_blocking(&purpose, &name))
    }

    pub async fn get_string(&self, purpose: &str, name: &str) -> anyhow::Result<String> {
        let bytes = self.get(purpose, name).await?;
        String::from_utf8(bytes).map_err(|_| anyhow!("secret is not valid UTF-8"))
    }

    pub async fn delete(&self, purpose: &str, name: &str) -> anyhow::Result<()> {
        let (purpose, name) = (purpose.to_string(), name.to_string());
        offload!(move || delete_blocking(&purpose, &name))
    }

    /// Ensure the master DB encryption key exists in the Keychain.
    pub async fn ensure_db_key(&self) -> anyhow::Result<[u8; 32]> {
        if let Ok(plain) = self.get(DB_KEY_PURPOSE, "master").await {
            if plain.len() == 32 {
                let mut key = [0u8; 32];
                key.copy_from_slice(&plain);
                return Ok(key);
            }
        }
        use rand::RngCore;
        let mut rng = rand::thread_rng();
        let mut bytes = [0u8; 32];
        rng.fill_bytes(&mut bytes);
        self.put(DB_KEY_PURPOSE, "master", &bytes).await?;
        Ok(bytes)
    }

    pub async fn db_master_key(&self) -> anyhow::Result<[u8; 32]> {
        self.ensure_db_key().await
    }

    /// Reachability check for the diagnostics pane.
    ///
    /// Reading a slot that is allowed to be absent proves the Keychain answers
    /// at all; a locked or permission-denied Keychain surfaces as an error.
    pub async fn probe(&self) -> anyhow::Result<()> {
        offload!(|| {
            let entry = Entry::new(SERVICE, "_diagnostics_probe")?;
            match entry.get_password() {
                Ok(_) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(err) => Err(anyhow!("Keychain is not reachable: {err}")),
            }
        })
    }
}

fn put_blocking(purpose: &str, name: &str, value: &[u8]) -> anyhow::Result<()> {
    let slot = keyring_key(purpose, name);
    let encrypted = encrypt_with_master(purpose, name, value)?;
    Entry::new(SERVICE, &slot)?.set_password(&encrypted)?;
    Ok(())
}

fn get_blocking(purpose: &str, name: &str) -> anyhow::Result<Vec<u8>> {
    let slot = keyring_key(purpose, name);
    let entry = Entry::new(SERVICE, &slot)?;
    let cipher = entry.get_password()?;
    decrypt_with_master(purpose, name, cipher.as_bytes())
}

fn delete_blocking(purpose: &str, name: &str) -> anyhow::Result<()> {
    let slot = keyring_key(purpose, name);
    let entry = Entry::new(SERVICE, &slot)?;
    match entry.delete_credential() {
        Ok(_) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(err.into()),
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

/// The Keychain-bound master key, derived once per process.
///
/// Argon2id at the default parameters costs tens of milliseconds, and this ran
/// on every `get`/`put` — i.e. once per provider row when listing them.
fn master_key() -> anyhow::Result<[u8; 32]> {
    static CACHED: OnceLock<[u8; 32]> = OnceLock::new();
    if let Some(key) = CACHED.get() {
        return Ok(*key);
    }
    // Two first-touch calls can run at once (secret access goes through
    // `spawn_blocking`), and each would generate its *own* seed. The Keychain
    // keeps whichever wrote last, so everything sealed under the other one
    // became permanently undecryptable.
    static SEED_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = SEED_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(key) = CACHED.get() {
        return Ok(*key);
    }
    let seed = load_master_seed()?;
    let key = derive_master_key(&seed)?;
    let _ = CACHED.set(key);
    // Whoever wins the cache wins for the process, and their key is the one
    // derived from the seed the Keychain now holds.
    Ok(*CACHED.get().expect("master key is cached"))
}

fn load_master_seed() -> anyhow::Result<String> {
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
            // Not `.ok()`: if this write fails the next launch generates a
            // different seed and every stored payload becomes undecryptable.
            entry
                .set_password(&seed)
                .context("storing the master seed in the Keychain")?;
            Ok(seed)
        }
        Err(err) => Err(anyhow!(err).context("reading the master seed from the Keychain")),
    }
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

fn encrypt_with_master(purpose: &str, name: &str, plaintext: &[u8]) -> anyhow::Result<String> {
    let master = master_key()?;
    let cipher = Aes256Gcm::new_from_slice(&master).map_err(|e| anyhow!("cipher: {}", e))?;
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

fn decrypt_with_master(purpose: &str, name: &str, payload: &[u8]) -> anyhow::Result<Vec<u8>> {
    let master = master_key()?;
    let cipher = Aes256Gcm::new_from_slice(&master).map_err(|e| anyhow!("cipher: {}", e))?;
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
}
