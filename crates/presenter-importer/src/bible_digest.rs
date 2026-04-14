//! Source-archive digest for bible imports.
//!
//! The digest is `SHA-256(file_bytes || parser_version_le_bytes)`. Bumping
//! `PARSER_VERSION` forces re-imports even when the source archive bytes
//! are unchanged — use this when the parser logic changes in a way that
//! would produce different passages from the same archive.

use sha2::{Digest, Sha256};

pub const PARSER_VERSION: u32 = 1;

pub fn compute_source_digest(file_bytes: &[u8], parser_version: u32) -> String {
    let mut h = Sha256::new();
    h.update(file_bytes);
    h.update(parser_version.to_le_bytes());
    format!("{:x}", h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_bytes_same_version_same_digest() {
        let bytes = b"hello world";
        assert_eq!(
            compute_source_digest(bytes, 1),
            compute_source_digest(bytes, 1),
        );
    }

    #[test]
    fn different_version_different_digest() {
        let bytes = b"hello world";
        assert_ne!(
            compute_source_digest(bytes, 1),
            compute_source_digest(bytes, 2),
            "parser version bump must change the digest",
        );
    }

    #[test]
    fn different_bytes_different_digest() {
        assert_ne!(
            compute_source_digest(b"a", 1),
            compute_source_digest(b"b", 1),
        );
    }

    #[test]
    fn digest_is_64_hex_chars() {
        let digest = compute_source_digest(b"x", 1);
        assert_eq!(digest.len(), 64);
        assert!(digest.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn golden_vector_hello_world_v1() {
        // Pins the exact algorithm: SHA-256 of b"hello world" concatenated
        // with 1u32.to_le_bytes(). Regenerate only if the algorithm itself
        // changes deliberately (which invalidates every stored digest).
        assert_eq!(
            compute_source_digest(b"hello world", 1),
            "08f06e2c00ab9ba6aed0980a436856b02feec3435bae117aeb8bdb2b35b444ba",
        );
    }
}
