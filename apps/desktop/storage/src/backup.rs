//! Encrypted backup and restore.
//!
//! A backup is a single file that can safely be emailed or copied to a USB
//! stick, so everything in it is encrypted with a key derived from the user's
//! password. Nothing sensitive is stored in the clear.
//!
//! # Container format (`v1`)
//!
//! ```text
//! magic   8 bytes   b"MIXBAK\x01\x00"
//! salt   16 bytes   Argon2id salt
//! nonce  12 bytes   AES-256-GCM nonce
//! body    n bytes   deflate( AES-256-GCM( deflate( tar( payload ) ) ) )
//! ```
//!
//! The payload is a single tar entry containing the SQLite snapshot plus a
//! JSON manifest, which keeps the format easy to extend without a real
//! multi-file archiver dependency.
//!
//! Secrets (API keys, OAuth tokens) are intentionally **not** included: they
//! live in the OS Keychain and are re-enterable, and bundling them would mean
//! writing decrypted key material to disk.

use std::io::Write;
use std::path::Path;

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{anyhow, bail, Context};
use argon2::password_hash::SaltString;
use argon2::{Algorithm, Argon2, Params, Version};

/// Bumped when the container layout changes incompatibly.
const MAGIC: &[u8; 8] = b"MIXBAK\x01\x00";
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;

/// What a backup contains, alongside the encrypted blob.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BackupManifest {
    /// Format version of the payload, not of the container.
    pub payload_version: u32,
    pub created_at: String,
    /// Name of the SQLite file inside the payload.
    pub db_file: String,
    pub db_bytes: u64,
    /// Tables present at snapshot time, as a human-readable sanity check.
    pub tables: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BackupInfo {
    pub path: String,
    pub bytes: u64,
    pub manifest: BackupManifest,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RestoreInfo {
    /// Path the snapshot was decrypted to; the caller promotes it on restart.
    pub staged_path: String,
    pub manifest: BackupManifest,
}

fn derive_key(password: &str, salt: &[u8]) -> anyhow::Result<[u8; KEY_LEN]> {
    // Reject an empty password up front rather than silently producing a
    // well-known key.
    if password.is_empty() {
        bail!("a backup password is required");
    }
    let salt = SaltString::encode_b64(salt).map_err(|e| anyhow!("salt encoding failed: {e}"))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, Params::default());
    let mut key = [0u8; KEY_LEN];
    argon
        .hash_password_into(password.as_bytes(), salt.as_str().as_bytes(), &mut key)
        .map_err(|e| anyhow!("key derivation failed: {e}"))?;
    Ok(key)
}

/// Minimal uncompressed tar writer, so the payload stays a valid archive and
/// can be inspected with standard tools.
fn tar_single(name: &str, data: &[u8]) -> Vec<u8> {
    let mut header = [0u8; 512];
    let name_bytes = name.as_bytes();
    header[..name_bytes.len()].copy_from_slice(name_bytes);

    // mode, uid, gid are octal ASCII with a trailing NUL.
    let put = |h: &mut [u8; 512], off: usize, len: usize, val: &str| {
        let b = val.as_bytes();
        h[off..off + b.len().min(len - 1)].copy_from_slice(&b[..b.len().min(len - 1)]);
    };
    put(&mut header, 100, 8, "0000644");
    put(&mut header, 108, 8, "0000000");
    put(&mut header, 116, 8, "0000000");
    // size: 11 octal digits + NUL
    let size = format!("{:011o}", data.len());
    put(&mut header, 124, 12, &size);
    put(&mut header, 136, 12, "00000000000"); // mtime
    header[156] = b'0'; // regular file
                        // checksum: spaces while summing, then the octal value.
    for b in header.iter_mut().skip(148).take(8) {
        *b = b' ';
    }
    let sum: u32 = header.iter().map(|&b| b as u32).sum();
    let chk = format!("{:06o}\0 ", sum);
    header[148..148 + chk.len()].copy_from_slice(chk.as_bytes());

    let mut out = Vec::with_capacity(512 + data.len() + 1024);
    out.extend_from_slice(&header);
    out.extend_from_slice(data);
    let pad = (512 - (data.len() % 512)) % 512;
    out.extend(std::iter::repeat_n(0u8, pad));
    // Two zero blocks terminate the archive.
    out.extend(std::iter::repeat_n(0u8, 1024));
    out
}

/// Read the single regular-file entry out of an uncompressed tar.
fn tar_read(bytes: &[u8]) -> anyhow::Result<(String, Vec<u8>)> {
    if bytes.len() < 512 {
        bail!("payload is not a tar archive");
    }
    let header = &bytes[..512];
    let name: String = header[..100]
        .iter()
        .take_while(|&&b| b != 0)
        .map(|&b| b as char)
        .collect();
    let size_field: String = header[124..136]
        .iter()
        .take_while(|&&b| b != 0 && b != b' ')
        .map(|&b| b as char)
        .collect();
    let size = usize::from_str_radix(&size_field, 8).context("tar size field is not octal")?;
    if 512 + size > bytes.len() {
        bail!("tar entry is truncated");
    }
    Ok((name, bytes[512..512 + size].to_vec()))
}

fn compress(data: &[u8]) -> anyhow::Result<Vec<u8>> {
    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
    enc.write_all(data)?;
    Ok(enc.finish()?)
}

fn decompress(data: &[u8]) -> anyhow::Result<Vec<u8>> {
    use flate2::write::ZlibDecoder;
    let mut dec = ZlibDecoder::new(Vec::new());
    dec.write_all(data)?;
    Ok(dec.finish()?)
}

/// Consistent SQLite snapshot, safe to take while the app is running.
async fn snapshot_db(db_path: &Path, dest: &Path) -> anyhow::Result<u64> {
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let url = format!("sqlite://{}", db_path.display());
    let opts = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(false);
    // VACUUM INTO is the only way to get a consistent copy out of a live WAL
    // database; copying the file by hand can capture a torn page.
    let _ = url;
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .context("opening the database for backup")?;
    if dest.exists() {
        tokio::fs::remove_file(dest).await.ok();
    }
    sqlx::query("VACUUM INTO ?")
        .bind(dest.to_string_lossy().to_string())
        .execute(&pool)
        .await
        .context("VACUUM INTO failed")?;
    pool.close().await;
    Ok(tokio::fs::metadata(dest).await?.len())
}

/// Write an encrypted backup of the app data directory.
pub async fn create(
    data_dir: impl AsRef<Path>,
    destination: impl AsRef<Path>,
    password: &str,
) -> anyhow::Result<BackupInfo> {
    match create_inner(data_dir, destination, password).await {
        Ok(info) => {
            tracing::info!(path = %info.path, bytes = info.bytes, "backup written");
            Ok(info)
        }
        Err(err) => {
            tracing::error!(%err, "creating an encrypted backup failed");
            Err(err)
        }
    }
}

/// The actual work behind [`create`], split out so the entry point can log the
/// outcome without rebuilding (and so losing) the error's context chain.
async fn create_inner(
    data_dir: impl AsRef<Path>,
    destination: impl AsRef<Path>,
    password: &str,
) -> anyhow::Result<BackupInfo> {
    let data_dir = data_dir.as_ref();
    let destination = destination.as_ref();
    let db_path = data_dir.join("mix-agent.db");
    if !db_path.is_file() {
        bail!("no database found at {}", db_path.display());
    }

    let staging = data_dir.join("backup-staging.db");
    let db_bytes = snapshot_db(&db_path, &staging).await?;

    // Read the staged snapshot back to collect a table inventory.
    let (tables, db_blob) = {
        let bytes = tokio::fs::read(&staging).await?;
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&staging)
            .create_if_missing(false);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .context("reopening the snapshot")?;
        let names: Vec<String> =
            sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
                .fetch_all(&pool)
                .await
                .unwrap_or_default();
        pool.close().await;
        (names, bytes)
    };
    tokio::fs::remove_file(&staging).await.ok();

    let manifest = BackupManifest {
        payload_version: 1,
        created_at: chrono::Utc::now().to_rfc3339(),
        db_file: "mix-agent.db".to_string(),
        db_bytes,
        tables,
    };

    let mut payload = Vec::new();
    payload.extend_from_slice(&serde_json::to_vec_pretty(&manifest)?);
    payload.extend_from_slice(b"\n---MIX-DB---\n");
    payload.extend_from_slice(&db_blob);

    let archive = tar_single("mix-agent-backup.tar", &payload);
    let compressed = compress(&archive)?;

    let mut salt = [0u8; SALT_LEN];
    let mut nonce_raw = [0u8; NONCE_LEN];
    aes_gcm::aead::rand_core::RngCore::fill_bytes(&mut OsRng, &mut salt);
    aes_gcm::aead::rand_core::RngCore::fill_bytes(&mut OsRng, &mut nonce_raw);
    let key = derive_key(password, &salt)?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| anyhow!("cipher init failed: {e}"))?;
    let nonce_bytes = Nonce::from_slice(&nonce_raw);
    let ciphertext = cipher
        .encrypt(nonce_bytes, compressed.as_ref())
        .map_err(|e| anyhow!("encryption failed: {e}"))?;

    if let Some(parent) = destination.parent() {
        if !parent.as_os_str().is_empty() {
            tokio::fs::create_dir_all(parent).await?;
        }
    }
    let mut out = Vec::with_capacity(MAGIC.len() + SALT_LEN + NONCE_LEN + ciphertext.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&salt);
    out.extend_from_slice(&nonce_raw);
    out.extend_from_slice(&ciphertext);

    // Write to a sibling temp file then rename, so an interrupted write never
    // leaves a half-finished backup where a full one used to be.
    let tmp = destination.with_extension("mixbak-part");
    tokio::fs::write(&tmp, &out).await?;
    tokio::fs::rename(&tmp, destination).await?;

    Ok(BackupInfo {
        path: destination.to_string_lossy().to_string(),
        bytes: out.len() as u64,
        manifest,
    })
}

/// Decrypt a backup and stage the database for the next launch.
///
/// The snapshot is written to `<data_dir>/mix-agent.restored.db` rather than
/// swapped in immediately: the running process holds an open SQLite pool, and
/// replacing the file underneath it would risk corruption. `Storage::open`
/// promotes the staged file on the next start.
pub async fn restore(
    data_dir: impl AsRef<Path>,
    source: impl AsRef<Path>,
    password: &str,
) -> anyhow::Result<RestoreInfo> {
    match restore_inner(data_dir, source, password).await {
        Ok(info) => {
            tracing::info!(staged = %info.staged_path, "backup restored and staged");
            Ok(info)
        }
        Err(err) => {
            tracing::error!(%err, "restoring a backup failed");
            Err(err)
        }
    }
}

/// The actual work behind [`restore`]; see [`create_inner`].
async fn restore_inner(
    data_dir: impl AsRef<Path>,
    source: impl AsRef<Path>,
    password: &str,
) -> anyhow::Result<RestoreInfo> {
    let data_dir = data_dir.as_ref();
    let source = source.as_ref();
    let blob = tokio::fs::read(source)
        .await
        .with_context(|| format!("reading {}", source.display()))?;

    if blob.len() < MAGIC.len() + SALT_LEN + NONCE_LEN {
        bail!("file is too small to be a MIX-agent backup");
    }
    if &blob[..MAGIC.len()] != MAGIC {
        bail!("not a MIX-agent backup (bad magic)");
    }
    let mut offset = MAGIC.len();
    let salt = &blob[offset..offset + SALT_LEN];
    offset += SALT_LEN;
    let nonce_raw: [u8; NONCE_LEN] = blob[offset..offset + NONCE_LEN]
        .try_into()
        .map_err(|_| anyhow!("truncated nonce"))?;
    let nonce = Nonce::from_slice(&nonce_raw);
    offset += NONCE_LEN;
    let ciphertext = &blob[offset..];

    let key = derive_key(password, salt)?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| anyhow!("cipher init failed: {e}"))?;
    // A wrong password shows up as an authentication failure here.
    let compressed = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| anyhow!("could not decrypt: wrong password, or the file is corrupt"))?;

    let archive = decompress(&compressed)?;
    let (_name, payload) = tar_read(&archive)?;
    let sep = b"\n---MIX-DB---\n";
    let split = payload
        .windows(sep.len())
        .position(|w| w == sep)
        .ok_or_else(|| anyhow!("backup payload is malformed"))?;
    let manifest: BackupManifest = serde_json::from_slice(&payload[..split])
        .map_err(|e| anyhow!("backup manifest is malformed: {e}"))?;
    let db_blob = &payload[split + sep.len()..];

    if db_blob.is_empty() {
        bail!("backup contains no database");
    }
    // Reject a payload that is not actually SQLite, so a corrupt backup cannot
    // be promoted over good data.
    if !db_blob.starts_with(b"SQLite format 3\0") {
        bail!("backup database is not a SQLite file");
    }

    let staged = data_dir.join("mix-agent.restored.db");
    // Stage through a sidecar and rename: a crash mid-write used to leave a
    // truncated `mix-agent.restored.db`, which the next launch then promoted
    // over the live database.
    let partial = data_dir.join("mix-agent.restored.db.part");
    tokio::fs::remove_file(&partial).await.ok();
    tokio::fs::write(&partial, db_blob).await?;
    tokio::fs::rename(&partial, &staged).await?;
    Ok(RestoreInfo {
        staged_path: staged.to_string_lossy().to_string(),
        manifest,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tar_round_trips() {
        let payload = b"hello backup".to_vec();
        let archive = tar_single("x.tar", &payload);
        let (name, body) = tar_read(&archive).expect("read");
        assert_eq!(name, "x.tar");
        assert_eq!(body, payload);
    }

    #[test]
    fn tar_handles_multi_block_padding() {
        // 600 bytes forces a second data block.
        let payload = vec![7u8; 600];
        let archive = tar_single("x.tar", &payload);
        let (name, body) = tar_read(&archive).expect("read");
        assert_eq!(name, "x.tar");
        assert_eq!(body.len(), 600);
        assert!(body.iter().all(|b| *b == 7));
    }

    #[test]
    fn compress_round_trips() {
        let data = "compress me".repeat(200).into_bytes();
        let c = compress(&data).expect("compress");
        assert!(c.len() < data.len(), "repetitive input should shrink");
        assert_eq!(decompress(&c).expect("decompress"), data);
    }

    #[test]
    fn wrong_password_is_rejected() {
        let key = derive_key("correct horse", &[7u8; SALT_LEN]).expect("key");
        let cipher = Aes256Gcm::new_from_slice(&key).expect("cipher");
        let mut nonce_raw = [0u8; NONCE_LEN];
        aes_gcm::aead::rand_core::RngCore::fill_bytes(&mut OsRng, &mut nonce_raw);
        let nonce = Nonce::from_slice(&nonce_raw);
        let ct = cipher.encrypt(nonce, b"payload".as_ref()).expect("encrypt");
        assert!(cipher.decrypt(nonce, ct.as_ref()).is_ok());
        // A different key must fail closed.
        let other = derive_key("battery staple", &[7u8; SALT_LEN]).expect("key");
        let other_cipher = Aes256Gcm::new_from_slice(&other).expect("cipher");
        assert!(other_cipher.decrypt(nonce, ct.as_ref()).is_err());
    }

    #[test]
    fn empty_password_is_refused() {
        assert!(derive_key("", &[1u8; SALT_LEN]).is_err());
    }
}
