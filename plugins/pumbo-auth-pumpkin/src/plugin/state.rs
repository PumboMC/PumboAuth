//! The plugin state, reachable from every handler.
//!
//! A handler takes what it needs from the host first, decides inside [`with`]
//! and calls the host only after the state is released: a host call can come
//! back into this plugin (an event, an `ipc` message), and such a nested call
//! finds the state busy ([`with`] returns `None`) instead of waiting for itself.

use std::cell::RefCell;
use std::collections::HashMap;

use pumbo_auth_core::flow::Auth;

pub struct Rt {
    pub dir: String,
    pub auth: Auth,
    /// Entity id -> UUID of online players (damage events only name the entity).
    pub entities: HashMap<i32, String>,
}

thread_local! {
    static RT: RefCell<Option<Rt>> = const { RefCell::new(None) };
    /// Countdown bars by UUID. Kept apart from the state: creating and showing
    /// them are host calls.
    static BARS: RefCell<HashMap<String, super::api::BossBar>> = RefCell::new(HashMap::new());
}

pub fn install(rt: Rt) {
    RT.with(|c| {
        if let Ok(mut g) = c.try_borrow_mut() {
            *g = Some(rt);
        }
    });
}

/// Drops the state (closes the database, so a reloaded plugin can open it).
pub fn uninstall() {
    RT.with(|c| {
        if let Ok(mut g) = c.try_borrow_mut() {
            *g = None;
        }
    });
}

/// Runs `f` on the state; `None` when it is not loaded or busy (a nested call).
pub fn with<R>(f: impl FnOnce(&mut Rt) -> R) -> Option<R> {
    RT.with(|c| c.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

pub fn with_bars<R>(f: impl FnOnce(&mut HashMap<String, super::api::BossBar>) -> R) -> Option<R> {
    BARS.with(|c| c.try_borrow_mut().ok().map(|mut g| f(&mut g)))
}
