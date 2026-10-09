//! Passwords and session tokens.
//!
//! Passwords: Argon2id with a random 16-byte salt, stored as `argon2id$<salt hex>$<hash hex>`.
//! Tokens: 32 random bytes, given to the client in hex; only their SHA-256 is stored, so a
//! copy of the database does not let anyone sign in.

use argon2::Argon2;
use sha2::{Digest, Sha256};

const PREFIX: &str = "argon2id";

fn random<const N: usize>() -> [u8; N] {
    let mut buf = [0u8; N];
    getrandom::fill(&mut buf).expect("the operating system provides random numbers");
    buf
}

fn derive(password: &str, salt: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    Argon2::default().hash_password_into(password.as_bytes(), salt, &mut out).expect("valid Argon2 parameters");
    out
}

pub fn hash_password(password: &str) -> String {
    let salt: [u8; 16] = random();
    format!("{PREFIX}${}${}", hex::encode(salt), hex::encode(derive(password, &salt)))
}

pub fn verify_password(password: &str, stored: &str) -> bool {
    let mut parts = stored.split('$');
    let (Some(PREFIX), Some(salt), Some(hash), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else { return false };
    let (Ok(salt), Ok(hash)) = (hex::decode(salt), hex::decode(hash)) else { return false };
    let actual = derive(password, &salt);
    // Constant time, so timing does not reveal how much of the hash matched.
    hash.len() == actual.len() && hash.iter().zip(actual).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

/// A new session token (for the client) and its hash (for storage).
pub fn new_token() -> (String, String) {
    let token = hex::encode(random::<32>());
    let hash = token_hash(&token);
    (token, hash)
}

pub fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords_verify_only_themselves() {
        let stored = hash_password("correct horse");
        assert!(verify_password("correct horse", &stored));
        assert!(!verify_password("correct horsE", &stored));
        assert!(!verify_password("correct horse", "garbage"));
        assert_ne!(hash_password("correct horse"), stored, "salted");
    }

    #[test]
    fn tokens_are_random_and_hashed() {
        let (a, ha) = new_token();
        let (b, _) = new_token();
        assert_ne!(a, b);
        assert_eq!(a.len(), 64);
        assert_eq!(token_hash(&a), ha);
        assert_ne!(a, ha);
    }
}
