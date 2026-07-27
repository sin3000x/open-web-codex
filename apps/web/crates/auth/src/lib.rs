//! Password hashing and compatibility verification for platform identities.

use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use sha2::{Digest, Sha256};
use std::sync::OnceLock;
use subtle::ConstantTimeEq;
use thiserror::Error;

const LEGACY_SHA256_HEX_LENGTH: usize = 64;

#[derive(Debug, Error)]
pub enum PasswordError {
    #[error("password must not be empty")]
    Empty,
    #[error("password hashing failed")]
    Hash,
}

pub fn hash_password(password: &str) -> Result<String, PasswordError> {
    if password.is_empty() {
        return Err(PasswordError::Empty);
    }
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| PasswordError::Hash)
}

pub fn verify_password(password: &str, encoded: &str) -> bool {
    if let Ok(hash) = PasswordHash::new(encoded) {
        return Argon2::default()
            .verify_password(password.as_bytes(), &hash)
            .is_ok();
    }
    verify_legacy_sha256(password, encoded)
}

/// Verify an optional account hash while still doing an Argon2 verification
/// for unknown accounts, reducing account-existence timing differences.
pub fn verify_password_or_dummy(password: &str, encoded: Option<&str>) -> bool {
    static DUMMY_HASH: OnceLock<String> = OnceLock::new();
    let exists = encoded.is_some();
    let encoded = encoded.unwrap_or_else(|| {
        DUMMY_HASH
            .get_or_init(|| {
                hash_password("open-web-codex-unavailable-account")
                    .expect("static dummy password is valid")
            })
            .as_str()
    });
    verify_password(password, encoded) && exists
}

pub fn needs_rehash(encoded: &str) -> bool {
    !encoded.starts_with("$argon2id$")
}

fn verify_legacy_sha256(password: &str, encoded: &str) -> bool {
    if encoded.len() != LEGACY_SHA256_HEX_LENGTH
        || !encoded.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return false;
    }
    let candidate = hex::encode(Sha256::digest(password.as_bytes()));
    bool::from(candidate.as_bytes().ct_eq(encoded.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::{hash_password, needs_rehash, verify_password, verify_password_or_dummy};
    use sha2::{Digest, Sha256};

    #[test]
    fn hashes_with_argon2id_and_verifies_without_exposing_the_password() {
        let password = "correct horse battery staple";
        let encoded = hash_password(password).expect("hash password");

        assert!(encoded.starts_with("$argon2id$"));
        assert!(!encoded.contains(password));
        assert!(verify_password(password, &encoded));
        assert!(!verify_password("wrong", &encoded));
        assert!(!needs_rehash(&encoded));
    }

    #[test]
    fn accepts_legacy_sha256_only_for_an_in_place_upgrade() {
        let legacy = hex::encode(Sha256::digest(b"legacy-password"));

        assert!(verify_password("legacy-password", &legacy));
        assert!(!verify_password("wrong", &legacy));
        assert!(needs_rehash(&legacy));
    }

    #[test]
    fn unknown_accounts_use_the_dummy_verification_path() {
        assert!(!verify_password_or_dummy("anything", None));
    }
}
