use pumbo_common::lang::{COMMON, Lang};
use pumbo_common::store::Store;

use super::*;
use crate::config::{AuthSettings, HashingCfg};

const U: &str = "0b4f1c2a-0000-3000-8000-000000000001";
/// A version 4 UUID (Mojang account).
const P: &str = "069a79f4-44e9-4726-a5be-fca90e38aaf5";
const T0: u64 = 1_800_000_000_000;

fn settings() -> AuthSettings {
    AuthSettings {
        hashing: HashingCfg { argon2_memory_kib: 1024, argon2_iterations: 1, ..HashingCfg::default() },
        ..AuthSettings::default()
    }
}

fn auth() -> Auth {
    let lang = Lang::load(&[COMMON, crate::LANG], "en", Some("prefix: \"A » \"\n")).0;
    Auth::new(settings(), lang, Ok(Store::in_memory()))
}

fn info(name: &str, uuid: &str, authenticated: bool) -> JoinInfo {
    JoinInfo { uuid: uuid.into(), name: name.into(), ip: "1.2.3.4".into(), authenticated, locale: None }
}

fn said(fx: &[Effect]) -> Vec<String> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::Say(_, t) => Some(t.plain()),
            _ => None,
        })
        .collect()
}

fn kicked(fx: &[Effect]) -> Option<String> {
    fx.iter().find_map(|e| match e {
        Effect::Kick(_, t) => Some(t.plain()),
        _ => None,
    })
}

fn has(fx: &[Effect], f: impl Fn(&Effect) -> bool) -> bool {
    fx.iter().any(f)
}

fn finished(fx: &[Effect]) -> bool {
    has(fx, |e| matches!(e, Effect::Finish(_)))
}

/// A command at time `t` (each one later than the cooldown).
fn cmd(a: &mut Auth, line: &str, t: u64) -> Vec<Effect> {
    let (cancel, fx) = a.command(U, line, t);
    assert!(cancel, "own commands are always cancelled: {line}");
    fx
}

fn register(a: &mut Auth) {
    a.join(info("Steve", U, false), None, T0);
    let fx = cmd(a, "register hunter22 hunter22", T0 + 1000);
    assert!(finished(&fx), "{fx:?}");
}

#[test]
fn new_player_registers() {
    let mut a = auth();
    let fx = a.join(info("Steve", U, false), None, T0);
    assert!(has(&fx, |e| matches!(e, Effect::Hold(_))));
    assert!(said(&fx).iter().any(|s| s.contains("Create an account: /register <password> <password>")));
    assert!(a.is_held(U));
    // the first movement anchors the player, walking away puts them back
    assert!(a.moved(U, 1.5, 64.0, 1.5).is_empty());
    assert!(a.moved(U, 1.9, 64.0, 1.5).is_empty());
    assert_eq!(a.moved(U, 3.5, 64.0, 1.5), vec![Effect::Teleport(U.into(), (1.5, 64.0, 1.5))]);
    assert!(said(&cmd(&mut a, "register hunter22", T0 + 1000))[0].contains("Usage: /register <password> <password>"));
    assert!(said(&cmd(&mut a, "register hunter22 hunter23", T0 + 2000))[0].contains("passwords are different"));
    assert!(said(&cmd(&mut a, "login hunter22", T0 + 3000))[0].contains("no account yet"));
    assert!(said(&cmd(&mut a, "register abc abc", T0 + 4000))[0].contains("too short"));
    let fx = cmd(&mut a, "register hunter22 hunter22", T0 + 5000);
    assert!(said(&fx).iter().any(|s| s.contains("Account created. You are logged in.")));
    assert!(finished(&fx));
    assert!(has(&fx, |e| matches!(e, Effect::Bossbar(_, None))));
    assert!(!a.is_held(U), "free after registering");
    assert_eq!(a.holding(U), Hold::Free);
    let acc = a.accounts().unwrap().account("steve").unwrap().unwrap();
    assert!(acc.hash.unwrap().starts_with("$argon2id$"));
}

#[test]
fn login_replies_are_distinct() {
    let mut a = auth();
    register(&mut a);
    a.leave(U);
    // another address: no session
    let mut j = info("Steve", U, false);
    j.ip = "5.6.7.8".into();
    a.join(j, None, T0 + 10_000);
    let usage = said(&cmd(&mut a, "login", T0 + 11_000));
    let wrong = said(&cmd(&mut a, "l nothunter", T0 + 12_000));
    let register = said(&cmd(&mut a, "reg x x", T0 + 13_000));
    let fx = cmd(&mut a, "log hunter22", T0 + 14_000);
    let ok = said(&fx);
    assert_eq!(usage, vec!["A » Usage: /login <password>"]);
    assert_eq!(wrong, vec!["A » Wrong password. Attempts left: 2."]);
    assert!(register[0].contains("You already have an account. Log in: /login <password>"));
    assert_eq!(ok, vec!["A » Logged in."]);
    assert!(finished(&fx));
    assert_eq!(said(&cmd(&mut a, "login hunter22", T0 + 15_000)), vec!["A » You are already logged in."]);
}

#[test]
fn session_logs_in_on_the_same_address() {
    let mut a = auth();
    register(&mut a);
    a.leave(U);
    let fx = a.join(info("Steve", U, false), None, T0 + 60_000);
    assert!(!has(&fx, |e| matches!(e, Effect::Hold(_))));
    assert!(said(&fx)[0].contains("Logged in by your session"));
    assert!(!a.is_held(U));
    // /logout removes it
    assert!(said(&cmd(&mut a, "logout", T0 + 61_000))[0].contains("session was removed"));
    a.leave(U);
    assert!(has(&a.join(info("Steve", U, false), None, T0 + 62_000), |e| matches!(e, Effect::Hold(_))));
}

#[test]
fn attempts_and_time_run_out() {
    let mut a = auth();
    register(&mut a);
    a.leave(U);
    let mut j = info("Steve", U, false);
    j.ip = "5.6.7.8".into();
    a.join(j.clone(), None, T0 + 10_000);
    cmd(&mut a, "login a1", T0 + 11_000);
    cmd(&mut a, "login a2", T0 + 12_000);
    let fx = cmd(&mut a, "login a3", T0 + 13_000);
    assert!(kicked(&fx).unwrap().contains("Too many wrong passwords"));
    a.leave(U);
    a.join(j, None, T0 + 20_000);
    assert!(a.tick(T0 + 20_000 + 59_000).iter().all(|e| !matches!(e, Effect::Kick(..))));
    assert!(kicked(&a.tick(T0 + 20_000 + 60_000)).unwrap().contains("did not log in in time"));
}

#[test]
fn waits_for_the_filter() {
    let mut a = auth();
    let fx = a.join(info("Steve", U, false), Some(Hold::Holding), T0);
    assert!(has(&fx, |e| matches!(e, Effect::Hold(_))));
    // the gravity check moves the player: no putting back while waiting
    assert!(a.moved(U, 1.5, 1320.0, 1.5).is_empty());
    assert!(a.moved(U, 1.5, 1000.0, 1.5).is_empty());
    assert!(said(&fx).is_empty());
    assert_eq!(a.waiting(), vec![U.to_string()]);
    assert!(a.is_held(U));
    // commands and chat are cancelled silently (PumboFilter answers)
    assert_eq!(a.command(U, "login x", T0), (true, Vec::new()));
    assert_eq!(a.command(U, "spawn", T0), (true, Vec::new()));
    assert_eq!(a.chat(U), (true, Vec::new()));
    assert!(a.filter_state(U, Some(Hold::Holding), T0 + 100).is_empty());
    let fx = a.filter_state(U, Some(Hold::Free), T0 + 8000);
    assert!(said(&fx)[0].contains("Create an account"));
    assert!(a.waiting().is_empty());
}

#[test]
fn filter_unknown_then_gone() {
    let mut a = auth();
    a.join(info("Steve", U, false), Some(Hold::Unknown), T0);
    assert!(a.filter_state(U, Some(Hold::Unknown), T0 + 1000).is_empty());
    assert!(!a.filter_state(U, Some(Hold::Unknown), T0 + 3000).is_empty(), "unknown for 3 s counts as free");
    let mut a = auth();
    a.join(info("Steve", U, false), Some(Hold::Holding), T0);
    // PumboFilter was unloaded: PumboAuth goes on alone
    assert!(said(&a.filter_state(U, None, T0 + 500))[0].contains("Create an account"));
    // no filter at all: the prompt comes at once
    let mut a = auth();
    assert!(said(&a.join(info("Steve", U, false), None, T0))[0].contains("Create an account"));
}

#[test]
fn own_commands_are_always_cancelled() {
    let mut a = auth();
    register(&mut a);
    for line in ["login x", "changepassword a b", "unregister x", "2fa", "premium", "pumboauth help", "cp a b"] {
        assert!(a.command(U, line, T0 + 100_000).0, "{line}");
    }
    assert_eq!(a.command(U, "spawn", T0 + 100_000), (false, Vec::new()));
    // held players cannot use other commands or chat
    let mut b = auth();
    b.join(info("Steve", U, false), None, T0);
    let (cancel, fx) = b.command(U, "spawn", T0);
    assert!(cancel && said(&fx)[0].contains("Create an account first"));
    assert!(b.chat(U).0);
    let (cancel, fx) = b.command(U, "pumboauth forceregister Bob secret", T0);
    assert!(cancel);
    assert!(matches!(&fx[0], Effect::Admin { args, .. } if args[0] == "forceregister"));
}

#[test]
fn premium_logs_in_and_offline_cannot_use_the_nickname() {
    let mut a = auth();
    let fx = a.join(info("Notch", P, true), None, T0);
    assert!(!has(&fx, |e| matches!(e, Effect::Hold(_))));
    assert!(said(&fx)[0].contains("premium account"));
    assert!(!a.is_held(P));
    let acc = a.accounts().unwrap().account("notch").unwrap().unwrap();
    assert!(acc.premium && acc.premium_uuid.as_deref() == Some(P) && acc.hash.is_none());
    a.leave(P);
    // an offline client with this nickname: refused before it enters the world
    let offline = pumbo_common::id::Uuid::offline("Notch").to_string();
    let (entry, _) = a.pre_login("Notch", &offline, "6.6.6.6", None, T0 + 10);
    assert!(matches!(entry, Entry::Refuse(t) if t.plain().contains("premium account")));
    // and at join too (a version 4 UUID without a signed skin)
    let fx = a.join(info("Notch", P, false), None, T0 + 20);
    assert!(kicked(&fx).unwrap().contains("premium account"));
    // without trust in forwarded profiles nobody is premium
    let mut b = auth();
    b.cfg.premium.forwarded_profiles = false;
    assert!(has(&b.join(info("Notch", P, true), None, T0), |e| matches!(e, Effect::Hold(_))));
}

#[test]
fn premium_with_a_password_account_logs_in_with_the_password() {
    let mut a = auth();
    register(&mut a);
    a.leave(U);
    let fx = a.join(info("Steve", P, true), None, T0 + 10_000);
    assert!(has(&fx, |e| matches!(e, Effect::Hold(_))));
    assert!(said(&fx).iter().any(|s| s.contains("switches it to premium login")));
    let fx = a.command(P, "login hunter22", T0 + 11_000).1;
    assert!(finished(&fx));
    // /premium <password>, then /premium confirm
    assert!(said(&a.command(P, "premium hunter22", T0 + 12_000).1)[0].contains("Type /premium confirm"));
    assert!(said(&a.command(P, "premium confirm", T0 + 13_000).1)[0].contains("Premium login is on"));
    a.leave(P);
    assert!(!has(&a.join(info("Steve", P, true), None, T0 + 20_000), |e| matches!(e, Effect::Hold(_))));
}

#[test]
fn premium_conflict() {
    let mut a = auth();
    a.join(info("Notch", P, true), None, T0);
    a.leave(P);
    let other = "11111111-2222-4333-8444-555555555555";
    let fx = a.join(info("Notch", other, true), None, T0 + 10);
    assert!(kicked(&fx).unwrap().contains("taken by another account"));
}

#[test]
fn damage_protection_ends_on_every_login_path() {
    // register
    let mut a = auth();
    register(&mut a);
    assert!(!a.is_held(U));
    // password
    a.leave(U);
    let mut j = info("Steve", U, false);
    j.ip = "9.9.9.9".into();
    a.join(j.clone(), None, T0 + 10_000);
    assert!(a.is_held(U));
    a.command(U, "login hunter22", T0 + 11_000);
    assert!(!a.is_held(U));
    // session
    a.leave(U);
    a.join(j.clone(), None, T0 + 12_000);
    assert!(!a.is_held(U));
    // premium
    a.join(info("Notch", P, true), None, T0);
    assert!(!a.is_held(P));
    // forcelogin by an admin
    a.leave(U);
    j.ip = "8.8.8.8".into();
    a.join(j, None, T0 + 13_000);
    assert!(a.is_held(U));
    assert!(finished(&a.force_login(U, T0 + 14_000).unwrap()));
    assert!(!a.is_held(U));
    // after leaving nobody is protected
    a.leave(U);
    assert!(!a.is_held(U));
}

fn code_for(secret_b32: &str, now: u64) -> String {
    let secret = totp::base32_decode(secret_b32).unwrap();
    format!("{:06}", totp::totp_at(&secret, now / 1000, 6).unwrap())
}

#[test]
fn two_factor() {
    let mut a = auth();
    register(&mut a);
    let t = T0 + 100_000;
    let fx = cmd(&mut a, "2fa enable hunter22", t);
    let key = said(&fx)[0].rsplit(' ').next().unwrap().to_string();
    assert_eq!(key.len(), 32);
    assert!(said(&cmd(&mut a, "2fa confirm 000000", t + 1000))[0].contains("Wrong code"));
    let fx = cmd(&mut a, &format!("2fa confirm {}", code_for(&key, t + 31_000)), t + 31_000);
    assert!(said(&fx)[0].contains("Two-factor login is on"));
    let recovery: Vec<String> = said(&fx)[1].trim_start_matches("A » ").split_whitespace().map(String::from).collect();
    assert_eq!(recovery.len(), 8);
    // log in again: password, then the code
    a.leave(U);
    let mut j = info("Steve", U, false);
    j.ip = "9.9.9.9".into();
    a.join(j.clone(), None, t + 70_000);
    let fx = cmd(&mut a, "login hunter22", t + 71_000);
    assert!(said(&fx)[0].contains("Now enter the code"));
    assert!(!finished(&fx));
    assert!(a.is_held(U));
    let fx = cmd(&mut a, &format!("2fa {}", code_for(&key, t + 91_000)), t + 91_000);
    assert!(finished(&fx));
    // a recovery code works once, also in one go with the password
    a.leave(U);
    a.join(j.clone(), None, t + 200_000);
    a.leave(U);
    j.ip = "7.7.7.7".into();
    a.join(j, None, t + 300_000);
    assert!(finished(&cmd(&mut a, &format!("login hunter22 {}", recovery[0]), t + 301_000)));
}

#[test]
fn change_password_and_unregister() {
    let mut a = auth();
    register(&mut a);
    assert!(said(&cmd(&mut a, "changepassword wrong newpass1", T0 + 10_000))[0].contains("Wrong password."));
    assert!(said(&cmd(&mut a, "cp hunter22 newpass1", T0 + 20_000))[0].contains("Password changed"));
    let fx = cmd(&mut a, "unregister newpass1", T0 + 30_000);
    assert!(kicked(&fx).unwrap().contains("account was removed"));
    assert!(a.accounts().unwrap().account("steve").unwrap().is_none());
}

#[test]
fn second_connection_is_refused() {
    let mut a = auth();
    a.join(info("Steve", U, false), None, T0);
    let (entry, fx) = a.pre_login("Steve", U, "1.2.3.4", Some(U), T0);
    assert!(matches!(entry, Entry::Refuse(t) if t.plain().contains("Join again")));
    assert!(kicked(&fx).unwrap().contains("another place"));
    let mut a = auth();
    let (entry, fx) = a.pre_login("Steve", U, "1.2.3.4", Some(U), T0);
    assert!(matches!(entry, Entry::Refuse(t) if t.plain().contains("already on the server")));
    assert!(fx.is_empty());
}

#[test]
fn nicknames_and_case() {
    let mut a = auth();
    assert!(matches!(a.pre_login("ab", U, "1.2.3.4", None, T0).0, Entry::Refuse(_)));
    register(&mut a);
    a.leave(U);
    let (entry, _) = a.pre_login("STEVE", U, "1.2.3.4", None, T0);
    assert!(matches!(entry, Entry::Refuse(t) if t.plain().contains("registered as Steve")));
    assert_eq!(a.pre_login("Steve", U, "1.2.3.4", None, T0).0, Entry::Allow);
}

#[test]
fn unload_releases_who_is_not_logged_in() {
    let mut a = auth();
    a.join(info("Steve", U, false), None, T0);
    a.join(info("Notch", P, true), None, T0);
    let fx = a.unload();
    assert!(kicked(&fx).is_none(), "Pumpkin cannot disconnect players during the unload");
    assert_eq!(fx.iter().filter(|e| matches!(e, Effect::Unhold(_))).count(), 1);
    assert!(said(&fx)[0].contains("join the server again"));
}

#[test]
fn admin_commands() {
    use crate::commands::{Outcome, Sender, run};
    let mut a = auth();
    let all = |_: &str| true;
    let s = Sender { console: true, allowed: &all };
    let args = |l: &str| l.split_whitespace().map(String::from).collect::<Vec<_>>();
    let reply = |o: Outcome| match o {
        Outcome::Reply(t, fx) => (t.plain(), fx),
        other => (format!("{other:?}"), Vec::new()),
    };
    assert!(reply(run(&mut a, &s, &args("forceregister Bob secret123"), "test", T0)).0.contains("Bob has an account"));
    assert!(reply(run(&mut a, &s, &args("forceregister Bob secret123"), "test", T0)).0.contains("already has"));
    a.join(info("Bob", U, false), None, T0);
    let (text, fx) = reply(run(&mut a, &s, &args("forcelogin Bob"), "test", T0));
    assert!(text.contains("Bob is logged in") && finished(&fx));
    let (text, fx) = reply(run(&mut a, &s, &args("unregister Bob"), "test", T0));
    assert!(text.contains("removed") && kicked(&fx).is_some());
    assert!(reply(run(&mut a, &s, &args("forcepremium Bob"), "test", T0)).0.contains("has no account"));
    run(&mut a, &s, &args("forceregister Notch secret123"), "test", T0);
    assert!(reply(run(&mut a, &s, &args("forcepremium Notch"), "test", T0)).0.contains("premium"));
    // the next premium join claims it
    assert!(!has(&a.join(info("Notch", P, true), None, T0), |e| matches!(e, Effect::Hold(_))));
    assert!(reply(run(&mut a, &s, &args("stats"), "test", T0)).0.contains("Accounts: 1"));
    assert_eq!(run(&mut a, &s, &args("reload"), "test", T0), Outcome::Reload);
    assert_eq!(run(&mut a, &s, &args("import authme a.csv"), "test", T0), Outcome::Import("a.csv".into()));
    assert!(reply(run(&mut a, &s, &args("import authme ../x"), "test", T0)).0.contains("Invalid file name"));
    let csv = "realname,password\nAlex,$2y$05$oac08OAD6QQQAftaNoKgLeRMW9f5V/AMQ60X77jtjS9NvH5k9qgIa\nbroken\n";
    assert!(crate::commands::import(&a, csv, T0).plain().contains("Imported 1 accounts, skipped 0 existing, 1 lines"));
    assert!(reply(run(&mut a, &s, &args("version"), "test", T0)).0.contains("Platform: test"));
    assert!(reply(run(&mut a, &s, &args(""), "test", T0)).0.contains("/pumboauth forceregister <nick> <password>"));
}

#[test]
fn messages_are_complete() {
    assert_eq!(pumbo_common::lang::check_bundle(&crate::LANG), Vec::<String>::new());
}

#[test]
fn messages_in_the_language_of_the_client() {
    let mut a = auth();
    a.others = vec![Lang::load(&[COMMON, crate::LANG], "pl", Some("prefix: \"A » \"\n")).0];
    let fx = a.join(JoinInfo { locale: Some("pl_pl".into()), ..info("Steve", U, false) }, None, T0);
    assert!(said(&fx)[0].contains("Załóż konto"), "{fx:?}");
    let fx = a.tick(T0 + 1500);
    assert!(
        has(&fx, |e| matches!(e, Effect::Bossbar(_, Some((t, _))) if t.plain().contains("Czas na zalogowanie"))),
        "{fx:?}"
    );
    assert_eq!(a.lang.code(), "en");
    // Known only later: the prompt again, in that language.
    let other = "0b4f1c2a-0000-3000-8000-000000000002";
    let fx = a.join(info("Alex", other, false), None, T0);
    assert!(said(&fx)[0].contains("Create an account"), "{fx:?}");
    let fx = a.set_locale(other, Some("pl_PL"), T0 + 10);
    assert!(said(&fx)[0].contains("Załóż konto"), "{fx:?}");
    assert!(a.set_locale(other, Some("pl_pl"), T0 + 20).is_empty());
}
