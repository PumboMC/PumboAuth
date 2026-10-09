//! Password hashing. New passwords use argon2id (PHC string); imported hashes
//! from AuthMe-style plugins are verified and can be upgraded on login.

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};
use sha2::{Digest, Sha256, Sha512};

use pumbo_common::random::random_bytes;
use pumbo_common::util::{ct_eq, hex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashFormat {
    Argon2,
    Bcrypt,
    /// AuthMe `$SHA$salt$hash`: sha256(sha256(password) + salt).
    AuthMeSha,
    Sha256,
    Sha512,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verify {
    Ok { needs_rehash: bool },
    Wrong,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Argon2Settings {
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
}

impl Default for Argon2Settings {
    fn default() -> Self {
        Self { memory_kib: 19456, iterations: 2, parallelism: 1 }
    }
}

/// Longest password accepted by any verifier (bcrypt stops at 72 bytes).
pub const MAX_PASSWORD_BYTES: usize = 128;

pub fn detect(stored: &str) -> HashFormat {
    let s = stored.trim();
    if s.starts_with("$argon2") {
        HashFormat::Argon2
    } else if s.starts_with("$2a$") || s.starts_with("$2b$") || s.starts_with("$2y$") || s.starts_with("$2x$") {
        HashFormat::Bcrypt
    } else if s.starts_with("$SHA$") {
        HashFormat::AuthMeSha
    } else if s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit()) {
        HashFormat::Sha256
    } else if s.len() == 128 && s.chars().all(|c| c.is_ascii_hexdigit()) {
        HashFormat::Sha512
    } else {
        HashFormat::Unknown
    }
}

fn argon2(settings: Argon2Settings) -> Result<Argon2<'static>, String> {
    let params =
        Params::new(settings.memory_kib, settings.iterations, settings.parallelism, None).map_err(|e| e.to_string())?;
    Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
}

pub fn hash(password: &str, settings: Argon2Settings) -> Result<String, String> {
    if password.len() > MAX_PASSWORD_BYTES {
        return Err("password too long".into());
    }
    let mut salt_bytes = [0u8; 16];
    random_bytes(&mut salt_bytes);
    let salt = SaltString::encode_b64(&salt_bytes).map_err(|e| e.to_string())?;
    argon2(settings)?.hash_password(password.as_bytes(), &salt).map(|h| h.to_string()).map_err(|e| e.to_string())
}

pub fn verify(password: &str, stored: &str, settings: Argon2Settings) -> Verify {
    if password.len() > MAX_PASSWORD_BYTES {
        return Verify::Wrong;
    }
    let stored = stored.trim();
    match detect(stored) {
        HashFormat::Argon2 => {
            let Ok(parsed) = PasswordHash::new(stored) else {
                return Verify::Unsupported;
            };
            if Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok() {
                // Upgrade when the stored parameters differ from the configured ones.
                let current = format!("m={},t={},p={}", settings.memory_kib, settings.iterations, settings.parallelism);
                let needs_rehash = !stored.contains(&current) || !stored.starts_with("$argon2id$");
                Verify::Ok { needs_rehash }
            } else {
                Verify::Wrong
            }
        }
        HashFormat::Bcrypt => {
            // bcrypt only looks at the first 72 bytes.
            let pw = password.as_bytes();
            let pw = pw.get(..pw.len().min(72)).unwrap_or(pw);
            match bcrypt::verify(pw, stored) {
                Ok(true) => Verify::Ok { needs_rehash: true },
                Ok(false) => Verify::Wrong,
                Err(_) => Verify::Unsupported,
            }
        }
        HashFormat::AuthMeSha => {
            let mut parts = stored.split('$').filter(|p| !p.is_empty());
            let (Some(_), Some(salt), Some(expected)) = (parts.next(), parts.next(), parts.next()) else {
                return Verify::Unsupported;
            };
            let inner = hex(&Sha256::digest(password.as_bytes()));
            let outer = hex(&Sha256::digest(format!("{inner}{salt}").as_bytes()));
            ok_if(ct_eq(outer.as_bytes(), expected.to_ascii_lowercase().as_bytes()))
        }
        HashFormat::Sha256 => {
            let h = hex(&Sha256::digest(password.as_bytes()));
            ok_if(ct_eq(h.as_bytes(), stored.to_ascii_lowercase().as_bytes()))
        }
        HashFormat::Sha512 => {
            let h = hex(&Sha512::digest(password.as_bytes()));
            ok_if(ct_eq(h.as_bytes(), stored.to_ascii_lowercase().as_bytes()))
        }
        HashFormat::Unknown => Verify::Unsupported,
    }
}

fn ok_if(ok: bool) -> Verify {
    if ok { Verify::Ok { needs_rehash: true } } else { Verify::Wrong }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAST: Argon2Settings = Argon2Settings { memory_kib: 1024, iterations: 1, parallelism: 1 };

    #[test]
    fn argon2_roundtrip() {
        let h = hash("hunter22", FAST).unwrap();
        assert!(h.starts_with("$argon2id$"));
        assert_eq!(detect(&h), HashFormat::Argon2);
        assert_eq!(verify("hunter22", &h, FAST), Verify::Ok { needs_rehash: false });
        assert_eq!(verify("hunter23", &h, FAST), Verify::Wrong);
        // other configured parameters ask for a rehash
        assert_eq!(verify("hunter22", &h, Argon2Settings::default()), Verify::Ok { needs_rehash: true });
    }

    #[test]
    fn bcrypt_from_htpasswd() {
        // generated with `htpasswd -nbB -C 5 u hunter22`
        let h = "$2y$05$oac08OAD6QQQAftaNoKgLeRMW9f5V/AMQ60X77jtjS9NvH5k9qgIa";
        assert_eq!(detect(h), HashFormat::Bcrypt);
        assert_eq!(verify("hunter22", h, FAST), Verify::Ok { needs_rehash: true });
        assert_eq!(verify("hunter2", h, FAST), Verify::Wrong);
    }

    #[test]
    fn authme_sha() {
        // sha256(sha256("hunter22") + salt), computed with Python hashlib
        let h = "$SHA$a1b2c3d4e5f60718$7eea1a3165d1bf71483f6a0fe371becbdfbab9e6d70ca0480b4ff5baadbcf881";
        assert_eq!(detect(h), HashFormat::AuthMeSha);
        assert_eq!(verify("hunter22", h, FAST), Verify::Ok { needs_rehash: true });
        assert_eq!(verify("Hunter22", h, FAST), Verify::Wrong);
    }

    #[test]
    fn plain_sha() {
        let s256 = "20d2fe5e369db54ec7090639a9dc30ec4d608604936239d39e2de07fda09eb0b";
        let s512 = "e633ac1a9852f4a28539d8e6525c79d426cc78b96f48176a46622125504c5e87a9f27ac3d91620483883f6202856d8ac932ce764cce5952a526d399e43f32cf6";
        assert_eq!(detect(s256), HashFormat::Sha256);
        assert_eq!(detect(s512), HashFormat::Sha512);
        assert_eq!(verify("hunter22", s256, FAST), Verify::Ok { needs_rehash: true });
        assert_eq!(verify("hunter22", &s512.to_uppercase(), FAST), Verify::Ok { needs_rehash: true });
        assert_eq!(verify("x", s512, FAST), Verify::Wrong);
    }

    #[test]
    fn unknown_and_long() {
        assert_eq!(verify("a", "plaintext", FAST), Verify::Unsupported);
        assert_eq!(verify("a", "$SHA$broken", FAST), Verify::Unsupported);
        let long = "x".repeat(MAX_PASSWORD_BYTES + 1);
        assert!(hash(&long, FAST).is_err());
    }
}
