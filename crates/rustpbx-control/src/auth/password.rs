//! Password hashing for tenant IAM accounts.
//!
//! Uses bcrypt (self-salting, adaptive cost) so we never store plaintext. The
//! superadmin account stays config-driven (plaintext compare) for now — only
//! DB-backed tenant users are hashed here.

use anyhow::{Result, anyhow};

/// Hash a plaintext password for storage.
pub fn hash_password(plain: &str) -> Result<String> {
    bcrypt::hash(plain, bcrypt::DEFAULT_COST).map_err(|e| anyhow!("hash failed: {e}"))
}

/// Verify a plaintext password against a stored bcrypt hash. Never panics — a
/// malformed stored hash simply fails verification.
pub fn verify_password(plain: &str, hash: &str) -> bool {
    bcrypt::verify(plain, hash).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_then_verify_roundtrips() {
        let h = hash_password("s3cret!").unwrap();
        assert!(h != "s3cret!", "must not store plaintext");
        assert!(verify_password("s3cret!", &h));
        assert!(!verify_password("wrong", &h));
    }

    #[test]
    fn verify_rejects_garbage_hash() {
        assert!(!verify_password("anything", "not-a-bcrypt-hash"));
    }
}
