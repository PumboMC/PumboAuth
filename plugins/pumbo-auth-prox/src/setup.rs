//! Host-independent parts of the PumboProx layer: config and message files,
//! where the virtual world puts a player.

use pumbo_auth_core::config::{
    AuthCfg, AuthSettings, HashingCfg, ImportCfg, NicknameCfg, PremiumCfg, SessionsCfg, TwoFactorCfg,
};
use pumbo_common::config::{self, Check, Settings, Warning};
use pumbo_common::lang::{Bundle, COMMON, Lang};
use serde::{Deserialize, Serialize};

/// Plugin id in the manifest (`pumbo-auth.yml`); the `pumbo:player-*` topics belong to it.
pub const PLUGIN_ID: &str = "pumbo-auth";

/// The config file with comments, for the admin to copy into `plugins/pumbo-auth/`.
pub const CONFIG_TEMPLATE: &str = include_str!("../assets/config.yml");

/// Messages of the PumboProx layer (prefix).
pub const LANG: Bundle = Bundle {
    name: "auth-prox",
    files: &[("en", include_str!("../lang/en.yml")), ("pl", include_str!("../lang/pl.yml"))],
};

/// Every message bundle, in load order.
pub const BUNDLES: [Bundle; 3] = [COMMON, pumbo_auth_core::LANG, LANG];

/// Where a player waits for the login: on an invisible platform in the void.
pub const SPAWN: (f64, f64, f64) = (0.5, 64.0, 0.5);

/// Blocks of the platform under [`SPAWN`] (barriers: nothing to see).
pub fn platform() -> impl Iterator<Item = (i32, i32, i32)> {
    (-1..=1).flat_map(|x| (-1..=1).map(move |z| (x, 63, z)))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Config {
    pub language: String,
    pub per_player_language: bool,
    pub auth: AuthCfg,
    pub sessions: SessionsCfg,
    pub nickname: NicknameCfg,
    pub premium: PremiumCfg,
    pub two_factor: TwoFactorCfg,
    pub import: ImportCfg,
    pub hashing: HashingCfg,
}

impl Default for Config {
    fn default() -> Self {
        let s = AuthSettings::default();
        Self {
            language: "en".into(),
            per_player_language: true,
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
}

impl Settings for Config {
    fn validate(&mut self, check: &mut Check<'_>) {
        self.language = self.language.trim().to_lowercase();
        let ok = !self.language.is_empty()
            && self.language.len() <= 16
            && self.language.bytes().all(|b| b.is_ascii_lowercase() || b == b'-' || b == b'_');
        check.ensure("language", &mut self.language, "en".into(), |_| ok);
        let mut s = self.settings();
        s.validate(check);
        (self.auth, self.sessions, self.nickname, self.premium, self.two_factor, self.import, self.hashing) =
            (s.auth, s.sessions, s.nickname, s.premium, s.two_factor, s.import, s.hashing);
    }
}

/// Reads `config.yml` from the read-only config folder (`/config` in the
/// plugin); a missing file means the defaults.
pub fn load_config(dir: &str) -> (Config, Vec<Warning>, bool) {
    let (cfg, mut w, found) = match std::fs::read_to_string(format!("{dir}/config.yml")) {
        Ok(text) => {
            let (cfg, w) = config::load::<Config>(&text);
            (cfg, w, true)
        }
        Err(_) => (Config::default(), Vec::new(), false),
    };
    w.extend(config::old_files(dir));
    (cfg, w, found)
}

/// The configured language and, with `per-player-language`, every other one
/// with bundled messages or a file in `lang/` (overrides of the bundled ones).
pub fn load_langs(dir: &str, cfg: &Config) -> (Lang, Vec<Lang>, Vec<Warning>) {
    let lang_dir = format!("{dir}/lang");
    let mut codes: Vec<String> = Lang::builtin_codes(&BUNDLES).iter().map(|c| (*c).to_string()).collect();
    if let Ok(entries) = std::fs::read_dir(&lang_dir) {
        for e in entries.flatten() {
            if let Some(code) = e.file_name().to_string_lossy().strip_suffix(".yml") {
                codes.push(code.to_string());
            }
        }
    }
    codes.retain(|c| *c != cfg.language);
    codes.sort();
    codes.dedup();
    let mut warnings = Vec::new();
    let mut load = |code: &str| {
        let user = std::fs::read_to_string(format!("{lang_dir}/{code}.yml")).ok();
        let (lang, w) = Lang::load(&BUNDLES, code, user.as_deref());
        warnings.extend(w.into_iter().map(|w| Warning { message: format!("lang/{code}.yml: {}", w.message), ..w }));
        lang
    };
    let default = load(&cfg.language);
    let others = if cfg.per_player_language { codes.iter().map(|c| load(c)).collect() } else { Vec::new() };
    (default, others, warnings)
}

#[cfg(test)]
mod tests {
    use pumbo_common::lang::check_bundle;

    use super::*;

    #[test]
    fn built_in_language_files_are_current() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/lang");
        let written = Lang::write_templates(&BUNDLES, &dir);
        assert!(written.is_empty(), "rewritten from the message bundles, commit them: {written:?}");
    }

    #[test]
    fn template_is_the_default() {
        let (cfg, w) = config::load::<Config>(CONFIG_TEMPLATE);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn messages_and_languages() {
        assert_eq!(check_bundle(&LANG), Vec::<String>::new());
        let dir = std::env::temp_dir().join(format!("pumbo-auth-prox-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let d = dir.to_string_lossy().to_string();
        let (cfg, w, found) = load_config(&d);
        assert!(w.is_empty() && !found && cfg == Config::default());
        let (default, others, w) = load_langs(&d, &cfg);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!((default.code(), others.len()), ("en", 1));
        assert!(default.get("prefix").contains("PumboAuth"));
        let _ = std::fs::remove_dir(&dir);
    }
}
