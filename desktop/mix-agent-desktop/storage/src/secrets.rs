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

use std::collections::HashMap;

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
pub struct SecretStore {
    #[allow(dead_code)]
    cache: tokio::sync::Mutex<HashMap<String, Vec<u8>>>,
}

impl SecretStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn put(&self, purpose: &str, name: &str, value: &[u8]) -> anyhow::Result<()> {
        let slot = keyring_key(purpose, name);
        let encrypted = encrypt_with_master(purpose, name, value)?;
        Entry::new(SERVICE, &slot)?.set_password(&encrypted)?;
        Ok(())
    }

    pub fn get(&self, purpose: &str, name: &str) -> anyhow::Result<Vec<u8>> {
        let slot = keyring_key(purpose, name);
        let entry = Entry::new(SERVICE, &slot)?;
        let cipher = entry.get_password()?;
        let plain = decrypt_with_master(purpose, name, cipher.as_bytes())?;
        Ok(plain)
    }

    pub fn get_string(&self, purpose: &str, name: &str) -> anyhow::Result<String> {
        let bytes = self.get(purpose, name)?;
        String::from_utf8(bytes).map_err(|_| anyhow!("secret is not valid UTF-8"))
    }

    pub fn delete(&self, purpose: &str, name: &str) -> anyhow::Result<()> {
        let slot = keyring_key(purpose, name);
        let entry = Entry::new(SERVICE, &slot)?;
        match entry.delete_credential() {
            Ok(_) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(err.into()),
        }
    }

    /// Ensure the master DB encryption key exists in the Keychain.
    pub fn ensure_db_key(&self) -> anyhow::Result<[u8; 32]> {
        if let Ok(plain) = self.get(DB_KEY_PURPOSE, "master") {
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
        self.put(DB_KEY_PURPOSE, "master", &bytes)?;
        Ok(bytes)
    }

    pub fn db_master_key(&self) -> anyhow::Result<[u8; 32]> {
        self.ensure_db_key()
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

fn master_key() -> anyhow::Result<[u8; 32]> {
    let entry = Entry::new(SERVICE, MASTER_SEED_SLOT)?;
    let seed = match entry.get_password() {
        Ok(s) => s,
        Err(_) => {
            use rand::RngCore;
            let mut rng = rand::thread_rng();
            let mut buf = [0u8; 32];
            rng.fill_bytes(&mut buf);
            let s = BASE64.encode(buf);
            entry.set_password(&s).ok();
            s
        }
    };
    let salt = SaltString::from_b64(&BASE64.encode(format!("{}|master", SERVICE)))
        .map_err(|e| anyhow!("salt: {}", e))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, Params::default());
    let mut out = [0u8; 32];
    argon
        .hash_password_into(seed.as_bytes(), salt.as_str().as_bytes(), &mut out)
        .map_err(|e| anyhow!("argon2: {}", e))?;
    Ok(out)
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
