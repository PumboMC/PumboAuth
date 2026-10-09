//! PumboAuth for the PumboProx proxy: one account database for the whole
//! network, the login before any server sees the player.
//!
//! All rules live in `pumbo-auth-core`; this crate connects them to the proxy:
//!
//! - `on-pre-login`: nickname rules, blocked addresses and accounts, closed
//!   registrations (premium is decided by the proxy: `online-mode`),
//! - `on-gate` (gate `auth`, priority 200, after PumboFilter): premium players
//!   (logged in by the proxy with Mojang) and players with a session pass;
//!   the others wait in the virtual world for `/register` or `/login` (and
//!   `/2fa`), with a countdown bar. No proxy command runs there, so `/server`
//!   cannot skip the login, and no server sees the player before it,
//! - `/changepassword`, `/unregister`, `/logout`, `/2fa`, `/premium` on every
//!   server: the proxy takes them before the backend (sensitive commands are
//!   never logged), `/pumboauth <sub>` (`/pumbo auth <sub>`) for admins,
//! - for other plugins: the topics `pumbo:player-authenticated@1.0` and
//!   `pumbo:player-registered@1.0`, the placeholders `%pumboauth_logged_in%`,
//!   `%pumboauth_premium%` and `%pumboauth_registered%`; a ban from PumboBans
//!   (`pumbo:player-punished@1.0`) ends the player's sessions.

pub mod setup;

use std::cell::{OnceCell, RefCell};
use std::collections::{BTreeSet, HashMap};

use pumbo_auth_core::commands::{self as cmds, Outcome, Sender};
use pumbo_auth_core::flow::{Auth, Effect, Entry, JoinInfo, PLAYER_COMMANDS, Stats};
use pumbo_common::clock::now_ms;
use pumbo_common::id::{Uuid, strip_port};
use pumbo_common::rich::{Text as Rich, json};
use pumbo_common::store::{Store, StoreError};
use pumbo_common::style;
use pumbo_common::text::Args;
use pumbo_sdk::bindings::pumbo::prox::types::TitleTimes;
use pumbo_sdk::bossbar::{Bar, BossbarColor, BossbarOverlay};
use pumbo_sdk::contracts::{
    AuthMethod, PLAYER_AUTHENTICATED, PLAYER_PUNISHED, PLAYER_REGISTERED, PlayerAuthenticated, PlayerRegistered,
    Punishment, PunishmentKind,
};
use pumbo_sdk::virtual_world::{self as v, Input};
use pumbo_sdk::{
    Command, CommandEvent, Context, GateReply, PlayerId, PlayerInfo, PreLoginEvent, PreLoginReply, QueryContext, Text,
    bus, log, permissions, placeholders, players, scheduler,
};

const TICK_MS: u64 = 250;
/// Standard subcommands the proxy runs itself under `/pumbo auth`.
const HOST_SUBCOMMANDS: &[&str] = &["reload", "version", "debug"];
/// Umbrella names of the admin subcommands: `/pumboauth unregister <nick>` must
/// not look like the player's `/unregister <password>` in `on-command`.
const ADMIN: &str = "admin-";
/// Admin subcommands with a password in them.
const SECRET_SUBS: &[&str] = &["forceregister", "forcechangepassword"];
/// Player commands that only make sense before the login (virtual world only).
const PROMPT_COMMANDS: &[&str] = &["login", "register"];
const BUSY: &str = "&cPumboAuth is busy, please join again.";

#[derive(Default)]
struct Seen {
    uuid: String,
    name: String,
    /// Held by this gate, in the virtual world.
    held: bool,
    bar: Option<Bar>,
}

struct State {
    auth: Auth,
    players: HashMap<PlayerId, Seen>,
    by_uuid: HashMap<String, PlayerId>,
}

pub struct PumboAuth {
    state: RefCell<Option<State>>,
    /// Read-only config folder (`plugins/pumbo-auth/`).
    pub config_dir: String,
    /// Data folder (`plugins/data/pumbo-auth/`), holds `auth.redb` and import files.
    pub data_dir: String,
}

impl Default for PumboAuth {
    fn default() -> Self {
        PumboAuth { state: RefCell::new(None), config_dir: "/config".into(), data_dir: "/data".into() }
    }
}

fn uuid_of(p: &PlayerInfo) -> String {
    Uuid::from_high_low(p.profile.id.high, p.profile.id.low).to_string()
}

fn text(t: &Rich) -> Text {
    Text::Json(json(t))
}

fn reply(to: Option<PlayerId>, t: &Rich) {
    if t.is_empty() {
        return;
    }
    match to {
        Some(id) => players::send_message(id, text(t)),
        None => log::info(&t.plain()),
    }
}

fn uuid_in(e: &Effect) -> Option<&str> {
    match e {
        Effect::Hold(u)
        | Effect::Teleport(u, _)
        | Effect::Say(u, _)
        | Effect::Bossbar(u, _)
        | Effect::Kick(u, _)
        | Effect::Finish(u)
        | Effect::Unhold(u) => Some(u),
        Effect::Title { uuid, .. } | Effect::Admin { uuid, .. } => Some(uuid),
        Effect::Log(_) => None,
    }
}

/// What a call of the core did, read from its counters: a login and how
/// (2FA logins count as password logins), a new account.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Done {
    login: Option<AuthMethod>,
    registered: bool,
}

fn done(before: Stats, after: Stats) -> Done {
    let login = if after.premium > before.premium {
        Some(AuthMethod::Premium)
    } else if after.sessions > before.sessions {
        Some(AuthMethod::Session)
    } else if after.logins > before.logins {
        Some(AuthMethod::Password)
    } else {
        None
    };
    Done { login, registered: after.registrations > before.registrations }
}

thread_local! {
    /// One world for every waiting player: each gets its own view of it.
    static WORLD: OnceCell<v::World> = const { OnceCell::new() };
}

fn position((x, y, z): (f64, f64, f64)) -> v::Position {
    v::Position { x, y, z, yaw: 0.0, pitch: 0.0 }
}

/// Into the virtual world, made at the first call (the barrier platform).
fn enter(id: PlayerId) -> Result<(), String> {
    WORLD.with(|w| {
        let world = w.get_or_init(|| {
            let opts = v::WorldOptions { time: 18_000, light: 15, game_mode: v::GameMode::Adventure, view_distance: 2 };
            let world = v::World::new(opts);
            for (x, y, z) in setup::platform() {
                let _ = world.set_block(v::BlockPos { x, y, z }, "minecraft:barrier");
            }
            world
        });
        let commands: Vec<String> = PLAYER_COMMANDS
            .iter()
            .flat_map(|(n, a)| std::iter::once(*n).chain(a.iter().copied()))
            .map(String::from)
            .collect();
        v::enter(id, world, position(setup::SPAWN), &commands)
    })
}

impl PumboAuth {
    /// Runs `f` with the state; `None` when it is missing or borrowed. Never
    /// `.await` inside `f`.
    fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> Option<R> {
        self.state.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f))
    }

    /// Runs a core call for player `uuid` and tells the network what it did.
    fn track<R>(&self, uuid: &str, f: impl FnOnce(&mut Auth) -> R) -> Option<R> {
        let (r, d, name, premium) = self.with(|s| {
            let before = s.auth.stats;
            let r = f(&mut s.auth);
            let d = done(before, s.auth.stats);
            let id = s.by_uuid.get(uuid).copied();
            let name = id.and_then(|id| s.players.get(&id)).map(|p| p.name.clone()).unwrap_or_default();
            (r, d, (id, name), d.login == Some(AuthMethod::Premium))
        })?;
        if let (Some(id), name) = name {
            self.announce(id, uuid, &name, d, premium);
        }
        Some(r)
    }

    fn announce(&self, id: PlayerId, uuid: &str, name: &str, d: Done, premium: bool) {
        let simple = Uuid::parse(uuid).map(|u| u.simple()).unwrap_or_default();
        if d.registered {
            let at = now_ms() / 1000;
            let ev = PlayerRegistered { uuid: simple.clone(), name: name.to_string(), at };
            if let Err(e) = bus::publish_value(&PLAYER_REGISTERED.versioned(), &ev) {
                log::debug(&format!("PumboAuth: registration not published: {e}"));
            }
        }
        if let Some(method) = d.login {
            let ev = PlayerAuthenticated { uuid: simple, name: name.to_string(), method };
            if let Err(e) = bus::publish_value(&PLAYER_AUTHENTICATED.versioned(), &ev) {
                log::debug(&format!("PumboAuth: login not published: {e}"));
            }
            for (key, value) in [("logged_in", true), ("premium", premium), ("registered", true)] {
                let _ = placeholders::set(key, Some(id), pumbo_sdk::text::plain(value.to_string()), &Context::Global);
            }
        }
    }

    pub fn start(&self, store: Result<Store, StoreError>) -> Result<(), String> {
        let (cfg, mut warnings, found) = setup::load_config(&self.config_dir);
        if !found {
            log::info("PumboAuth: no config.yml in plugins/pumbo-auth/, using the defaults");
        }
        let (lang, others, w) = setup::load_langs(&self.config_dir, &cfg);
        warnings.extend(w);
        for w in &warnings {
            log::warn(&format!("PumboAuth: config: {w}"));
        }
        let mut auth = Auth::new(cfg.settings(), lang, store);
        auth.others = others;
        if let Some(why) = &auth.store_error {
            log::error(&format!("PumboAuth: the database is not available, nobody can log in: {why}"));
        }
        for (name, aliases) in PLAYER_COMMANDS {
            let mut spec = Command::new(name).sensitive().usage(&format!("/{name}"));
            if PROMPT_COMMANDS.contains(name) {
                spec = spec.virtual_only();
            }
            for a in *aliases {
                spec = spec.alias(a);
            }
            if let Err(e) = spec.register() {
                log::warn(&format!("PumboAuth: /{name} not registered: {e}"));
            }
        }
        let tree = cmds::tree();
        for s in tree.subs().iter().filter(|s| !HOST_SUBCOMMANDS.contains(&s.name)) {
            let mut spec = Command::new(&format!("{ADMIN}{}", s.name))
                .alias(s.name)
                .permission(&tree.permission(s))
                .usage(&format!("/pumboauth {} {}", s.name, s.usage))
                .umbrella();
            if SECRET_SUBS.contains(&s.name) {
                spec = spec.sensitive();
            }
            if let Err(e) = spec.register() {
                log::warn(&format!("PumboAuth: /pumboauth {} not registered: {e}", s.name));
            }
        }
        if let Err(e) = Command::new("help").usage("/pumboauth help [page]").umbrella().register() {
            log::warn(&format!("PumboAuth: /pumboauth help not registered: {e}"));
        }
        scheduler::every(TICK_MS);
        *self.state.borrow_mut() = Some(State { auth, players: HashMap::new(), by_uuid: HashMap::new() });
        log::info(&format!("PumboAuth {} loaded (gate \"auth\")", env!("CARGO_PKG_VERSION")));
        Ok(())
    }

    /// The number of warnings, or the error of a file that is not valid YAML
    /// (then the current settings stay).
    fn reload(&self) -> Result<usize, String> {
        let (cfg, mut warnings, _) = setup::load_config(&self.config_dir);
        let (lang, others, w) = setup::load_langs(&self.config_dir, &cfg);
        warnings.extend(w);
        if let Some(w) = warnings.iter().find(|w| w.fatal) {
            log::warn(&format!("PumboAuth: reload refused, the current settings stay: {w}"));
            return Err(w.message.clone());
        }
        for w in &warnings {
            log::warn(&format!("PumboAuth: config: {w}"));
        }
        self.with(|s| {
            s.auth.reload(cfg.settings(), lang);
            s.auth.others = others;
        });
        Ok(warnings.len())
    }

    /// Carries out the core's effects; `by` is who ran a command (admin effects).
    fn apply(&self, effects: Vec<Effect>) {
        for e in effects {
            if let Effect::Log(line) = &e {
                log::info(&format!("PumboAuth: {line}"));
                continue;
            }
            let Some(uuid) = uuid_in(&e).map(str::to_string) else { continue };
            // Before the world shows the proxy keeps messages, titles and bars.
            let target = self
                .with(|s| {
                    let id = *s.by_uuid.get(&uuid)?;
                    Some((id, s.players.get(&id)?.held))
                })
                .flatten();
            let Some((id, held)) = target else { continue };
            match e {
                Effect::Teleport(_, at) => {
                    v::teleport(id, position(at));
                }
                Effect::Say(_, t) => reply(Some(id), &t),
                Effect::Title { title, subtitle, stay_ticks, .. } => {
                    if title.is_empty() && subtitle.is_empty() {
                        players::clear_title(id);
                    } else {
                        let stay = u32::try_from(stay_ticks).unwrap_or(0);
                        let times = TitleTimes { fade_in: 5, stay, fade_out: 10 };
                        players::send_title(id, text(&title), text(&subtitle), times);
                    }
                }
                Effect::Bossbar(_, Some((t, progress))) => {
                    let shown = self
                        .with(|s| {
                            let seen = s.players.get_mut(&id)?;
                            match &seen.bar {
                                Some(bar) => {
                                    bar.set_title(&text(&t));
                                    bar.set_progress(progress);
                                }
                                None => {
                                    let bar =
                                        Bar::new(&text(&t), progress, BossbarColor::Yellow, BossbarOverlay::Progress);
                                    bar.show(id);
                                    seen.bar = Some(bar);
                                }
                            }
                            Some(())
                        })
                        .flatten();
                    if shown.is_none() {
                        log::debug("PumboAuth: bossbar not shown");
                    }
                }
                Effect::Bossbar(_, None) => {
                    let bar = self.with(|s| s.players.get_mut(&id).and_then(|p| p.bar.take())).flatten();
                    if let Some(bar) = bar {
                        bar.hide(id);
                    }
                }
                Effect::Kick(_, t) => players::kick(id, text(&t)),
                Effect::Finish(_) | Effect::Unhold(_) => {
                    // Only a player this gate holds goes on (to the next gate or a server).
                    if held {
                        self.with(|s| s.players.get_mut(&id).map(|p| p.held = false));
                        if let Err(err) = v::release(id) {
                            log::warn(&format!("PumboAuth: cannot release {uuid}: {err}"));
                        }
                    }
                }
                Effect::Admin { args, .. } => self.admin(Some(id), args),
                Effect::Hold(_) | Effect::Log(_) => {}
            }
        }
    }

    /// `/pumboauth <args>` from a player or the console.
    fn admin(&self, player: Option<PlayerId>, args: Vec<String>) {
        let nodes: Vec<String> = cmds::tree().subs().iter().map(|s| cmds::tree().permission(s)).collect();
        let granted: BTreeSet<String> = match player {
            Some(id) => nodes.into_iter().filter(|n| permissions::has(id, n, &QueryContext::Current)).collect(),
            None => nodes.into_iter().collect(),
        };
        let allowed = |n: &str| granted.contains(n);
        let sender = Sender { console: player.is_none(), allowed: &allowed };
        let out = self.with(|s| cmds::run(&mut s.auth, &sender, &args, "PumboProx", now_ms()));
        match out {
            Some(Outcome::Reply(t, fx)) => {
                reply(player, &t);
                self.apply(fx);
            }
            Some(Outcome::Reload) => {
                let result = self.reload();
                let t = self.with(|s| match &result {
                    Ok(warnings) => cmds::reloaded(&s.auth, *warnings),
                    Err(err) => {
                        style::error(&s.auth.lang, "command-reload-failed", &Args::new().arg(style::value(err)))
                    }
                });
                if let Some(t) = t {
                    reply(player, &t);
                }
            }
            Some(Outcome::Import(file)) => {
                let t = match std::fs::read_to_string(format!("{}/{file}", self.data_dir)) {
                    Ok(content) => self.with(|s| cmds::import(&s.auth, &content, now_ms())),
                    Err(e) => Some(Rich::parse(&format!("&cCannot read {file}: {e}"))),
                };
                if let Some(t) = t {
                    reply(player, &t);
                }
            }
            None => reply(player, &Rich::parse("&cPumboAuth is busy, please try again.")),
        }
    }

    fn forget(&self, id: PlayerId) {
        // The countdown bar of the player goes with its entry.
        self.with(|s| {
            let seen = s.players.remove(&id)?;
            s.by_uuid.remove(&seen.uuid);
            s.auth.leave(&seen.uuid);
            Some(())
        });
    }

    /// A player's command line (without the slash) for the core.
    fn player_command(&self, id: PlayerId, line: &str) {
        let Some(uuid) = self.with(|s| s.players.get(&id).map(|p| p.uuid.clone())).flatten() else {
            // Not seen at the gate (the plugin was loaded later): known now.
            let Some(info) = players::get(id) else { return };
            let uuid = uuid_of(&info);
            self.with(|s| {
                s.auth.adopt(&uuid, &info.profile.name, &strip_port(&info.connection.address));
                s.by_uuid.insert(uuid.clone(), id);
                s.players.insert(id, Seen { uuid: uuid.clone(), name: info.profile.name.clone(), ..Seen::default() });
            });
            return self.player_command(id, line);
        };
        // The client may have changed its language, or the gate never saw it.
        let locale = players::get(id).and_then(|i| i.settings).map(|c| c.locale);
        let mut fx = self.with(|s| s.auth.set_locale(&uuid, locale.as_deref(), now_ms())).unwrap_or_default();
        fx.extend(self.track(&uuid, |a| a.command(&uuid, line, now_ms()).1).unwrap_or_default());
        self.apply(fx);
    }
}

impl pumbo_sdk::Plugin for PumboAuth {
    async fn init(&self) -> Result<(), String> {
        let store = Store::open(format!("{}/auth.redb", self.data_dir));
        self.start(store)
    }

    async fn on_reload(&self) -> Result<(), String> {
        self.reload().map(|_| ())
    }

    async fn on_pre_login(&self, e: PreLoginEvent) -> PreLoginReply {
        let ip = strip_port(&e.connection.address);
        // The UUID is not final yet (the proxy decides online or offline after
        // this); the premium-only check happens at the gate.
        let r = self.with(|s| s.auth.pre_login(&e.name, "", &ip, None, now_ms()));
        match r {
            Some((Entry::Allow, _)) => PreLoginReply::Allow,
            Some((Entry::Refuse(t), _)) => PreLoginReply::Deny(text(&t)),
            None => PreLoginReply::Deny(Text::Legacy(BUSY.into())),
        }
    }

    async fn on_gate(&self, p: PlayerInfo) -> GateReply {
        let uuid = uuid_of(&p);
        let info = JoinInfo {
            uuid: uuid.clone(),
            name: p.profile.name.clone(),
            ip: strip_port(&p.connection.address),
            authenticated: p.online_mode,
            // The proxy reads the client's settings before the gates; none
            // (bots, ViaProxy) means the configured language.
            locale: p.settings.as_ref().map(|c| c.locale.clone()),
        };
        let seen = Seen { uuid: uuid.clone(), name: p.profile.name.clone(), held: true, ..Seen::default() };
        let ok = self.with(|s| {
            s.players.insert(p.id, seen);
            s.by_uuid.insert(uuid.clone(), p.id);
        });
        if ok.is_none() {
            return GateReply::Deny(Text::Legacy(BUSY.into()));
        }
        let fx = self.track(&uuid, |a| a.join(info, None, now_ms())).unwrap_or_default();
        if !fx.iter().any(|e| matches!(e, Effect::Hold(_))) {
            self.with(|s| s.players.get_mut(&p.id).map(|seen| seen.held = false));
            let kick = fx.iter().find_map(|e| match e {
                Effect::Kick(_, t) => Some(t.clone()),
                _ => None,
            });
            self.apply(fx);
            return match kick {
                Some(t) => GateReply::Deny(text(&t)),
                None => GateReply::Pass,
            };
        }
        if let Err(e) = enter(p.id) {
            log::warn(&format!("PumboAuth: {} cannot enter the virtual world: {e}", p.profile.name));
            self.forget(p.id);
            return GateReply::Deny(Text::Legacy(BUSY.into()));
        }
        self.apply(fx);
        GateReply::Hold
    }

    async fn on_virtual_input(&self, batch: Vec<Input>) {
        for input in batch {
            match input {
                Input::Moved((id, pos, _)) => {
                    let fx = self
                        .with(|s| {
                            let uuid = s.players.get(&id)?.uuid.clone();
                            Some(s.auth.moved(&uuid, pos.x, pos.y, pos.z))
                        })
                        .flatten()
                        .unwrap_or_default();
                    self.apply(fx);
                }
                Input::Chat((id, _)) => {
                    let fx = self
                        .with(|s| {
                            let uuid = s.players.get(&id)?.uuid.clone();
                            Some(s.auth.chat(&uuid).1)
                        })
                        .flatten()
                        .unwrap_or_default();
                    self.apply(fx);
                }
                // Never logged: the proxy hands commands of the virtual world only to us.
                Input::Command((id, line)) => self.player_command(id, &line),
                Input::Left(id) => self.forget(id),
                _ => {}
            }
        }
    }

    async fn on_disconnect(&self, p: PlayerId) {
        self.forget(p);
    }

    async fn on_timer(&self, _timer: u64) {
        let fx = self.with(|s| s.auth.tick(now_ms())).unwrap_or_default();
        self.apply(fx);
    }

    async fn on_command(&self, e: CommandEvent) {
        if let Some(sub) = e.name.strip_prefix(ADMIN) {
            let mut args = vec![sub.to_string()];
            args.extend(e.args);
            self.admin(e.player, args);
            return;
        }
        if e.name == "help" {
            let mut args = vec![e.name];
            args.extend(e.args);
            self.admin(e.player, args);
            return;
        }
        let Some(id) = e.player else {
            log::info("PumboAuth: player commands are for players");
            return;
        };
        let mut line = e.name;
        for a in &e.args {
            line.push(' ');
            line.push_str(a);
        }
        self.player_command(id, &line);
    }

    async fn on_bus_event(&self, topic: String, _major: u16, _minor: u16, _publisher: String, payload: Vec<u8>) {
        if topic != PLAYER_PUNISHED.name && topic != PLAYER_PUNISHED.versioned() {
            return;
        }
        let Ok(p) = pumbo_sdk::service::from_cbor::<Punishment>(&payload) else { return };
        if p.kind != PunishmentKind::Ban || Uuid::parse(&p.target).is_none() {
            return;
        }
        let r = self.with(|s| s.auth.accounts().map(|a| a.delete_sessions_of(&p.target)));
        match r.flatten() {
            Some(Ok(n)) if n > 0 => log::info(&format!("PumboAuth: ban #{}: {n} sessions ended", p.id)),
            Some(Err(e)) => log::warn(&format!("PumboAuth: ban #{}: sessions not ended: {e}", p.id)),
            _ => {}
        }
    }
}

pumbo_sdk::plugin!(PumboAuth);
pumbo_sdk::embed!(
    manifest = "pumbo-auth.yml",
    config = "assets/config.yml",
    lang = ["assets/lang/en.yml", "assets/lang/pl.yml"],
);

#[cfg(test)]
mod tests;
