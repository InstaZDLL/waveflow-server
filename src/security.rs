//! Password hashing, opaque tokens and instance-key encryption.

use std::{fs::OpenOptions, io::Write, path::Path};

use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Nonce,
};
use rand::{rngs::SysRng, TryRng};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

const INSTANCE_KEY_BYTES: usize = 32;
const NONCE_BYTES: usize = 12;

#[derive(Debug, thiserror::Error)]
pub enum SecurityError {
    #[error("password hashing failed")]
    PasswordHash,
    #[error("password verification failed")]
    PasswordVerify,
    #[error("instance key must contain exactly 32 bytes")]
    InvalidInstanceKey,
    #[error("system randomness is unavailable")]
    Random,
    #[error("secret encryption failed")]
    Encrypt,
    #[error("secret decryption failed")]
    Decrypt,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Clone)]
pub struct SecretBox {
    cipher: ChaCha20Poly1305,
    key: [u8; INSTANCE_KEY_BYTES],
}

pub struct EncryptedSecret {
    pub nonce: [u8; NONCE_BYTES],
    pub ciphertext: Vec<u8>,
}

impl SecretBox {
    pub fn load_or_create(path: &Path) -> Result<Self, SecurityError> {
        let key = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut bytes = vec![0u8; INSTANCE_KEY_BYTES];
                fill_random(&mut bytes)?;
                write_new_secret_file(path, &bytes)?;
                // Re-read so a concurrent process that won create_new supplies
                // the key both processes actually use.
                std::fs::read(path)?
            }
            Err(error) => return Err(error.into()),
        };

        Self::from_key_bytes(&key)
    }

    pub fn from_key_bytes(key: &[u8]) -> Result<Self, SecurityError> {
        let key: [u8; INSTANCE_KEY_BYTES] = key
            .try_into()
            .map_err(|_| SecurityError::InvalidInstanceKey)?;
        Ok(Self {
            cipher: ChaCha20Poly1305::new_from_slice(&key)
                .map_err(|_| SecurityError::InvalidInstanceKey)?,
            key,
        })
    }

    /// Derives a stable, unforgeable bearer token without persisting any
    /// reversible token material. The domain label prevents reuse for another
    /// keyed purpose from producing the same output.
    pub fn derive_share_token(&self, share_id: uuid::Uuid) -> String {
        let mut hasher = blake3::Hasher::new_keyed(&self.key);
        hasher.update(b"waveflow/share-token/v1\0");
        hasher.update(share_id.as_bytes());
        format!(
            "wfs_{}",
            URL_SAFE_NO_PAD.encode(hasher.finalize().as_bytes())
        )
    }

    pub fn encrypt(&self, plaintext: &[u8]) -> Result<EncryptedSecret, SecurityError> {
        let mut nonce = [0u8; NONCE_BYTES];
        fill_random(&mut nonce)?;
        let cipher_nonce = Nonce::from(nonce);
        let ciphertext = self
            .cipher
            .encrypt(&cipher_nonce, plaintext)
            .map_err(|_| SecurityError::Encrypt)?;
        Ok(EncryptedSecret { nonce, ciphertext })
    }

    pub fn decrypt(&self, nonce: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, SecurityError> {
        let nonce: [u8; NONCE_BYTES] = nonce.try_into().map_err(|_| SecurityError::Decrypt)?;
        let cipher_nonce = Nonce::from(nonce);
        self.cipher
            .decrypt(&cipher_nonce, ciphertext)
            .map_err(|_| SecurityError::Decrypt)
    }
}

/// Random bytes straight from the operating system.
///
/// `rand` 0.10 renamed `OsRng` to `SysRng` and made it fallible where the
/// previous version panicked on its own. Callers whose signature can carry the
/// failure now report it; [`generate_token`] cannot change shape, and it keeps
/// the panic it already had — a system CSPRNG that refuses to answer leaves
/// nothing to fall back on, and a token drawn from anything else would be
/// worse than no token.
fn fill_random(dst: &mut [u8]) -> Result<(), SecurityError> {
    SysRng
        .try_fill_bytes(dst)
        .map_err(|_| SecurityError::Random)
}

pub fn hash_password(password: &str) -> Result<String, SecurityError> {
    if password.len() < 12 {
        return Err(SecurityError::PasswordHash);
    }
    let mut salt_bytes = [0u8; 16];
    fill_random(&mut salt_bytes)?;
    let salt = SaltString::encode_b64(&salt_bytes).map_err(|_| SecurityError::PasswordHash)?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| SecurityError::PasswordHash)
}

pub fn verify_password(password: &str, encoded_hash: &str) -> Result<bool, SecurityError> {
    let parsed = PasswordHash::new(encoded_hash).map_err(|_| SecurityError::PasswordVerify)?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

pub fn generate_token(prefix: &str) -> String {
    let mut bytes = [0u8; 32];
    fill_random(&mut bytes).expect("the system CSPRNG must answer");
    format!("{prefix}{}", URL_SAFE_NO_PAD.encode(bytes))
}

pub fn token_hash(token: &str) -> [u8; 32] {
    bytes_hash(token.as_bytes())
}

pub fn bytes_hash(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

pub fn constant_time_bytes_eq(left: &[u8], right: &[u8]) -> bool {
    left.ct_eq(right).into()
}

pub fn token_matches(token: &str, expected_hash: &[u8]) -> bool {
    let actual = token_hash(token);
    actual.as_slice().ct_eq(expected_hash).into()
}

fn write_new_secret_file(path: &Path, bytes: &[u8]) -> Result<(), SecurityError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    match options.open(path) {
        Ok(mut file) => {
            file.write_all(bytes)?;
            file.sync_all()?;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_round_trip() {
        let hash = hash_password("correct horse battery staple").unwrap();
        assert!(verify_password("correct horse battery staple", &hash).unwrap());
        assert!(!verify_password("incorrect password", &hash).unwrap());
        assert!(!hash.contains("correct horse"));
    }

    #[test]
    fn encrypted_secret_round_trip() {
        let secret_box = SecretBox::from_key_bytes(&[7; 32]).unwrap();
        let encrypted = secret_box.encrypt(b"subsonic-only-password").unwrap();
        assert_ne!(encrypted.ciphertext, b"subsonic-only-password");
        assert_eq!(
            secret_box
                .decrypt(&encrypted.nonce, &encrypted.ciphertext)
                .unwrap(),
            b"subsonic-only-password"
        );
    }

    #[test]
    fn share_tokens_are_stable_and_bound_to_the_instance_and_share() {
        let secret = SecretBox::from_key_bytes(&[7; 32]).unwrap();
        let other_secret = SecretBox::from_key_bytes(&[8; 32]).unwrap();
        let share = uuid::Uuid::new_v4();

        assert_eq!(
            secret.derive_share_token(share),
            secret.derive_share_token(share)
        );
        assert_ne!(
            secret.derive_share_token(share),
            secret.derive_share_token(uuid::Uuid::new_v4())
        );
        assert_ne!(
            secret.derive_share_token(share),
            other_secret.derive_share_token(share)
        );
    }
}
