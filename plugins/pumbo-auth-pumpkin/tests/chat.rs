//! What PumboAuth shows on Pumpkin, kept in `tests/snapshots/chat.txt`: the
//! replies to `/pumboauth`, what a player sees while logging in (chat,
//! titles, the countdown bar), two-factor login, the premium question and the
//! console. A screen starts with `== <screen> <who>: <command or event>`;
//! every chat message is one line of its JSON text component, other places
//! say what they are (`title:`, `bossbar:`, `kick:`), and the console gets its
//! plain text indented. The screens carry the numbers of the chat UI mockups
//! (A1-A7, S2, S4). Random keys and codes are replaced by fixed ones.
//! `PUMBO_SNAPSHOT_WRITE=1` writes the file anew.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::fmt::Write as _;
use std::path::PathBuf;

use pumbo_auth_core::commands::{self, Outcome, Sender};
use pumbo_auth_core::config::{AuthSettings, HashingCfg};
use pumbo_auth_core::flow::{Auth, Effect, JoinInfo};
use pumbo_auth_core::totp;
use pumbo_auth_pumpkin::BUNDLES;
use pumbo_common::lang::Lang;
use pumbo_common::rich::{self, Click, Text};
use pumbo_common::store::Store;

/// An offline UUID and a version 4 (premium) one.
const STEVE: &str = "0b4f1c2a-0000-3000-8000-000000000001";
const PREMIUM: &str = "069a79f4-44e9-4726-a5be-fca90e38aaf5";
const T0: u64 = 1_800_000_000_000;
const PLATFORM: &str = "Pumpkin 0.2.2 (MC 26.3)";
const KEY: &str = "JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP";
const CODES: [&str; 8] =
    ["4k7q-m2xd", "9tbn-h3lp", "w6rc-8yfa", "a2b3-c4d5", "e6f7-g8h9", "j2k3-m4n5", "p6q7-r8s9", "t2u3-v4w5"];

#[derive(Default)]
struct Snapshot {
    out: String,
    /// Random text and what stands in its place.
    masks: Vec<(String, String)>,
}

impl Snapshot {
    fn screen(&mut self, screen: &str, who: &str, what: &str) {
        writeln!(self.out, "\n== {screen} {who}: {what}").unwrap();
    }

    fn chat(&mut self, t: &Text) {
        writeln!(self.out, "{}", rich::json(t)).unwrap();
    }

    fn other(&mut self, place: &str, t: &Text) {
        writeln!(self.out, "{place}: {}", rich::json(t)).unwrap();
    }

    fn console(&mut self, t: &Text) {
        for line in t.plain().lines() {
            writeln!(self.out, "  {line}").unwrap();
        }
    }

    /// What a player sees of the effects (holds and moves are left out).
    fn effects(&mut self, fx: &[Effect]) {
        for e in fx {
            match e {
                Effect::Say(_, t) => self.chat(t),
                Effect::Title { title, subtitle, .. } => {
                    self.other("title", title);
                    self.other("subtitle", subtitle);
                }
                Effect::Bossbar(_, Some((t, _))) => self.other("bossbar", t),
                Effect::Kick(_, t) => self.other("kick", t),
                _ => {}
            }
        }
    }

    fn finish(mut self) -> String {
        for (from, to) in &self.masks {
            self.out = self.out.replace(from.as_str(), to);
        }
        self.out
    }
}

fn settings() -> AuthSettings {
    let mut cfg = AuthSettings {
        hashing: HashingCfg { argon2_memory_kib: 1024, argon2_iterations: 1, ..HashingCfg::default() },
        ..AuthSettings::default()
    };
    cfg.two_factor.enabled = true;
    cfg
}

fn lang(code: &str, file: Option<&str>) -> Lang {
    let (lang, warnings) = Lang::load(&BUNDLES, code, file);
    assert!(warnings.is_empty(), "{warnings:?}");
    lang
}

fn auth(code: &str, file: Option<&str>) -> Auth {
    Auth::new(settings(), lang(code, file), Ok(Store::in_memory()))
}

fn join(name: &str, uuid: &str, premium: bool) -> JoinInfo {
    JoinInfo { uuid: uuid.into(), name: name.into(), ip: "203.0.113.7".into(), authenticated: premium, locale: None }
}

fn run(a: &mut Auth, console: bool, allowed: &dyn Fn(&str) -> bool, line: &str) -> Text {
    let args: Vec<String> = line.split_whitespace().map(String::from).collect();
    let sender = Sender { console, allowed };
    match commands::run(a, &sender, &args, PLATFORM, T0) {
        Outcome::Reply(t, _) => t,
        Outcome::Reload => commands::reloaded(a, 0),
        Outcome::Import(file) => Text::parse(&format!("import {file}")),
    }
}

/// The admin screens of `/pumboauth`.
fn admin(s: &mut Snapshot) {
    let all = |_: &str| true;
    let none = |_: &str| false;
    let mut a = auth("en", None);
    s.screen("A1", "admin", "/pumboauth help");
    s.chat(&run(&mut a, false, &all, "help"));
    s.screen("A1", "console", "pumboauth help");
    s.console(&run(&mut a, true, &all, "help"));

    for line in [
        "forceregister Steve hunter22",
        "forceregister Steve hunter22",
        "forcechangepassword Steve hunter23",
        "forcelogin Alex",
        "forcepremium Steve",
        "forceoffline Steve",
        "reset2fa Steve",
        "unblock 203.0.113.7",
        "logout Steve",
        "unregister Steve",
        "unregister Steve",
        "forceregister",
        "import nothing x",
        "unregisterr",
        "version",
    ] {
        s.screen("S2", "admin", &format!("/pumboauth {line}"));
        s.chat(&run(&mut a, false, &all, line));
    }
    s.screen("S2", "admin", "reload with 2 warnings");
    s.chat(&commands::reloaded(&a, 2));
    s.screen("S2", "player", "/pumboauth stats");
    s.chat(&run(&mut a, false, &none, "stats"));

    a.stats.logins = 5_310;
    a.stats.registrations = 48;
    a.stats.sessions = 2_100;
    a.stats.premium = 911;
    a.stats.wrong = 37;
    s.screen("A7", "admin", "/pumboauth stats");
    s.chat(&run(&mut a, false, &all, "stats"));
    s.screen("A7", "console", "pumboauth stats");
    s.console(&run(&mut a, true, &all, "stats"));
}

/// A new player creates an account; a known one logs in and gets a password wrong.
fn login(s: &mut Snapshot, code: &str) {
    let who = format!("player ({code})");
    let mut a = auth(code, None);
    s.screen("A2", &who, "join without an account");
    s.effects(&a.join(join("Steve", STEVE, false), None, T0));
    s.screen("A2", &who, "/register hunter22 hunter22");
    s.effects(&a.command(STEVE, "register hunter22 hunter22", T0 + 1_000).1);
    a.leave(STEVE);
    let mut again = join("Steve", STEVE, false);
    again.ip = "198.51.100.20".into();
    s.screen("A3", &who, "join with an account");
    s.effects(&a.join(again, None, T0 + 60_000));
    s.screen("A2", &who, "/login wrong");
    s.effects(&a.command(STEVE, "login wrong", T0 + 61_000).1);
    s.screen("A2", &who, "/login hunter22");
    s.effects(&a.command(STEVE, "login hunter22", T0 + 62_000).1);
}

/// Two-factor login: the key, the code, the recovery codes.
fn two_factor(s: &mut Snapshot) {
    let mut a = auth("en", None);
    a.join(join("Steve", STEVE, false), None, T0);
    a.command(STEVE, "register hunter22 hunter22", T0 + 1_000);
    let t = T0 + 100_000;
    s.screen("A4", "player", "/2fa enable hunter22");
    let fx = a.command(STEVE, "2fa enable hunter22", t).1;
    let url = fx
        .iter()
        .find_map(|e| match e {
            Effect::Say(_, t) => t.lines.iter().flat_map(|l| &l.segments).find_map(|s| match &s.click {
                Some(Click::Copy(u)) => Some(u.clone()),
                _ => None,
            }),
            _ => None,
        })
        .unwrap();
    let secret = url.split("secret=").nth(1).unwrap().split('&').next().unwrap().to_string();
    s.masks.push((secret.clone(), KEY.into()));
    s.effects(&fx);
    let raw = totp::base32_decode(&secret).unwrap();
    let now = t + 31_000;
    let code = format!("{:06}", totp::totp_at(&raw, now / 1000, 6).unwrap());
    s.screen("A5", "player", "/2fa confirm <code>");
    let fx = a.command(STEVE, &format!("2fa confirm {code}"), now).1;
    let codes = fx.iter().filter_map(|e| match e {
        Effect::Say(_, t) => Some(t.plain()),
        _ => None,
    });
    let found: Vec<String> = codes
        .flat_map(|p| p.split_whitespace().map(String::from).collect::<Vec<_>>())
        .filter(|w| w.len() == 9 && w.as_bytes()[4] == b'-')
        .collect();
    assert_eq!(found.len(), CODES.len(), "{found:?}");
    for (from, to) in found.into_iter().zip(CODES) {
        s.masks.push((from, to.into()));
    }
    s.effects(&fx);
}

/// A premium account that still has a password: the question and the answer.
fn premium(s: &mut Snapshot) {
    let mut a = auth("en", None);
    a.join(join("Steve", STEVE, false), None, T0);
    a.command(STEVE, "register hunter22 hunter22", T0 + 1_000);
    a.leave(STEVE);
    s.screen("A6", "player", "premium join with a password account");
    s.effects(&a.join(join("Steve", PREMIUM, true), None, T0 + 10_000));
    a.command(PREMIUM, "login hunter22", T0 + 11_000);
    s.screen("A6", "player", "/premium hunter22");
    s.effects(&a.command(PREMIUM, "premium hunter22", T0 + 12_000).1);
    s.screen("A6", "player", "/premium confirm");
    s.effects(&a.command(PREMIUM, "premium confirm", T0 + 13_000).1);
}

/// S4: the server's own prefix in front of what a player sees.
fn own_prefix(s: &mut Snapshot) {
    let mut a = auth("en", Some("prefix: \"&#7cc0ffSkyBlock &8» \"\n"));
    a.join(join("Steve", STEVE, false), None, T0);
    a.command(STEVE, "register hunter22 hunter22", T0 + 1_000);
    a.leave(STEVE);
    s.screen("S4", "player", "join, wrong password, login with prefix: SkyBlock");
    let mut again = join("Steve", STEVE, false);
    again.ip = "198.51.100.20".into();
    let mut fx = a.join(again, None, T0 + 60_000);
    fx.extend(a.command(STEVE, "login wrong", T0 + 61_000).1);
    fx.extend(a.command(STEVE, "login hunter22", T0 + 62_000).1);
    s.effects(&fx);
}

#[test]
fn chat_screens() {
    let mut s = Snapshot::default();
    s.out.push_str("PumboAuth on Pumpkin: chat screens. Written by tests/chat.rs (PUMBO_SNAPSHOT_WRITE=1).\n");
    admin(&mut s);
    for code in ["en", "pl"] {
        login(&mut s, code);
    }
    two_factor(&mut s);
    premium(&mut s);
    own_prefix(&mut s);
    let out = s.finish();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/chat.txt");
    if std::env::var_os("PUMBO_SNAPSHOT_WRITE").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &out).unwrap();
        return;
    }
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    if old != out {
        let first = old.lines().zip(out.lines()).position(|(a, b)| a != b).unwrap_or(0);
        panic!(
            "tests/snapshots/chat.txt differs from line {} on (PUMBO_SNAPSHOT_WRITE=1 writes it anew):\n- {}\n+ {}",
            first + 1,
            old.lines().nth(first).unwrap_or(""),
            out.lines().nth(first).unwrap_or("")
        );
    }
}
