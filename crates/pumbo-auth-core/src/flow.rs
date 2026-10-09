//! The login of every online player: who logs in automatically (premium,
//! session), who has to register or log in, the player commands, and the
//! effects a platform layer carries out.
//!
//! A player who has to log in is held (dark screen on the client only, hidden,
//! protected, actions cancelled) and stands at their own place: a player who
//! walks farther than [`ANCHOR_RADIUS`] from where the prompt found them is put
//! back. When PumboFilter checks the player first, PumboAuth waits for it
//! ([`Auth::filter_state`]) and leaves the movement alone meanwhile (the
//! gravity check needs it).
//!
//! Every call takes the time (`now` in Unix ms) and returns [`Effect`]s.

use std::collections::HashMap;
use std::time::Duration;

use pumbo_common::gate::Hold;
use pumbo_common::id::{Uuid, name_key};
use pumbo_common::lang::Lang;
use pumbo_common::rich::{Click, Line, Segment, Text};
use pumbo_common::store::{Store, StoreError};
use pumbo_common::style;
use pumbo_common::text::Args;

use crate::config::AuthSettings;
use crate::hashing::{self, Verify};
use crate::rules::{NickProblem, PasswordProblem, check_nickname, check_password};
use crate::store::{Account, AuthStore, Counter};
use crate::totp;

/// `unknown` from PumboFilter for longer than this counts as "free" (it never
/// saw the player join, for example because it was loaded later).
const UNKNOWN_FILTER_MS: u64 = 3000;
const PENDING_TOTP_MS: u64 = 5 * 60_000;
const PENDING_PREMIUM_MS: u64 = 30_000;
const BOSSBAR_EVERY_MS: u64 = 1000;
/// How far a player at the prompt may move before being put back. Not
/// Pumpkin's movement lock: that answers every movement with a teleport, and
/// the client answers every teleport with a movement (a loop).
pub const ANCHOR_RADIUS: f64 = 1.0;

/// Player commands with their aliases (all handled and cancelled before the
/// server runs them, so passwords never reach the log).
pub const PLAYER_COMMANDS: &[(&str, &[&str])] = &[
    ("login", &["l", "log"]),
    ("register", &["reg"]),
    ("changepassword", &["changepass", "cp"]),
    ("unregister", &["unreg"]),
    ("logout", &["destroysession"]),
    ("2fa", &["totp"]),
    ("premium", &["license"]),
];

/// The canonical name of a player command, if `name` is one.
pub fn player_command(name: &str) -> Option<&'static str> {
    let name = name.to_lowercase();
    PLAYER_COMMANDS.iter().find(|(n, aliases)| *n == name || aliases.contains(&name.as_str())).map(|(n, _)| *n)
}

/// What the layer knows about a player when they join.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinInfo {
    pub uuid: String,
    pub name: String,
    /// Address without the port.
    pub ip: String,
    /// Version 4 UUID with a signed skin ([`pumbo_common::id::is_authenticated_profile`]).
    pub authenticated: bool,
    /// Client locale (`pl_pl`): the player's messages use that language when
    /// PumboAuth has it ([`Auth::others`]).
    pub locale: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Dark screen on the client, hidden from other players.
    Hold(String),
    /// Back to a position in the player's own world (rotation unchanged).
    Teleport(String, (f64, f64, f64)),
    Say(String, Text),
    Title {
        uuid: String,
        title: Text,
        subtitle: Text,
        stay_ticks: i32,
    },
    /// The countdown bar (`None` removes it).
    Bossbar(String, Option<(Text, f32)>),
    Kick(String, Text),
    /// Done with the player: release them unless PumboFilter still holds them.
    Finish(String),
    /// Release at once (the plugin is being unloaded).
    Unhold(String),
    /// `/pumboauth <args>` from this player: the layer runs
    /// [`crate::commands::run`] with the player's permissions.
    Admin {
        uuid: String,
        args: Vec<String>,
    },
    Log(String),
}

/// Answer to a connection before the player is in the world.
#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    Allow,
    Refuse(Text),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Register,
    Login,
    /// The password was right, the two-factor code is missing.
    Totp,
}

#[derive(Debug, Clone, PartialEq)]
enum State {
    Free,
    /// Held, waiting for PumboFilter to finish its checks.
    Waiting {
        unknown_since: Option<u64>,
    },
    /// `anchor`: where the player stands, from the first movement at the prompt.
    Prompt {
        step: Step,
        until: u64,
        attempts_left: u32,
        anchor: Option<(f64, f64, f64)>,
    },
}

#[derive(Debug, Clone, PartialEq)]
enum Pending {
    TotpSetup { secret: String, until: u64 },
    Premium { until: u64 },
}

#[derive(Debug, Clone)]
struct Player {
    name: String,
    ip: String,
    premium: bool,
    state: State,
    last_secret: u64,
    pending: Option<Pending>,
    bar_at: u64,
    /// Index into [`Auth::others`].
    lang: Option<usize>,
}

/// Counters for `/pumboauth stats`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub logins: u64,
    pub registrations: u64,
    pub sessions: u64,
    pub premium: u64,
    pub wrong: u64,
}

pub struct Auth {
    pub cfg: AuthSettings,
    pub lang: Lang,
    /// Other languages, for players whose client uses one of them (empty:
    /// everyone gets `lang`).
    pub others: Vec<Lang>,
    store: Option<Store>,
    /// Why the database is not available (nobody can log in then).
    pub store_error: Option<String>,
    players: HashMap<String, Player>,
    /// Names of the admin command (`pumboauth` and its aliases).
    pub admin_names: Vec<String>,
    pub stats: Stats,
}

fn key(uuid: &str) -> String {
    uuid.trim().to_lowercase()
}

impl Auth {
    pub fn new(cfg: AuthSettings, lang: Lang, store: pumbo_common::store::Result<Store>) -> Self {
        let (store, store_error) = match store {
            Ok(s) => (Some(s), None),
            Err(e) => (None, Some(e.to_string())),
        };
        Self {
            cfg,
            lang,
            others: Vec::new(),
            store,
            store_error,
            players: HashMap::new(),
            admin_names: vec!["pumboauth".into()],
            stats: Stats::default(),
        }
    }

    pub fn reload(&mut self, cfg: AuthSettings, lang: Lang) {
        self.cfg = cfg;
        self.lang = lang;
    }

    /// Runs `f` with `lang` set to the language of the player (swapped in and
    /// back out, so the messages below keep using `self.lang`).
    fn as_player<R>(&mut self, lang: Option<usize>, f: impl FnOnce(&mut Self) -> R) -> R {
        let Some(i) = lang.filter(|i| *i < self.others.len()) else { return f(self) };
        self.swap_lang(i);
        let r = f(self);
        self.swap_lang(i);
        r
    }

    fn swap_lang(&mut self, i: usize) {
        if let Some(other) = self.others.get_mut(i) {
            std::mem::swap(&mut self.lang, other);
        }
    }

    fn lang_of(&self, uuid: &str) -> Option<usize> {
        self.players.get(uuid).and_then(|p| p.lang)
    }

    /// The client's language became known after the join: messages from now on
    /// use it, and a player at the prompt gets it again in that language.
    pub fn set_locale(&mut self, uuid: &str, locale: Option<&str>, now: u64) -> Vec<Effect> {
        let uuid = key(uuid);
        let lang = pumbo_common::lang::index_for_locale(&self.others, locale);
        let Some(p) = self.players.get_mut(&uuid) else { return Vec::new() };
        if p.lang == lang {
            return Vec::new();
        }
        p.lang = lang;
        let State::Prompt { step, .. } = p.state else { return Vec::new() };
        self.as_player(lang, |a| a.prompt(&uuid, step, now))
    }

    pub fn accounts(&self) -> Option<AuthStore<'_>> {
        self.store.as_ref().map(AuthStore::new)
    }

    /// Players already online when the plugin loads: logged in.
    pub fn adopt(&mut self, uuid: &str, name: &str, ip: &str) {
        let p = Player {
            name: name.to_string(),
            ip: ip.to_string(),
            premium: false,
            state: State::Free,
            last_secret: 0,
            pending: None,
            bar_at: 0,
            lang: None,
        };
        self.players.insert(key(uuid), p);
    }

    pub fn holding(&self, uuid: &str) -> Hold {
        match self.players.get(&key(uuid)) {
            None => Hold::Unknown,
            Some(p) if p.state == State::Free => Hold::Free,
            Some(_) => Hold::Holding,
        }
    }

    /// Damage and actions are cancelled only for players this plugin holds.
    pub fn is_held(&self, uuid: &str) -> bool {
        self.holding(uuid) == Hold::Holding
    }

    pub fn held(&self) -> Vec<String> {
        self.players.iter().filter(|(_, p)| p.state != State::Free).map(|(u, _)| u.clone()).collect()
    }

    /// Players waiting for PumboFilter (the layer asks it about them).
    pub fn waiting(&self) -> Vec<String> {
        self.players.iter().filter(|(_, p)| matches!(p.state, State::Waiting { .. })).map(|(u, _)| u.clone()).collect()
    }

    /// The UUID of an online player by nickname.
    pub fn online_uuid(&self, name: &str) -> Option<String> {
        let k = name_key(name);
        self.players.iter().find(|(_, p)| name_key(&p.name) == k).map(|(u, _)| u.clone())
    }

    pub fn online_count(&self) -> usize {
        self.players.len()
    }

    fn text(&self, key: &str) -> Text {
        Text::parse(&self.lang.get(key))
    }

    fn text_args(&self, key: &str, args: &Args) -> Text {
        Text::parse(&self.lang.format(key, args))
    }

    fn say(&self, uuid: &str, t: Text) -> Effect {
        Effect::Say(uuid.to_string(), t)
    }

    fn err(&self, uuid: &str, key: &str, args: &Args) -> Effect {
        self.say(uuid, style::error(&self.lang, key, args))
    }

    fn usage(&self, uuid: &str, line: &str) -> Effect {
        self.say(uuid, style::usage(&self.lang, line))
    }

    fn register_line(&self) -> &'static str {
        if self.cfg.auth.register_repeat_password { "/register <password> <password>" } else { "/register <password>" }
    }

    /// A connection before it enters the world. `online`: the UUID of a player
    /// with the same UUID or nickname who is on the server already.
    pub fn pre_login(
        &mut self,
        name: &str,
        uuid: &str,
        ip: &str,
        online: Option<&str>,
        now: u64,
    ) -> (Entry, Vec<Effect>) {
        if !self.cfg.auth.enabled {
            return (Entry::Allow, Vec::new());
        }
        if let Some(old) = online {
            // One player never plays on two connections.
            if self.is_held(old) {
                let fx = vec![Effect::Kick(key(old), self.text("kick-other-connection"))];
                return (Entry::Refuse(self.text("kick-join-again")), fx);
            }
            return (Entry::Refuse(self.text("kick-already-online")), Vec::new());
        }
        if self.cfg.nickname.enabled {
            match check_nickname(name, &self.cfg.nickname) {
                Err(NickProblem::Invalid) => {
                    let n = &self.cfg.nickname;
                    let a = Args::new().arg(n.min_length).arg(n.max_length);
                    return (Entry::Refuse(self.text_args("kick-nickname-invalid", &a)), Vec::new());
                }
                Err(NickProblem::Blocked) => return (Entry::Refuse(self.text("kick-nickname-blocked")), Vec::new()),
                Ok(()) => {}
            }
        }
        let Some(store) = self.store.as_ref().map(AuthStore::new) else {
            return (Entry::Refuse(self.text("kick-db-error")), Vec::new());
        };
        let account = match store.account(name) {
            Ok(a) => a,
            Err(_) => return (Entry::Refuse(self.text("kick-db-error")), Vec::new()),
        };
        let a = &self.cfg.auth;
        if a.ip_bruteforce_max > 0
            && store.failures(Counter::Ip, ip, now, a.ip_window_ms()).unwrap_or(0) >= a.ip_bruteforce_max
        {
            return (Entry::Refuse(self.text("kick-ip-blocked")), Vec::new());
        }
        match account {
            Some(acc) => {
                if self.cfg.nickname.case_protection && acc.name != name {
                    let args = Args::new().arg(&acc.name).arg(name);
                    return (Entry::Refuse(self.text_args("kick-nickname-case", &args)), Vec::new());
                }
                let offline = Uuid::parse(uuid).is_some_and(|u| u.version() == 3);
                if acc.premium && offline && self.cfg.nickname.premium_protection {
                    return (Entry::Refuse(self.text("kick-premium-offline")), Vec::new());
                }
                if a.account_bruteforce_max > 0
                    && store.failures(Counter::Account, name, now, a.account_window_ms()).unwrap_or(0)
                        >= a.account_bruteforce_max
                {
                    return (Entry::Refuse(self.text("kick-account-locked")), Vec::new());
                }
            }
            None if a.disable_registrations => {
                return (Entry::Refuse(self.text("kick-registrations-disabled")), Vec::new());
            }
            None => {}
        }
        (Entry::Allow, Vec::new())
    }

    /// A player joins. `filter`: PumboFilter's answer about them (`None`: no
    /// PumboFilter).
    pub fn join(&mut self, info: JoinInfo, filter: Option<Hold>, now: u64) -> Vec<Effect> {
        let lang = pumbo_common::lang::index_for_locale(&self.others, info.locale.as_deref());
        self.as_player(lang, |a| a.join_as(info, filter, lang, now))
    }

    fn join_as(&mut self, info: JoinInfo, filter: Option<Hold>, lang: Option<usize>, now: u64) -> Vec<Effect> {
        let uuid = key(&info.uuid);
        let premium = info.authenticated && self.cfg.premium.enabled && self.cfg.premium.forwarded_profiles;
        let mut p = Player {
            name: info.name.clone(),
            ip: info.ip.clone(),
            premium,
            state: State::Free,
            last_secret: 0,
            pending: None,
            bar_at: 0,
            lang,
        };
        if !self.cfg.auth.enabled {
            self.players.insert(uuid, p);
            return Vec::new();
        }
        let Some(store) = self.store.as_ref().map(AuthStore::new) else {
            self.players.insert(uuid.clone(), p);
            return vec![Effect::Kick(uuid, self.text("kick-db-error"))];
        };
        let mut fx = Vec::new();
        let account = if premium {
            match self.premium_account(&store, &info, now) {
                Ok(PremiumJoin::LoggedIn(log)) => {
                    fx.extend(log.map(Effect::Log));
                    self.stats.premium += 1;
                    fx.push(self.say(&uuid, style::success(&self.lang, "auth-premium-login", &Args::new())));
                    self.players.insert(uuid, p);
                    return fx;
                }
                Ok(PremiumJoin::Password(acc)) => {
                    let acc = *acc;
                    let hint = Args::new().arg(style::command("/premium <password>"));
                    fx.push(self.say(&uuid, style::info(&self.lang, "auth-premium-hint", &hint)));
                    Some(acc)
                }
                Ok(PremiumJoin::Conflict) => {
                    self.players.insert(uuid.clone(), p);
                    return vec![Effect::Kick(uuid, self.text("kick-account-conflict"))];
                }
                Err(_) => {
                    self.players.insert(uuid.clone(), p);
                    return vec![Effect::Kick(uuid, self.text("kick-db-error"))];
                }
            }
        } else {
            match store.account(&info.name) {
                Ok(a) => a,
                Err(_) => {
                    self.players.insert(uuid.clone(), p);
                    return vec![Effect::Kick(uuid, self.text("kick-db-error"))];
                }
            }
        };
        if let Some(acc) = &account {
            if acc.premium && !premium && self.cfg.nickname.premium_protection {
                self.players.insert(uuid.clone(), p);
                return vec![Effect::Kick(uuid, self.text("kick-premium-offline"))];
            }
            if self.cfg.sessions.enabled && store.session_valid(&info.name, &info.ip, &uuid, now).unwrap_or(false) {
                self.stats.sessions += 1;
                fx.push(self.say(&uuid, style::success(&self.lang, "auth-session-login", &Args::new())));
                self.players.insert(uuid, p);
                return fx;
            }
        }
        fx.push(Effect::Hold(uuid.clone()));
        match filter {
            Some(Hold::Holding) => p.state = State::Waiting { unknown_since: None },
            Some(Hold::Unknown) => p.state = State::Waiting { unknown_since: Some(now) },
            Some(Hold::Free) | None => {}
        }
        let waiting = matches!(p.state, State::Waiting { .. });
        self.players.insert(uuid.clone(), p);
        if !waiting {
            let step = if account.is_some() { Step::Login } else { Step::Register };
            fx.extend(self.prompt(&uuid, step, now));
        }
        fx
    }

    fn premium_account(&self, store: &AuthStore<'_>, info: &JoinInfo, now: u64) -> Result<PremiumJoin, StoreError> {
        if let Some(acc) = store.account_by_premium_uuid(&info.uuid)? {
            // The same Mojang account under a new nickname: the account moves.
            let mut log = None;
            if name_key(&acc.name) != name_key(&info.name) {
                if store.account(&info.name)?.is_some() {
                    return Ok(PremiumJoin::Conflict);
                }
                store.rename_account(&acc.name, &info.name)?;
                log = Some(format!("premium account {} is now {}", acc.name, info.name));
            }
            return Ok(PremiumJoin::LoggedIn(log));
        }
        match store.account(&info.name)? {
            None => {
                if self.cfg.premium.save_accounts {
                    store.put_account(&Account {
                        name: info.name.clone(),
                        premium: true,
                        premium_uuid: Some(info.uuid.clone()),
                        uuid: Some(info.uuid.clone()),
                        reg_ip: Some(info.ip.clone()),
                        reg_time: now,
                        last_ip: Some(info.ip.clone()),
                        last_login: now,
                        ..Account::default()
                    })?;
                }
                Ok(PremiumJoin::LoggedIn(None))
            }
            // Marked premium by an admin (`forcepremium`): the first premium join claims it.
            Some(mut acc) if acc.premium && acc.premium_uuid.is_none() => {
                acc.premium_uuid = Some(info.uuid.clone());
                store.put_account(&acc)?;
                Ok(PremiumJoin::LoggedIn(Some(format!("premium account {} claimed by its Mojang account", acc.name))))
            }
            Some(acc) if acc.premium => Ok(PremiumJoin::Conflict),
            // A password account with this nickname: it is protected by its password.
            Some(acc) => Ok(PremiumJoin::Password(Box::new(acc))),
        }
    }

    /// Shows the login or registration prompt and starts the clock.
    fn prompt(&mut self, uuid: &str, step: Step, now: u64) -> Vec<Effect> {
        let until = now + u64::from(self.cfg.auth.timeout_seconds) * 1000;
        let attempts = self.cfg.auth.login_attempts;
        let Some(p) = self.players.get_mut(uuid) else { return Vec::new() };
        p.state = State::Prompt { step, until, attempts_left: attempts, anchor: None };
        p.bar_at = 0;
        let (msg, line, title, subtitle) = match step {
            Step::Register => {
                ("auth-register-prompt", self.register_line(), "auth-register-title", "auth-register-subtitle")
            }
            _ => ("auth-login-prompt", "/login <password>", "auth-login-title", "auth-login-subtitle"),
        };
        let stay = i32::try_from(self.cfg.auth.timeout_seconds.saturating_mul(20)).unwrap_or(1200);
        let mut fx = vec![
            self.say(uuid, style::info(&self.lang, msg, &Args::new().arg(style::command(line)))),
            Effect::Title {
                uuid: uuid.to_string(),
                title: self.text(title),
                subtitle: self.text(subtitle),
                stay_ticks: stay,
            },
        ];
        fx.extend(self.bossbar(uuid, now));
        fx
    }

    fn bossbar(&mut self, uuid: &str, now: u64) -> Option<Effect> {
        if !self.cfg.auth.bossbar {
            return None;
        }
        let total = u64::from(self.cfg.auth.timeout_seconds) * 1000;
        let p = self.players.get_mut(uuid)?;
        let State::Prompt { until, .. } = p.state else { return None };
        p.bar_at = now + BOSSBAR_EVERY_MS;
        let left = until.saturating_sub(now);
        let progress = (left as f32 / total.max(1) as f32).clamp(0.0, 1.0);
        let words = style::duration(&self.lang, Duration::from_millis(left.div_ceil(1000) * 1000));
        let text = self.text_args("auth-bossbar", &Args::new().arg(words));
        Some(Effect::Bossbar(uuid.to_string(), Some((text, progress))))
    }

    /// The server reported a movement: a player at the prompt who walks away
    /// is put back.
    pub fn moved(&mut self, uuid: &str, x: f64, y: f64, z: f64) -> Vec<Effect> {
        let uuid = key(uuid);
        self.as_player(self.lang_of(&uuid), |a| a.moved_as(uuid, x, y, z))
    }

    fn moved_as(&mut self, uuid: String, x: f64, y: f64, z: f64) -> Vec<Effect> {
        let Some(p) = self.players.get_mut(&uuid) else { return Vec::new() };
        let State::Prompt { anchor, .. } = &mut p.state else { return Vec::new() };
        match *anchor {
            None => {
                *anchor = Some((x, y, z));
                Vec::new()
            }
            Some(a) if (x - a.0).powi(2) + (y - a.1).powi(2) + (z - a.2).powi(2) > ANCHOR_RADIUS * ANCHOR_RADIUS => {
                vec![Effect::Teleport(uuid, a)]
            }
            Some(_) => Vec::new(),
        }
    }

    /// PumboFilter's latest answer about a waiting player (`None`: PumboFilter
    /// is gone).
    pub fn filter_state(&mut self, uuid: &str, filter: Option<Hold>, now: u64) -> Vec<Effect> {
        let uuid = key(uuid);
        self.as_player(self.lang_of(&uuid), |a| a.filter_state_as(uuid, filter, now))
    }

    fn filter_state_as(&mut self, uuid: String, filter: Option<Hold>, now: u64) -> Vec<Effect> {
        let Some(p) = self.players.get_mut(&uuid) else { return Vec::new() };
        let State::Waiting { unknown_since } = &mut p.state else { return Vec::new() };
        let go = match filter {
            Some(Hold::Holding) => {
                *unknown_since = None;
                false
            }
            Some(Hold::Unknown) => {
                let since = *unknown_since.get_or_insert(now);
                now.saturating_sub(since) >= UNKNOWN_FILTER_MS
            }
            Some(Hold::Free) | None => true,
        };
        if !go {
            return Vec::new();
        }
        let name = p.name.clone();
        let has_account = self.store.as_ref().map(AuthStore::new).map(|s| s.account(&name));
        match has_account {
            Some(Ok(a)) => self.prompt(&uuid, if a.is_some() { Step::Login } else { Step::Register }, now),
            _ => vec![Effect::Kick(uuid, self.text("kick-db-error"))],
        }
    }

    /// Logged in: the session, the account's last login, then release.
    fn logged_in(&mut self, uuid: &str, key: &str, now: u64, session: bool) -> Vec<Effect> {
        let Some(p) = self.players.get_mut(uuid) else { return Vec::new() };
        p.state = State::Free;
        let (name, ip) = (p.name.clone(), p.ip.clone());
        let mut fx = Vec::new();
        if let Some(store) = self.store.as_ref().map(AuthStore::new) {
            if session && self.cfg.sessions.enabled {
                let _ = store.create_session(&name, &ip, uuid, now + self.cfg.sessions.ttl_ms());
            }
            if let Ok(Some(mut acc)) = store.account(&name) {
                acc.last_ip = Some(ip);
                acc.last_login = now;
                acc.uuid.get_or_insert_with(|| uuid.to_string());
                let _ = store.put_account(&acc);
            }
        }
        self.stats.logins += 1;
        fx.push(self.say(uuid, style::success(&self.lang, key, &Args::new())));
        fx.push(Effect::Title {
            uuid: uuid.to_string(),
            title: Text::default(),
            subtitle: Text::default(),
            stay_ticks: 0,
        });
        fx.push(Effect::Bossbar(uuid.to_string(), None));
        fx.push(Effect::Finish(uuid.to_string()));
        fx
    }

    /// Chat from a player; `true` = cancel it (it is neither sent nor logged).
    pub fn chat(&mut self, uuid: &str) -> (bool, Vec<Effect>) {
        let uuid = key(uuid);
        self.as_player(self.lang_of(&uuid), |a| a.chat_as(uuid))
    }

    fn chat_as(&mut self, uuid: String) -> (bool, Vec<Effect>) {
        match self.players.get(&uuid).map(|p| p.state.clone()) {
            None | Some(State::Free) => (false, Vec::new()),
            // PumboFilter answers while it holds the player.
            Some(State::Waiting { .. }) => (true, Vec::new()),
            Some(State::Prompt { step, .. }) => (true, vec![self.blocked(&uuid, step)]),
        }
    }

    fn blocked(&self, uuid: &str, step: Step) -> Effect {
        match step {
            Step::Register => {
                self.err(uuid, "auth-blocked-register", &Args::new().arg(style::command(self.register_line())))
            }
            Step::Login => self.err(uuid, "auth-blocked", &Args::new().arg(style::command("/login <password>"))),
            Step::Totp => self.err(uuid, "auth-totp-prompt", &Args::new().arg(style::command("/2fa <code>"))),
        }
    }

    /// A command from a player (without the slash). Returns whether to cancel
    /// it: always for PumboAuth's own commands, and for every command of a held
    /// player.
    pub fn command(&mut self, uuid: &str, line: &str, now: u64) -> (bool, Vec<Effect>) {
        let uuid = key(uuid);
        self.as_player(self.lang_of(&uuid), |a| a.command_as(uuid, line, now))
    }

    fn command_as(&mut self, uuid: String, line: &str, now: u64) -> (bool, Vec<Effect>) {
        let (name, args) = pumbo_common::command::split_command(line);
        if self.admin_names.contains(&name) {
            // Cancelled too: `forceregister` has a password in it.
            return (true, vec![Effect::Admin { uuid, args }]);
        }
        let state = self.players.get(&uuid).map(|p| p.state.clone());
        let Some(cmd) = player_command(&name) else {
            return match state {
                None | Some(State::Free) => (false, Vec::new()),
                Some(State::Waiting { .. }) => (true, Vec::new()),
                Some(State::Prompt { step, .. }) => (true, vec![self.blocked(&uuid, step)]),
            };
        };
        let fx = match state {
            None => Vec::new(),
            Some(State::Waiting { .. }) => Vec::new(),
            Some(State::Prompt { step, .. }) => self.in_prompt(&uuid, step, cmd, &args, now),
            Some(State::Free) => self.when_free(&uuid, cmd, &args, now),
        };
        (true, fx)
    }

    fn in_prompt(&mut self, uuid: &str, step: Step, cmd: &str, args: &[String], now: u64) -> Vec<Effect> {
        match (step, cmd) {
            (Step::Register, "register") => self.register(uuid, args, now),
            (Step::Register, "login") => {
                vec![self.err(uuid, "auth-not-registered", &Args::new().arg(style::command(self.register_line())))]
            }
            (Step::Login, "login") => self.login(uuid, args, now),
            (Step::Login, "changepassword") => self.change_password(uuid, args, now, true),
            (Step::Login, "register") => {
                vec![self.err(uuid, "auth-already-registered", &Args::new().arg(style::command("/login <password>")))]
            }
            (Step::Totp, "2fa") => {
                let Some(code) = args.first() else { return vec![self.usage(uuid, "/2fa <code>")] };
                self.finish_totp(uuid, code, now)
            }
            (Step::Totp, "login") => match args.get(1) {
                // "/login <password> <code>" again: the password was checked already.
                Some(code) => self.finish_totp(uuid, code, now),
                None => vec![self.blocked(uuid, Step::Totp)],
            },
            (step, _) => vec![self.blocked(uuid, step)],
        }
    }

    fn when_free(&mut self, uuid: &str, cmd: &str, args: &[String], now: u64) -> Vec<Effect> {
        match cmd {
            "login" | "register" => {
                vec![self.say(uuid, style::info(&self.lang, "auth-already-logged-in", &Args::new()))]
            }
            "changepassword" => self.change_password(uuid, args, now, false),
            "unregister" => self.unregister(uuid, args, now),
            "logout" => self.logout(uuid),
            "2fa" => self.two_factor(uuid, args, now),
            "premium" => self.premium(uuid, args, now),
            _ => Vec::new(),
        }
    }

    fn names(&self, uuid: &str) -> Option<(String, String)> {
        self.players.get(uuid).map(|p| (p.name.clone(), p.ip.clone()))
    }

    /// A message instead of hashing, when the player sends password commands too fast.
    fn throttled(&mut self, uuid: &str, now: u64) -> Option<Effect> {
        let cooldown = u64::from(self.cfg.auth.command_cooldown_ms);
        let p = self.players.get_mut(uuid)?;
        if p.last_secret != 0 && now.saturating_sub(p.last_secret) < cooldown {
            return Some(self.err(uuid, "auth-cooldown", &Args::new()));
        }
        p.last_secret = now;
        None
    }

    /// The account of an online player, or the effects to show instead.
    fn account(&self, uuid: &str) -> Result<Account, Vec<Effect>> {
        let Some((name, _)) = self.names(uuid) else { return Err(Vec::new()) };
        let Some(store) = self.store.as_ref().map(AuthStore::new) else {
            return Err(vec![Effect::Kick(uuid.to_string(), self.text("kick-db-error"))]);
        };
        match store.account(&name) {
            Ok(Some(a)) => Ok(a),
            Ok(None) => {
                Err(vec![self.err(uuid, "auth-not-registered", &Args::new().arg(style::command(self.register_line())))])
            }
            Err(e) => Err(vec![
                Effect::Log(format!("cannot read the account {name}: {e}")),
                self.err(uuid, "auth-error", &Args::new()),
            ]),
        }
    }

    fn save(&self, uuid: &str, acc: &Account) -> Result<(), Vec<Effect>> {
        let Some(store) = self.store.as_ref().map(AuthStore::new) else {
            return Err(vec![Effect::Kick(uuid.to_string(), self.text("kick-db-error"))]);
        };
        store.put_account(acc).map_err(|e| {
            vec![
                Effect::Log(format!("cannot save the account {}: {e}", acc.name)),
                self.err(uuid, "auth-error", &Args::new()),
            ]
        })
    }

    /// Checks the password of an account (an old format is upgraded on success).
    fn check_password(&self, uuid: &str, acc: &mut Account, password: &str) -> Result<bool, Vec<Effect>> {
        let Some(stored) = acc.hash.clone() else {
            return Err(vec![self.err(uuid, "auth-premium-no-password", &Args::new())]);
        };
        let settings = self.cfg.hashing.argon2();
        match hashing::verify(password, &stored, settings) {
            Verify::Ok { needs_rehash } => {
                if needs_rehash
                    && self.cfg.hashing.rehash_legacy
                    && let Ok(h) = hashing::hash(password, settings)
                {
                    acc.hash = Some(h);
                    let _ = self.save(uuid, acc);
                }
                Ok(true)
            }
            Verify::Wrong => Ok(false),
            Verify::Unsupported => Err(vec![
                Effect::Log(format!("the account {} has a password hash in an unknown format", acc.name)),
                self.err(uuid, "auth-error", &Args::new()),
            ]),
        }
    }

    /// Counts a wrong password or code: address and account limits, attempts.
    fn wrong(&mut self, uuid: &str, now: u64, key: &str) -> Vec<Effect> {
        self.stats.wrong += 1;
        let Some((name, ip)) = self.names(uuid) else { return Vec::new() };
        let a = self.cfg.auth.clone();
        let (ip_count, acc_count) = match self.store.as_ref().map(AuthStore::new) {
            Some(s) => (
                s.record_failure(Counter::Ip, &ip, now, a.ip_window_ms()).unwrap_or(u32::MAX),
                s.record_failure(Counter::Account, &name, now, a.account_window_ms()).unwrap_or(u32::MAX),
            ),
            None => (u32::MAX, u32::MAX),
        };
        if a.ip_bruteforce_max > 0 && ip_count >= a.ip_bruteforce_max {
            return vec![
                Effect::Log(format!("address {ip} blocked after {ip_count} wrong passwords")),
                Effect::Kick(uuid.to_string(), self.text("kick-ip-blocked")),
            ];
        }
        if a.account_bruteforce_max > 0 && acc_count >= a.account_bruteforce_max {
            return vec![
                Effect::Log(format!("account {name} locked after {acc_count} wrong passwords")),
                Effect::Kick(uuid.to_string(), self.text("kick-account-locked")),
            ];
        }
        let Some(p) = self.players.get_mut(uuid) else { return Vec::new() };
        match &mut p.state {
            State::Prompt { attempts_left, .. } => {
                *attempts_left = attempts_left.saturating_sub(1);
                let left = *attempts_left;
                if left == 0 {
                    return vec![Effect::Kick(uuid.to_string(), self.text("kick-too-many-attempts"))];
                }
                vec![self.err(uuid, key, &Args::new().arg(style::value(left)))]
            }
            _ => vec![self.err(uuid, &format!("{key}-free"), &Args::new())],
        }
    }

    fn login(&mut self, uuid: &str, args: &[String], now: u64) -> Vec<Effect> {
        let Some(password) = args.first() else { return vec![self.usage(uuid, "/login <password>")] };
        if let Some(e) = self.throttled(uuid, now) {
            return vec![e];
        }
        let mut acc = match self.account(uuid) {
            Ok(a) => a,
            Err(fx) => return fx,
        };
        match self.check_password(uuid, &mut acc, password) {
            Err(fx) => fx,
            Ok(false) => self.wrong(uuid, now, "auth-wrong-password"),
            Ok(true) => {
                if self.cfg.two_factor.enabled && acc.totp_secret.is_some() {
                    if let Some(code) = args.get(1) {
                        return self.finish_totp(uuid, code, now);
                    }
                    if let Some(State::Prompt { step, .. }) = self.players.get_mut(uuid).map(|p| &mut p.state) {
                        *step = Step::Totp;
                    }
                    let a = Args::new().arg(style::command("/2fa <code>"));
                    return vec![
                        self.say(uuid, style::info(&self.lang, "auth-totp-prompt", &a)),
                        Effect::Title {
                            uuid: uuid.to_string(),
                            title: self.text("auth-totp-title"),
                            subtitle: self.text("auth-totp-subtitle"),
                            stay_ticks: 600,
                        },
                    ];
                }
                self.logged_in(uuid, "auth-login-success", now, true)
            }
        }
    }

    /// A TOTP code or a recovery code of the account; true when accepted.
    fn code_ok(&self, uuid: &str, acc: &mut Account, code: &str, now: u64) -> bool {
        let Some(secret) = acc.totp_secret.as_deref().and_then(totp::base32_decode) else { return false };
        if let Some(step) = totp::verify(&secret, code, now / 1000, acc.totp_last_step) {
            acc.totp_last_step = Some(step);
            let _ = self.save(uuid, acc);
            return true;
        }
        let hashed = totp::hash_recovery(code);
        if let Some(i) = acc.recovery.iter().position(|h| *h == hashed) {
            acc.recovery.remove(i);
            let _ = self.save(uuid, acc);
            return true;
        }
        false
    }

    fn finish_totp(&mut self, uuid: &str, code: &str, now: u64) -> Vec<Effect> {
        let mut acc = match self.account(uuid) {
            Ok(a) => a,
            Err(fx) => return fx,
        };
        if self.code_ok(uuid, &mut acc, code, now) {
            return self.logged_in(uuid, "auth-login-success", now, true);
        }
        self.wrong(uuid, now, "auth-totp-wrong")
    }

    fn password_problem(&self, uuid: &str, p: PasswordProblem) -> Effect {
        match p {
            PasswordProblem::TooShort(n) => {
                self.err(uuid, "auth-password-too-short", &Args::new().arg(style::value(n)))
            }
            PasswordProblem::TooLong(n) => self.err(uuid, "auth-password-too-long", &Args::new().arg(style::value(n))),
            PasswordProblem::Common => self.err(uuid, "auth-password-unsafe", &Args::new()),
            PasswordProblem::SameAsName => self.err(uuid, "auth-password-name", &Args::new()),
        }
    }

    fn register(&mut self, uuid: &str, args: &[String], now: u64) -> Vec<Effect> {
        let repeat = self.cfg.auth.register_repeat_password;
        let line = self.register_line();
        let (Some(password), confirm) = (args.first(), args.get(1)) else { return vec![self.usage(uuid, line)] };
        if repeat {
            match confirm {
                None => return vec![self.usage(uuid, line)],
                Some(c) if c != password => {
                    return vec![self.err(uuid, "auth-passwords-differ", &Args::new().arg(style::command(line)))];
                }
                Some(_) => {}
            }
        }
        let Some((name, ip)) = self.names(uuid) else { return Vec::new() };
        if let Err(p) = check_password(password, &name, &self.cfg.auth) {
            return vec![self.password_problem(uuid, p)];
        }
        if let Some(e) = self.throttled(uuid, now) {
            return vec![e];
        }
        let Some(store) = self.store.as_ref().map(AuthStore::new) else {
            return vec![Effect::Kick(uuid.to_string(), self.text("kick-db-error"))];
        };
        match store.account(&name) {
            Ok(None) => {}
            Ok(Some(_)) => {
                if let Some(State::Prompt { step, .. }) = self.players.get_mut(uuid).map(|p| &mut p.state) {
                    *step = Step::Login;
                }
                return vec![self.err(
                    uuid,
                    "auth-already-registered",
                    &Args::new().arg(style::command("/login <password>")),
                )];
            }
            Err(_) => return vec![Effect::Kick(uuid.to_string(), self.text("kick-db-error"))],
        }
        let a = &self.cfg.auth;
        if a.ip_registrations_max > 0
            && store.registrations(&ip, now, a.registration_window_ms()).unwrap_or(u32::MAX) >= a.ip_registrations_max
        {
            return vec![Effect::Kick(uuid.to_string(), self.text("kick-ip-registrations"))];
        }
        let Ok(hash) = hashing::hash(password, self.cfg.hashing.argon2()) else {
            return vec![self.err(uuid, "auth-error", &Args::new())];
        };
        let acc = Account {
            name: name.clone(),
            hash: Some(hash),
            uuid: Some(uuid.to_string()),
            reg_ip: Some(ip.clone()),
            reg_time: now,
            password_changed: now,
            ..Account::default()
        };
        if let Err(fx) = self.save(uuid, &acc) {
            return fx;
        }
        if a.ip_registrations_max > 0 {
            let _ = store.add_registration(&ip, now, a.registration_window_ms());
        }
        self.stats.registrations += 1;
        let mut fx = vec![Effect::Log(format!("{name} registered from {ip}"))];
        fx.extend(self.logged_in(uuid, "auth-register-success", now, true));
        fx
    }

    fn change_password(&mut self, uuid: &str, args: &[String], now: u64, in_prompt: bool) -> Vec<Effect> {
        let line = "/changepassword <old password> <new password>";
        let (Some(old), Some(new)) = (args.first(), args.get(1)) else { return vec![self.usage(uuid, line)] };
        let mut acc = match self.account(uuid) {
            Ok(a) => a,
            Err(fx) => return fx,
        };
        if acc.hash.is_none() {
            return vec![self.err(uuid, "auth-premium-no-password", &Args::new())];
        }
        if let Err(p) = check_password(new, &acc.name, &self.cfg.auth) {
            return vec![self.password_problem(uuid, p)];
        }
        if let Some(e) = self.throttled(uuid, now) {
            return vec![e];
        }
        match self.check_password(uuid, &mut acc, old) {
            Err(fx) => return fx,
            Ok(false) => return self.wrong(uuid, now, "auth-wrong-password"),
            Ok(true) => {}
        }
        if in_prompt && self.cfg.two_factor.enabled && acc.totp_secret.is_some() {
            // Changing the password does not skip the second factor.
            let Some(code) = args.get(2) else {
                return vec![self.err(
                    uuid,
                    "auth-totp-needed",
                    &Args::new().arg(style::command(format!("{line} <code>"))),
                )];
            };
            if !self.code_ok(uuid, &mut acc, code, now) {
                return self.wrong(uuid, now, "auth-totp-wrong");
            }
        }
        let Ok(hash) = hashing::hash(new, self.cfg.hashing.argon2()) else {
            return vec![self.err(uuid, "auth-error", &Args::new())];
        };
        acc.hash = Some(hash);
        acc.password_changed = now;
        if let Err(fx) = self.save(uuid, &acc) {
            return fx;
        }
        if let Some(s) = self.store.as_ref().map(AuthStore::new) {
            let _ = s.delete_sessions(&acc.name);
        }
        let mut fx = vec![
            Effect::Log(format!("{} changed the password", acc.name)),
            self.say(uuid, style::success(&self.lang, "changepassword-success", &Args::new())),
        ];
        if in_prompt {
            fx.extend(self.logged_in(uuid, "auth-login-success", now, true));
        } else if let (true, Some(s), Some((name, ip))) =
            (self.cfg.sessions.enabled, self.store.as_ref().map(AuthStore::new), self.names(uuid))
        {
            let _ = s.create_session(&name, &ip, uuid, now + self.cfg.sessions.ttl_ms());
        }
        fx
    }

    fn unregister(&mut self, uuid: &str, args: &[String], now: u64) -> Vec<Effect> {
        let Some(password) = args.first() else { return vec![self.usage(uuid, "/unregister <password>")] };
        let mut acc = match self.account(uuid) {
            Ok(a) => a,
            Err(fx) => return fx,
        };
        if let Some(e) = self.throttled(uuid, now) {
            return vec![e];
        }
        match self.check_password(uuid, &mut acc, password) {
            Err(fx) => fx,
            Ok(false) => self.wrong(uuid, now, "auth-wrong-password"),
            Ok(true) => {
                if let Some(s) = self.store.as_ref().map(AuthStore::new)
                    && let Err(e) = s.delete_account(&acc.name)
                {
                    return vec![
                        Effect::Log(format!("cannot remove {}: {e}", acc.name)),
                        self.err(uuid, "auth-error", &Args::new()),
                    ];
                }
                vec![
                    Effect::Log(format!("{} removed their account", acc.name)),
                    Effect::Kick(uuid.to_string(), self.text("kick-unregistered")),
                ]
            }
        }
    }

    fn logout(&mut self, uuid: &str) -> Vec<Effect> {
        let Some((name, _)) = self.names(uuid) else { return Vec::new() };
        if let Some(s) = self.store.as_ref().map(AuthStore::new) {
            let _ = s.delete_sessions(&name);
        }
        vec![self.say(uuid, style::success(&self.lang, "logout-success", &Args::new()))]
    }

    fn two_factor(&mut self, uuid: &str, args: &[String], now: u64) -> Vec<Effect> {
        let usage = "/2fa <enable|confirm|disable> ...";
        if !self.cfg.two_factor.enabled {
            return vec![self.err(uuid, "totp-off", &Args::new())];
        }
        let sub = args.first().map(|s| s.to_lowercase()).unwrap_or_default();
        let mut acc = match self.account(uuid) {
            Ok(a) => a,
            Err(fx) => return fx,
        };
        match sub.as_str() {
            "enable" => {
                if acc.totp_secret.is_some() {
                    return vec![self.err(uuid, "totp-already-enabled", &Args::new())];
                }
                let Some(password) = args.get(1) else { return vec![self.usage(uuid, "/2fa enable <password>")] };
                if let Some(e) = self.throttled(uuid, now) {
                    return vec![e];
                }
                match self.check_password(uuid, &mut acc, password) {
                    Err(fx) => return fx,
                    Ok(false) => return self.wrong(uuid, now, "auth-wrong-password"),
                    Ok(true) => {}
                }
                let secret = totp::base32_encode(&totp::new_secret());
                let url = totp::otpauth_url(&self.cfg.two_factor.issuer, &acc.name, &secret);
                if let Some(p) = self.players.get_mut(uuid) {
                    p.pending = Some(Pending::TotpSetup { secret: secret.clone(), until: now + PENDING_TOTP_MS });
                }
                let copy = Text::from(
                    Line::new().with(
                        Segment::new(self.lang.get("totp-setup-copy"), pumbo_common::text::Style::colored(style::WARN))
                            .on_click(Click::Copy(url)),
                    ),
                );
                vec![
                    self.say(uuid, style::info(&self.lang, "totp-setup", &Args::new().arg(style::value(&secret)))),
                    self.say(uuid, copy),
                    self.say(
                        uuid,
                        style::info(
                            &self.lang,
                            "totp-confirm-prompt",
                            &Args::new().arg(style::command("/2fa confirm <code>")),
                        ),
                    ),
                ]
            }
            "confirm" => {
                let pending = self.players.get(uuid).and_then(|p| match &p.pending {
                    Some(Pending::TotpSetup { secret, .. }) => Some(secret.clone()),
                    _ => None,
                });
                let Some(secret) = pending else {
                    return vec![self.err(
                        uuid,
                        "totp-not-pending",
                        &Args::new().arg(style::command("/2fa enable <password>")),
                    )];
                };
                let Some(code) = args.get(1) else { return vec![self.usage(uuid, "/2fa confirm <code>")] };
                let raw = totp::base32_decode(&secret).unwrap_or_default();
                let Some(step) = totp::verify(&raw, code, now / 1000, None) else {
                    return vec![self.err(uuid, "totp-wrong-code", &Args::new())];
                };
                let codes = totp::recovery_codes(self.cfg.two_factor.recovery_codes);
                acc.totp_secret = Some(secret);
                acc.totp_last_step = Some(step);
                acc.recovery = codes.iter().map(|c| totp::hash_recovery(c)).collect();
                if let Err(fx) = self.save(uuid, &acc) {
                    return fx;
                }
                if let Some(p) = self.players.get_mut(uuid) {
                    p.pending = None;
                }
                let mut fx = vec![
                    Effect::Log(format!("{} turned on two-factor login", acc.name)),
                    self.say(uuid, style::success(&self.lang, "totp-enabled", &Args::new())),
                ];
                if !codes.is_empty() {
                    fx.push(self.say(
                        uuid,
                        style::info(
                            &self.lang,
                            "totp-recovery-codes",
                            &Args::new().arg(style::value(codes.join("  "))),
                        ),
                    ));
                }
                fx
            }
            "disable" => {
                if acc.totp_secret.is_none() {
                    return vec![self.err(uuid, "totp-not-enabled", &Args::new())];
                }
                let Some(code) = args.get(1) else { return vec![self.usage(uuid, "/2fa disable <code>")] };
                if !self.code_ok(uuid, &mut acc, code, now) {
                    return self.wrong(uuid, now, "auth-totp-wrong");
                }
                acc.totp_secret = None;
                acc.totp_last_step = None;
                acc.recovery.clear();
                if let Err(fx) = self.save(uuid, &acc) {
                    return fx;
                }
                vec![
                    Effect::Log(format!("{} turned off two-factor login", acc.name)),
                    self.say(uuid, style::success(&self.lang, "totp-disabled", &Args::new())),
                ]
            }
            _ => vec![self.usage(uuid, usage)],
        }
    }

    fn premium(&mut self, uuid: &str, args: &[String], now: u64) -> Vec<Effect> {
        let keyword = self.cfg.auth.confirm_keyword.clone();
        let verified = self.players.get(uuid).is_some_and(|p| p.premium);
        let mut acc = match self.account(uuid) {
            Ok(a) => a,
            Err(fx) => return fx,
        };
        if acc.premium {
            return vec![self.say(uuid, style::info(&self.lang, "premium-already", &Args::new()))];
        }
        let Some(arg) = args.first() else { return vec![self.usage(uuid, "/premium <password>")] };
        if !verified {
            return vec![self.err(uuid, "premium-not-verified", &Args::new())];
        }
        if arg.eq_ignore_ascii_case(&keyword) {
            let pending = self
                .players
                .get(uuid)
                .is_some_and(|p| matches!(p.pending, Some(Pending::Premium { until }) if until > now));
            if !pending {
                return vec![self.err(
                    uuid,
                    "premium-not-pending",
                    &Args::new().arg(style::command("/premium <password>")),
                )];
            }
            acc.premium = true;
            acc.premium_uuid = Some(uuid.to_string());
            acc.hash = None;
            acc.totp_secret = None;
            acc.totp_last_step = None;
            acc.recovery.clear();
            if let Err(fx) = self.save(uuid, &acc) {
                return fx;
            }
            if let Some(s) = self.store.as_ref().map(AuthStore::new) {
                let _ = s.delete_sessions(&acc.name);
            }
            if let Some(p) = self.players.get_mut(uuid) {
                p.pending = None;
            }
            return vec![
                Effect::Log(format!("{} switched to premium login", acc.name)),
                self.say(uuid, style::success(&self.lang, "premium-enabled", &Args::new())),
            ];
        }
        if let Some(e) = self.throttled(uuid, now) {
            return vec![e];
        }
        match self.check_password(uuid, &mut acc, arg) {
            Err(fx) => fx,
            Ok(false) => self.wrong(uuid, now, "auth-wrong-password"),
            Ok(true) => {
                if let Some(p) = self.players.get_mut(uuid) {
                    p.pending = Some(Pending::Premium { until: now + PENDING_PREMIUM_MS });
                }
                let a = Args::new().arg(style::command(format!("/premium {keyword}")));
                vec![self.say(uuid, style::warn(&self.lang, "premium-warning", &a))]
            }
        }
    }

    /// Timers: login time, the countdown bar, pending confirmations.
    pub fn tick(&mut self, now: u64) -> Vec<Effect> {
        let mut fx = Vec::new();
        let uuids: Vec<String> = self.players.keys().cloned().collect();
        for uuid in uuids {
            fx.extend(self.as_player(self.lang_of(&uuid), |a| a.tick_player(&uuid, now)));
        }
        fx
    }

    fn tick_player(&mut self, uuid: &str, now: u64) -> Vec<Effect> {
        let Some(p) = self.players.get_mut(uuid) else { return Vec::new() };
        if let Some(Pending::TotpSetup { until, .. } | Pending::Premium { until }) = &p.pending
            && *until <= now
        {
            p.pending = None;
        }
        match p.state {
            State::Prompt { until, .. } if now >= until => {
                vec![Effect::Kick(uuid.to_string(), self.text("kick-auth-timeout"))]
            }
            State::Prompt { .. } if now >= p.bar_at => self.bossbar(uuid, now).into_iter().collect(),
            _ => Vec::new(),
        }
    }

    pub fn leave(&mut self, uuid: &str) {
        self.players.remove(&key(uuid));
    }

    /// The plugin is unloaded. Pumpkin lets no plugin disconnect a player while
    /// it is being unloaded, so players who did not log in are released and
    /// told to join again (nothing guards the server without the plugin).
    pub fn unload(&mut self) -> Vec<Effect> {
        let mut fx = Vec::new();
        for uuid in self.held() {
            fx.push(Effect::Bossbar(uuid.clone(), None));
            fx.push(Effect::Unhold(uuid.clone()));
            fx.push(Effect::Say(uuid, style::warn(&self.lang, "reload-join-again", &Args::new())));
        }
        fx
    }

    /// `forcelogin`: logs in an online player who is at the prompt.
    pub fn force_login(&mut self, uuid: &str, now: u64) -> Option<Vec<Effect>> {
        let uuid = key(uuid);
        self.as_player(self.lang_of(&uuid), |a| a.force_login_as(uuid, now))
    }

    fn force_login_as(&mut self, uuid: String, now: u64) -> Option<Vec<Effect>> {
        match self.players.get(&uuid).map(|p| &p.state) {
            Some(State::Prompt { .. } | State::Waiting { .. }) => {
                Some(self.logged_in(&uuid, "auth-login-success", now, false))
            }
            _ => None,
        }
    }

    pub fn store_ref(&self) -> Option<&Store> {
        self.store.as_ref()
    }
}

enum PremiumJoin {
    LoggedIn(Option<String>),
    /// A password account with this nickname: the player logs in with it.
    Password(Box<Account>),
    Conflict,
}

#[cfg(test)]
mod tests;
