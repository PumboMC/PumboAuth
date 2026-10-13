//! PumboAuth core: the account logic, without platform code.
//!
//! - [`flow`]: the login of every online player (premium, session, register,
//!   log in, two-factor), the player commands and the effects a platform layer
//!   carries out
//! - [`commands`]: `/pumboauth` and its subcommands
//! - [`rules`]: password and nickname rules
//! - [`hashing`]: argon2id for new passwords; bcrypt, AuthMe `$SHA$` and plain
//!   SHA-256/512 for imported ones, upgraded on login
//! - [`totp`]: two-factor login (RFC 6238), recovery codes, setup QR code on a map
//! - [`import`]: AuthMe CSV exports
//! - [`store`]: accounts, sessions, brute-force counters, registration limits
//! - [`config`]: the `auth`, `sessions`, `nickname`, `premium`,
//!   `two-factor`, `import` and `hashing` sections
//!
//! Time comes in as `now` arguments (Unix ms); nothing here talks to a host.

pub mod commands;
pub mod config;
pub mod flow;
pub mod hashing;
pub mod import;
pub mod rules;
pub mod store;
pub mod totp;

use pumbo_common::lang::Bundle;

/// Plugin id: permissions `pumbo.auth.<action>`, bundle name.
pub const ID: &str = "auth";

/// Player-facing messages of the account logic.
pub const LANG: Bundle =
    Bundle { name: "auth", files: &[("en", include_str!("../lang/en.yml")), ("pl", include_str!("../lang/pl.yml"))] };

#[cfg(test)]
mod tests {
    use pumbo_common::lang::{COMMON, Lang, check_bundle};

    use super::*;

    #[test]
    fn messages_are_complete_in_every_language() {
        assert_eq!(check_bundle(&LANG), Vec::<String>::new());
        let (pl, w) = Lang::load(&[COMMON, LANG], "pl", Some("prefix: \"[P] \"\n"));
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(pl.get("auth-login-success"), "Zalogowano.");
    }

    #[test]
    fn pumbo_nodes_cover_plugin_nodes() {
        let t = commands::tree();
        let mut nodes: Vec<String> = t.subs().iter().map(|s| t.permission(s)).collect();
        nodes.extend(["command", "player"].map(|a| pumbo_common::command::permission(ID, a)));
        let hosts: Vec<String> = nodes.iter().map(|n| n.replacen("pumbo.auth.", "pumboauth:", 1)).collect();
        for host in &hosts {
            assert!(pumbo_common::pumpkin::PUMBO_NODES.contains(&host.as_str()), "{host} missing from PUMBO_NODES");
        }
        for n in pumbo_common::pumpkin::PUMBO_NODES.iter().filter(|n| n.starts_with("pumboauth:")) {
            assert!(hosts.iter().any(|h| h == n), "{n} in PUMBO_NODES is no node of the plugin");
        }
    }
}
