//! `config.yml`: the language, `commands` and the core sections.

use pumbo_auth_core::config::{
    AuthCfg, AuthSettings, HashingCfg, ImportCfg, NicknameCfg, PremiumCfg, SessionsCfg, TwoFactorCfg,
};
use pumbo_auth_core::flow::player_command;
use pumbo_common::config::{Check, Settings};
use serde::{Deserialize, Serialize};

use crate::PLUGIN_NAME;

/// The file written at the first start.
pub const TEMPLATE: &str = include_str!("../assets/config.yml");

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Config {
    pub language: String,
    pub commands: CommandsCfg,
    pub auth: AuthCfg,
    pub sessions: SessionsCfg,
    pub nickname: NicknameCfg,
    pub premium: PremiumCfg,
    pub two_factor: TwoFactorCfg,
    pub import: ImportCfg,
    pub hashing: HashingCfg,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct CommandsCfg {
    /// Other names of `/pumboauth`.
    pub admin_aliases: Vec<String>,
}

impl Default for CommandsCfg {
    fn default() -> Self {
        Self { admin_aliases: vec![pumbo_auth_core::commands::ALIAS.into(), "auth".into()] }
    }
}

impl Default for Config {
    fn default() -> Self {
        let s = AuthSettings::default();
        Self {
            language: "en".into(),
            commands: CommandsCfg::default(),
            auth: s.auth,
            sessions: s.sessions,
            nickname: s.nickname,
            premium: s.premium,
            two_factor: s.two_factor,
            import: s.import,
            hashing: s.hashing,
        }
    }
}

impl Config {
    pub fn settings(&self) -> AuthSettings {
        AuthSettings {
            auth: self.auth.clone(),
            sessions: self.sessions.clone(),
            nickname: self.nickname.clone(),
            premium: self.premium.clone(),
            two_factor: self.two_factor.clone(),
            import: self.import.clone(),
            hashing: self.hashing.clone(),
        }
    }

    /// `pumboauth` and the valid aliases.
    pub fn admin_names(&self) -> Vec<String> {
        let mut names = vec![PLUGIN_NAME.to_string()];
        names.extend(self.commands.admin_aliases.iter().cloned());
        names
    }
}

fn valid_alias(a: &str) -> bool {
    !a.is_empty()
        && a.len() <= 32
        && a.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        && a != PLUGIN_NAME
        && player_command(a).is_none()
}

impl Settings for Config {
    fn validate(&mut self, check: &mut Check<'_>) {
        let lang = self.language.trim().to_lowercase();
        check.ensure("language", &mut self.language, "en".into(), |_| {
            !lang.is_empty()
                && lang.len() <= 16
                && lang.bytes().all(|b| b.is_ascii_lowercase() || b == b'-' || b == b'_')
        });
        self.language = self.language.trim().to_lowercase();
        let aliases: Vec<String> = self.commands.admin_aliases.iter().map(|a| a.trim().to_lowercase()).collect();
        if let Some(bad) = aliases.iter().find(|a| !valid_alias(a)) {
            check.invalid("commands.admin-aliases", bad, &"[\"pa\", \"auth\"]");
            self.commands = CommandsCfg::default();
        } else {
            self.commands.admin_aliases = aliases;
        }
        let mut s = self.settings();
        s.validate(check);
        (self.auth, self.sessions, self.nickname, self.premium, self.two_factor, self.import, self.hashing) =
            (s.auth, s.sessions, s.nickname, s.premium, s.two_factor, s.import, s.hashing);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pumbo_common::config;

    #[test]
    fn template_gives_exactly_the_defaults() {
        let (c, w) = config::load::<Config>(TEMPLATE);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(c, Config::default());
        assert_eq!(c.admin_names(), vec!["pumboauth", "pa", "auth"]);
    }

    #[test]
    fn bad_values_fall_back() {
        let (c, w) = config::load::<Config>("commands:\n  admin-aliases: [login]\nauth:\n  login-attempts: 0\n");
        assert_eq!(c.commands, CommandsCfg::default());
        assert_eq!(c.auth.login_attempts, 1);
        assert_eq!(w.len(), 2, "{w:?}");
        let (c, w) = config::load::<Config>("commands:\n  admin-aliases: []\n");
        assert!(w.is_empty());
        assert_eq!(c.admin_names(), vec!["pumboauth"]);
    }
}
