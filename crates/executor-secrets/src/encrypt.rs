//! ChaCha20-Poly1305 box around the default JSON secret store.

use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use executor_core::{ExecutorError, StorageError};
use sha2::{Digest, Sha256};

use super::{FileSecrets, MemorySecrets};

const MAGIC: &[u8] = b"EXS1";

/// Resolve a 32-byte key from hex env or a 0600 key file.
///
/// # Errors
///
/// Illegal hex or IO.
pub fn load_or_create_key(key_path: &std::path::Path) -> Result<[u8; 32], ExecutorError> {
    if let Ok(hex_key) = std::env::var("EXECUTOR_SECRET_KEY") {
        return decode_key(&hex_key);
    }
    if key_path.is_file() {
        let text = std::fs::read_to_string(key_path).map_err(StorageError::new)?;
        return decode_key(text.trim());
    }
    let key = random_key();
    if let Some(parent) = key_path.parent() {
        std::fs::create_dir_all(parent).map_err(StorageError::new)?;
    }
    std::fs::write(key_path, hex::encode(key)).map_err(StorageError::new)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(key_path)
            .map_err(StorageError::new)?
            .permissions();
        perms.set_mode(0o600);
        std::fs::set_permissions(key_path, perms).map_err(StorageError::new)?;
    }
    Ok(key)
}

fn decode_key(hex_key: &str) -> Result<[u8; 32], ExecutorError> {
    let bytes = hex::decode(hex_key).map_err(|e| StorageError::new(e.to_string()))?;
    let digest = if bytes.len() == 32 {
        bytes
    } else {
        Sha256::digest(hex_key.as_bytes()).to_vec()
    };
    let mut key = [0_u8; 32];
    key.copy_from_slice(&digest[..32]);
    Ok(key)
}

fn random_key() -> [u8; 32] {
    let generated = ChaCha20Poly1305::generate_key(&mut OsRng);
    let mut key = [0_u8; 32];
    key.copy_from_slice(generated.as_slice());
    key
}

pub fn encrypt(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>, ExecutorError> {
    let cipher = ChaCha20Poly1305::new(&Key::from(*key));
    let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|e| StorageError::new(e.to_string()))?;
    let mut out = Vec::with_capacity(MAGIC.len() + nonce.len() + ciphertext.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(nonce.as_slice());
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

pub fn decrypt(key: &[u8; 32], blob: &[u8]) -> Result<Vec<u8>, ExecutorError> {
    if blob.starts_with(b"{") {
        return Ok(blob.to_vec());
    }
    if blob.len() < MAGIC.len() + 12 || !blob.starts_with(MAGIC) {
        return Err(ExecutorError::CredentialResolution(
            "secrets file is not an executor box".into(),
        ));
    }
    let nonce_bytes: [u8; 12] = blob[MAGIC.len()..MAGIC.len() + 12]
        .try_into()
        .map_err(|_| ExecutorError::CredentialResolution("truncated nonce".into()))?;
    let nonce = Nonce::from(nonce_bytes);
    let cipher = ChaCha20Poly1305::new(&Key::from(*key));
    cipher
        .decrypt(&nonce, &blob[MAGIC.len() + 12..])
        .map_err(|e| ExecutorError::CredentialResolution(format!("decrypt: {e}")))
}

impl FileSecrets {
    /// Open `path`, decrypting with `key` when the file is an `EXS1` box.
    ///
    /// # Errors
    ///
    /// IO, decrypt, or JSON.
    pub fn open_encrypted(
        path: impl Into<std::path::PathBuf>,
        key: [u8; 32],
    ) -> Result<Self, ExecutorError> {
        let path = path.into();
        let inner = MemorySecrets::new();
        if path.is_file() {
            let blob = std::fs::read(&path).map_err(StorageError::new)?;
            let text = decrypt(&key, &blob)?;
            let map: std::collections::BTreeMap<String, String> =
                serde_json::from_slice(&text).map_err(StorageError::new)?;
            inner.load_map(map);
        }
        Ok(Self {
            path,
            inner,
            key: Some(key),
        })
    }
}
