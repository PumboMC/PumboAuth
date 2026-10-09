//! Password and nickname rules.

use pumbo_common::util::glob;

use crate::config::{AuthCfg, NicknameCfg};

/// The most common leaked passwords (lowercase). Rejected when strength checks are on.
const COMMON: &[&str] = &[
    "123456",
    "password",
    "12345678",
    "qwerty",
    "123456789",
    "12345",
    "1234",
    "111111",
    "1234567",
    "dragon",
    "123123",
    "baseball",
    "abc123",
    "football",
    "monkey",
    "letmein",
    "696969",
    "shadow",
    "master",
    "666666",
    "qwertyuiop",
    "123321",
    "mustang",
    "1234567890",
    "michael",
    "654321",
    "superman",
    "1qaz2wsx",
    "7777777",
    "121212",
    "000000",
    "qazwsx",
    "123qwe",
    "killer",
    "trustno1",
    "jordan",
    "jennifer",
    "zxcvbnm",
    "asdfgh",
    "hunter",
    "buster",
    "soccer",
    "harley",
    "batman",
    "andrew",
    "tigger",
    "sunshine",
    "iloveyou",
    "2000",
    "charlie",
    "robert",
    "thomas",
    "hockey",
    "ranger",
    "daniel",
    "starwars",
    "klaster",
    "112233",
    "george",
    "computer",
    "michelle",
    "jessica",
    "pepper",
    "1111",
    "zxcvbn",
    "555555",
    "11111111",
    "131313",
    "freedom",
    "777777",
    "pass",
    "maggie",
    "159753",
    "aaaaaa",
    "ginger",
    "princess",
    "joshua",
    "cheese",
    "amanda",
    "summer",
    "love",
    "ashley",
    "nicole",
    "chelsea",
    "biteme",
    "matthew",
    "access",
    "yankees",
    "987654321",
    "dallas",
    "austin",
    "thunder",
    "taylor",
    "matrix",
    "minecraft",
    "admin",
    "admin123",
    "root",
    "toor",
    "test",
    "test123",
    "guest",
    "welcome",
    "login",
    "passw0rd",
    "password1",
    "password123",
    "qwerty123",
    "1q2w3e4r",
    "1q2w3e",
    "q1w2e3r4",
    "asdf",
    "asdfasdf",
    "asdfghjkl",
    "zaq12wsx",
    "!@#$%^&*",
    "aa123456",
    "abcd1234",
    "abcdef",
    "abc",
    "0000",
    "00000000",
    "88888888",
    "999999",
    "123654",
    "147258369",
    "qwe123",
    "polska",
    "haslo",
    "haslo123",
    "kochamcie",
    "zaq1@wsx",
    "misiek",
    "marcin",
    "lol123",
    "creeper",
    "steve",
    "herobrine",
    "diamond",
    "notch",
    "skyblock",
    "survival",
    "pvp",
    "server",
    "mojang",
    "microsoft",
    "google",
    "apple",
    "samsung",
    "secret",
    "changeme",
    "default",
    "hello",
    "hello123",
    "qwer",
    "qwer1234",
    "1234qwer",
    "11111",
    "22222",
    "123",
    "12",
    "1",
    "a",
    "aaa",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordProblem {
    TooShort(u32),
    TooLong(u32),
    Common,
    SameAsName,
}

pub fn check_password(password: &str, nickname: &str, cfg: &AuthCfg) -> Result<(), PasswordProblem> {
    let len = u32::try_from(password.chars().count()).unwrap_or(u32::MAX);
    if len < cfg.min_password_length {
        return Err(PasswordProblem::TooShort(cfg.min_password_length));
    }
    if len > cfg.max_password_length || password.len() > crate::hashing::MAX_PASSWORD_BYTES {
        return Err(PasswordProblem::TooLong(cfg.max_password_length));
    }
    if cfg.check_password_strength {
        let lower = password.to_lowercase();
        if lower == nickname.to_lowercase() {
            return Err(PasswordProblem::SameAsName);
        }
        if COMMON.contains(&lower.as_str()) {
            return Err(PasswordProblem::Common);
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NickProblem {
    Invalid,
    Blocked,
}

/// Validates a nickname. Bedrock prefixes (Floodgate) are stripped by the caller.
pub fn check_nickname(name: &str, cfg: &NicknameCfg) -> Result<(), NickProblem> {
    let len = u32::try_from(name.chars().count()).unwrap_or(u32::MAX);
    if len < cfg.min_length || len > cfg.max_length || !name.chars().all(|c| cfg.allowed_chars.contains(c)) {
        return Err(NickProblem::Invalid);
    }
    let lower = name.to_lowercase();
    if cfg.blocked.iter().any(|pattern| glob(&pattern.to_lowercase(), &lower)) {
        return Err(NickProblem::Blocked);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords() {
        let cfg = AuthCfg::default();
        assert_eq!(check_password("abc", "Steve", &cfg), Err(PasswordProblem::TooShort(4)));
        assert_eq!(check_password("password", "Steve", &cfg), Err(PasswordProblem::Common));
        assert_eq!(check_password("StEvE", "steve", &cfg), Err(PasswordProblem::SameAsName));
        assert_eq!(check_password(&"x".repeat(72), "a", &cfg), Err(PasswordProblem::TooLong(71)));
        assert_eq!(check_password("correct horse", "Steve", &cfg), Ok(()));
        let lax = AuthCfg { check_password_strength: false, ..AuthCfg::default() };
        assert_eq!(check_password("password", "Steve", &lax), Ok(()));
    }

    #[test]
    fn nicknames() {
        let mut cfg = NicknameCfg::default();
        assert_eq!(check_nickname("Steve_01", &cfg), Ok(()));
        assert_eq!(check_nickname("ab", &cfg), Err(NickProblem::Invalid));
        assert_eq!(check_nickname("bad-name", &cfg), Err(NickProblem::Invalid));
        assert_eq!(check_nickname("ThisNameIsWayTooLong", &cfg), Err(NickProblem::Invalid));
        cfg.blocked = vec!["admin*".into(), "*staff*".into()];
        assert_eq!(check_nickname("AdminBob", &cfg), Err(NickProblem::Blocked));
        assert_eq!(check_nickname("TheStaffGuy", &cfg), Err(NickProblem::Blocked));
        assert_eq!(check_nickname("Bob", &cfg), Ok(()));
    }
}
