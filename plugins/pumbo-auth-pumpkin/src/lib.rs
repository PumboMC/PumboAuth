//! PumboAuth for Pumpkin: accounts, login, premium, two-factor login, sessions.
//!
//! The rules live in `pumbo-auth-core`; this crate connects them to the Pumpkin
//! plugin API (events, commands, the countdown bar, the hand-off from
//! PumboFilter over `ipc`). [`config`] is plain Rust and tested natively; the
//! plugin itself ([`plugin`]) only exists in the WebAssembly build.

pub mod config;

use pumbo_common::lang::Bundle;

/// Messages of the Pumpkin layer (the prefix).
pub const LANG: Bundle = Bundle {
    name: "pumboauth",
    files: &[("en", include_str!("../lang/en.yml")), ("pl", include_str!("../lang/pl.yml"))],
};

/// Every message bundle, in load order.
pub const BUNDLES: [Bundle; 3] = [pumbo_common::lang::COMMON, pumbo_auth_core::LANG, LANG];

/// Name of the plugin on Pumpkin: data folder, permission namespace, `ipc` name.
pub const PLUGIN_NAME: &str = "pumboauth";

/// The other gate plugin on Pumpkin.
pub const PARTNER: &str = "pumbofilter";

/// `pumbo.auth.<action>` as a Pumpkin node (`pumboauth:<action>`).
pub fn pumpkin_node(node: &str) -> String {
    let action = node.strip_prefix("pumbo.auth.").unwrap_or(node);
    format!("{PLUGIN_NAME}:{action}")
}

#[cfg(all(target_arch = "wasm32", feature = "mc263", feature = "mc262"))]
compile_error!("enable only one of the features `mc263` and `mc262`");
#[cfg(all(target_arch = "wasm32", not(any(feature = "mc263", feature = "mc262"))))]
compile_error!("enable one of the features `mc263` (Pumpkin 0.2.0) or `mc262` (Pumpkin 0.1.0-dev)");

#[cfg(all(target_arch = "wasm32", feature = "mc262", not(feature = "mc263")))]
extern crate api262 as papi;
#[cfg(all(target_arch = "wasm32", feature = "mc263", not(feature = "mc262")))]
extern crate api263 as papi;

#[cfg(target_arch = "wasm32")]
pub mod plugin;

#[cfg(test)]
mod tests {
    use pumbo_common::lang::{Lang, check_bundle};

    use super::*;

    #[test]
    fn messages_are_complete_in_every_language() {
        assert_eq!(check_bundle(&LANG), Vec::<String>::new());
        for code in ["en", "pl"] {
            let (l, w) = Lang::load(&BUNDLES, code, None);
            assert!(w.is_empty(), "{w:?}");
            assert!(l.get("prefix").contains("PumboAuth"));
        }
    }

    #[test]
    fn message_template_loads_without_warnings() {
        for code in ["en", "pl"] {
            let t = Lang::template(&BUNDLES, code);
            let (_, w) = Lang::load(&BUNDLES, code, Some(&t));
            assert!(w.is_empty(), "{w:?}");
        }
    }

    #[test]
    fn nodes() {
        assert_eq!(pumpkin_node("pumbo.auth.forceregister"), "pumboauth:forceregister");
    }
}
