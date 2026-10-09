//! Configuration sections of the account logic: `auth`, `sessions`,
//! `nickname`, `premium`, `two-factor`, `import` and `hashing`.
//!
//! The platform layer puts these sections into its own config struct and calls
//! each section's `validate` from its `Settings::validate`, passing the section
//! name used in its file (warnings name options as `<section>.<option>`).

use pumbo_common::config::Check;
use serde::{Deserialize, Serialize};

use crate::hashing::Argon2Settings;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct AuthCfg {
    pub enabled: bool,
    pub timeout_seconds: u32,
    pub bossbar: bool,
    pub login_attempts: u32,
    pub min_password_length: u32,
    pub max_password_length: u32,
    pub check_password_strength: bool,
    pub register_repeat_password: bool,
    pub disable_registrations: bool,
    pub ip_bruteforce_max: u32,
    pub ip_bruteforce_hours: u32,
    pub account_bruteforce_max: u32,
    pub account_bruteforce_hours: u32,
    pub ip_registrations_max: u32,
    pub ip_registrations_hours: u32,
    pub command_cooldown_ms: u32,
    pub confirm_keyword: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct SessionsCfg {
    pub enabled: bool,
    pub ttl_minutes: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct NicknameCfg {
    pub enabled: bool,
    pub min_length: u32,
    pub max_length: u32,
    pub allowed_chars: String,
    pub blocked: Vec<String>,
    pub case_protection: bool,
    pub premium_protection: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct PremiumCfg {
    pub enabled: bool,
    /// Trust a version 4 UUID with a signed skin (a server in online mode,
    /// authenticated proxy forwarding). Off behind BungeeCord forwarding without
    /// BungeeGuard, where a client can send any profile.
    pub forwarded_profiles: bool,
    /// Store an account for a premium player at their first join.
    pub save_accounts: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct TwoFactorCfg {
    pub enabled: bool,
    pub issuer: String,
    pub recovery_codes: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct ImportCfg {
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct HashingCfg {
    pub argon2_memory_kib: u32,
    pub argon2_iterations: u32,
    pub argon2_parallelism: u32,
    pub rehash_legacy: bool,
}

impl Default for AuthCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            timeout_seconds: 60,
            bossbar: true,
            login_attempts: 3,
            min_password_length: 4,
            max_password_length: 71,
            check_password_strength: true,
            register_repeat_password: true,
            disable_registrations: false,
            ip_bruteforce_max: 10,
            ip_bruteforce_hours: 8,
            account_bruteforce_max: 20,
            account_bruteforce_hours: 8,
            ip_registrations_max: 3,
            ip_registrations_hours: 6,
            command_cooldown_ms: 750,
            confirm_keyword: "confirm".into(),
        }
    }
}

impl Default for SessionsCfg {
    fn default() -> Self {
        Self { enabled: true, ttl_minutes: 60 }
    }
}

impl Default for NicknameCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            min_length: 3,
            max_length: 16,
            allowed_chars: "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_".into(),
            blocked: Vec::new(),
            case_protection: true,
            premium_protection: true,
        }
    }
}

impl Default for PremiumCfg {
    fn default() -> Self {
        Self { enabled: true, forwarded_profiles: true, save_accounts: true }
    }
}

impl Default for TwoFactorCfg {
    fn default() -> Self {
        Self { enabled: true, issuer: "Minecraft".into(), recovery_codes: 8 }
    }
}

impl Default for ImportCfg {
    fn default() -> Self {
        Self { enabled: true }
    }
}

impl Default for HashingCfg {
    fn default() -> Self {
        Self { argon2_memory_kib: 19456, argon2_iterations: 2, argon2_parallelism: 1, rehash_legacy: true }
    }
}

/// Every section of the account logic, as a platform layer puts them in its file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct AuthSettings {
    pub auth: AuthCfg,
    pub sessions: SessionsCfg,
    pub nickname: NicknameCfg,
    pub premium: PremiumCfg,
    pub two_factor: TwoFactorCfg,
    pub import: ImportCfg,
    pub hashing: HashingCfg,
}

impl AuthSettings {
    /// Validates every section under its usual name.
    pub fn validate(&mut self, c: &mut Check<'_>) {
        self.auth.validate(c, "auth");
        self.nickname.validate(c, "nickname");
        self.two_factor.validate(c, "two-factor");
        self.hashing.validate(c, "hashing");
    }
}

fn opt(section: &str, name: &str) -> String {
    if section.is_empty() { name.to_string() } else { format!("{section}.{name}") }
}

impl AuthCfg {
    pub fn validate(&mut self, c: &mut Check<'_>, section: &str) {
        let d = Self::default();
        c.clamp(&opt(section, "timeout-seconds"), &mut self.timeout_seconds, 5, 3600);
        c.clamp(&opt(section, "login-attempts"), &mut self.login_attempts, 1, 100);
        c.clamp(&opt(section, "min-password-length"), &mut self.min_password_length, 1, 128);
        let min = self.min_password_length;
        c.clamp(&opt(section, "max-password-length"), &mut self.max_password_length, min, 128);
        c.ensure(&opt(section, "confirm-keyword"), &mut self.confirm_keyword, d.confirm_keyword, |k| {
            !k.trim().is_empty() && !k.contains(' ')
        });
    }

    /// Window of the per-address failure counter.
    pub fn ip_window_ms(&self) -> u64 {
        u64::from(self.ip_bruteforce_hours) * 3_600_000
    }

    /// Window of the per-account failure counter.
    pub fn account_window_ms(&self) -> u64 {
        u64::from(self.account_bruteforce_hours) * 3_600_000
    }

    /// Window of the registrations-per-address limit.
    pub fn registration_window_ms(&self) -> u64 {
        u64::from(self.ip_registrations_hours) * 3_600_000
    }
}

impl SessionsCfg {
    pub fn ttl_ms(&self) -> u64 {
        u64::from(self.ttl_minutes) * 60_000
    }
}

impl NicknameCfg {
    pub fn validate(&mut self, c: &mut Check<'_>, section: &str) {
        let d = Self::default();
        c.clamp(&opt(section, "min-length"), &mut self.min_length, 1, 64);
        let min = self.min_length;
        c.clamp(&opt(section, "max-length"), &mut self.max_length, min, 64);
        c.ensure(&opt(section, "allowed-chars"), &mut self.allowed_chars, d.allowed_chars, |s| !s.is_empty());
    }
}

impl TwoFactorCfg {
    pub fn validate(&mut self, c: &mut Check<'_>, section: &str) {
        let d = Self::default();
        c.clamp(&opt(section, "recovery-codes"), &mut self.recovery_codes, 0, 32);
        c.ensure(&opt(section, "issuer"), &mut self.issuer, d.issuer, |s| !s.trim().is_empty());
    }
}

impl HashingCfg {
    pub fn validate(&mut self, c: &mut Check<'_>, section: &str) {
        c.clamp(&opt(section, "argon2-memory-kib"), &mut self.argon2_memory_kib, 1024, 262_144);
        c.clamp(&opt(section, "argon2-iterations"), &mut self.argon2_iterations, 1, 16);
        c.clamp(&opt(section, "argon2-parallelism"), &mut self.argon2_parallelism, 1, 8);
    }

    pub fn argon2(&self) -> Argon2Settings {
        Argon2Settings {
            memory_kib: self.argon2_memory_kib,
            iterations: self.argon2_iterations,
            parallelism: self.argon2_parallelism,
        }
    }
}

#[cfg(test)]
mod tests {
    use pumbo_common::config::{self, Settings, Warning};

    use super::*;

    type Cfg = AuthSettings;

    impl Settings for AuthSettings {
        fn validate(&mut self, c: &mut Check<'_>) {
            AuthSettings::validate(self, c);
        }
    }

    fn names(w: &[Warning]) -> Vec<String> {
        let mut n: Vec<String> = w.iter().filter_map(|w| w.option.clone()).collect();
        n.sort();
        n
    }

    #[test]
    fn defaults_are_valid() {
        let (c, w) = config::load::<Cfg>("");
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(c, Cfg::default());
        assert_eq!(c.hashing.argon2(), Argon2Settings::default());
    }

    #[test]
    fn invalid_values_fall_back_per_option() {
        let text = "auth:\n  timeout-seconds: -5\n  login-attempts: 500\n  min-password-length: 10\n  max-password-length: 5\n  confirm-keyword: a b\n  bogus: 1\nnickname:\n  allowed-chars: \"\"\npremium:\n  forwarded-profiles: \"yes\"\ntwo-factor:\n  issuer: \" \"\nhashing:\n  argon2-memory-kib: 1\n";
        let (c, w) = config::load::<Cfg>(text);
        assert_eq!(c.auth.timeout_seconds, 60);
        assert_eq!(c.auth.login_attempts, 100);
        assert_eq!((c.auth.min_password_length, c.auth.max_password_length), (10, 10));
        assert_eq!(c.auth.confirm_keyword, "confirm");
        assert!(!c.nickname.allowed_chars.is_empty());
        assert!(c.premium.forwarded_profiles);
        assert_eq!(c.two_factor.issuer, "Minecraft");
        assert_eq!(c.hashing.argon2_memory_kib, 1024);
        assert_eq!(
            names(&w),
            vec![
                "auth.bogus",
                "auth.confirm-keyword",
                "auth.login-attempts",
                "auth.max-password-length",
                "auth.timeout-seconds",
                "hashing.argon2-memory-kib",
                "nickname.allowed-chars",
                "premium.forwarded-profiles",
                "two-factor.issuer",
            ]
        );
    }

    #[test]
    fn windows() {
        let a = AuthCfg::default();
        assert_eq!(a.ip_window_ms(), 8 * 3_600_000);
        assert_eq!(a.account_window_ms(), 8 * 3_600_000);
        assert_eq!(a.registration_window_ms(), 6 * 3_600_000);
        assert_eq!(SessionsCfg::default().ttl_ms(), 3_600_000);
    }
}
