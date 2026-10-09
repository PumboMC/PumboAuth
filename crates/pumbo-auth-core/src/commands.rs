//! `/pumboauth` (alias `/auth`) and its subcommands (permissions
//! `pumbo.auth.<action>`).

use pumbo_common::command::{Commands, Dispatch, Sub};
use pumbo_common::help::{Help, page_arg};
use pumbo_common::rich::{Line, Text};
use pumbo_common::style;
use pumbo_common::text::Args;

use crate::ID;
use crate::flow::{Auth, Effect};
use crate::hashing;
use crate::import::parse_authme_csv;
use crate::rules::check_password;
use crate::store::{Account, Counter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Reload,
    Stats,
    ForceRegister,
    ForceChangePassword,
    Unregister,
    ForceLogin,
    ForcePremium,
    ForceOffline,
    Reset2fa,
    Unblock,
    Logout,
    Import,
    Version,
}

/// The command shown in usage lines and the help.
pub const LABEL: &str = "/pumboauth";
/// Short command for [`LABEL`], on PumboProx (`short-alias`) and Pumpkin.
pub const ALIAS: &str = "pa";

pub fn tree() -> Commands<Action> {
    let sub = |name, action: Action, usage, min| {
        Sub::new(name, name, action).usage(usage, min).description(help_key(name)).details(details_key(name))
    };
    Commands::new(ID)
        .label(LABEL)
        .short_alias(format!("/{ALIAS}"))
        .with(Sub::new("reload", "reload", Action::Reload).description("help-reload"))
        .with(Sub::new("stats", "stats", Action::Stats).description("help-stats"))
        .with(sub("forceregister", Action::ForceRegister, "<nick> <password>", 2))
        .with(sub("forcechangepassword", Action::ForceChangePassword, "<nick> <password>", 2))
        .with(sub("unregister", Action::Unregister, "<nick>", 1))
        .with(sub("forcelogin", Action::ForceLogin, "<nick>", 1))
        .with(sub("forcepremium", Action::ForcePremium, "<nick>", 1))
        .with(sub("forceoffline", Action::ForceOffline, "<nick>", 1))
        .with(sub("reset2fa", Action::Reset2fa, "<nick>", 1))
        .with(sub("unblock", Action::Unblock, "<ip|nick>", 1))
        .with(sub("logout", Action::Logout, "<nick>", 1))
        .with(sub("import", Action::Import, "authme <file>", 2))
        .with(Sub::new("version", "version", Action::Version).description("version-description"))
}

fn help_key(name: &str) -> &'static str {
    match name {
        "forceregister" => "help-forceregister",
        "forcechangepassword" => "help-forcechangepassword",
        "unregister" => "help-unregister",
        "forcelogin" => "help-forcelogin",
        "forcepremium" => "help-forcepremium",
        "forceoffline" => "help-forceoffline",
        "reset2fa" => "help-reset2fa",
        "unblock" => "help-unblock",
        "logout" => "help-logout",
        _ => "help-import",
    }
}

fn details_key(name: &str) -> &'static str {
    match name {
        "forceregister" => "help-forceregister-details",
        "forcechangepassword" => "help-forcechangepassword-details",
        "unregister" => "help-unregister-details",
        "forcelogin" => "help-forcelogin-details",
        "forcepremium" => "help-forcepremium-details",
        "forceoffline" => "help-forceoffline-details",
        "reset2fa" => "help-reset2fa-details",
        "unblock" => "help-unblock-details",
        "logout" => "help-logout-details",
        _ => "help-import-details",
    }
}

/// Who runs a command.
pub struct Sender<'a> {
    pub console: bool,
    pub allowed: &'a dyn Fn(&str) -> bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Reply(Text, Vec<Effect>),
    /// Read the config and messages again, then reply with [`reloaded`].
    Reload,
    /// Read this file from the data folder and pass it to [`import`].
    Import(String),
}

pub fn reloaded(auth: &Auth, warnings: usize) -> Text {
    if warnings == 0 {
        style::success(&auth.lang, "command-reloaded", &Args::new())
    } else {
        style::warn(&auth.lang, "reloaded-warnings", &Args::new().arg(style::value(warnings)))
    }
}

fn help(auth: &Auth, sender: &Sender, page: usize) -> Text {
    let help = Help::from_commands("PumboAuth", &tree(), &auth.lang)
        .version(env!("CARGO_PKG_VERSION"))
        .section(auth.lang.get("help-section"));
    if sender.console { help.console(&auth.lang, sender.allowed) } else { help.chat(&auth.lang, page, sender.allowed) }
}

fn ok(auth: &Auth, key: &str, args: &Args) -> Outcome {
    Outcome::Reply(style::success(&auth.lang, key, args), Vec::new())
}

fn fail(auth: &Auth, key: &str, args: &Args) -> Outcome {
    Outcome::Reply(style::error(&auth.lang, key, args), Vec::new())
}

fn db_error(auth: &Auth, e: impl std::fmt::Display) -> Outcome {
    fail(auth, "database-error", &Args::new().arg(style::value(e)))
}

/// Runs `/pumboauth <args>`. `platform` is shown by `version`.
pub fn run(auth: &mut Auth, sender: &Sender, args: &[String], platform: &str, now: u64) -> Outcome {
    let tree = tree();
    let lang = auth.lang.clone();
    match tree.dispatch(args, sender.allowed) {
        Dispatch::Help => Outcome::Reply(help(auth, sender, page_arg(args.get(1..).unwrap_or(&[]))), Vec::new()),
        Dispatch::Unknown { name } => {
            Outcome::Reply(style::unknown_subcommand(&lang, &name, &tree.help_line()), Vec::new())
        }
        Dispatch::NoPermission { .. } => fail(auth, "command-no-permission", &Args::new()),
        Dispatch::Usage { usage } => Outcome::Reply(style::usage(&lang, &usage), Vec::new()),
        Dispatch::Run { sub, args } => {
            let nick = args.first().cloned().unwrap_or_default();
            let v = style::value(&nick);
            match sub.handler {
                Action::Reload => Outcome::Reload,
                Action::Version => Outcome::Reply(
                    style::version(
                        "PumboAuth",
                        env!("CARGO_PKG_VERSION"),
                        [(lang.get("version-platform"), platform.to_string())],
                    ),
                    Vec::new(),
                ),
                Action::Stats => Outcome::Reply(stats(auth), Vec::new()),
                Action::Import => {
                    if !auth.cfg.import.enabled {
                        return fail(auth, "admin-import-disabled", &Args::new());
                    }
                    if !nick.eq_ignore_ascii_case("authme") {
                        return Outcome::Reply(style::usage(&lang, &tree.usage(sub)), Vec::new());
                    }
                    let file = args.get(1).cloned().unwrap_or_default();
                    if file.is_empty() || file.contains("..") || file.starts_with('/') {
                        return fail(auth, "admin-file-name", &Args::new().arg(style::value(&file)));
                    }
                    Outcome::Import(file)
                }
                Action::ForceRegister | Action::ForceChangePassword => {
                    let password = args.get(1).cloned().unwrap_or_default();
                    if let Err(p) = check_password(&password, &nick, &auth.cfg.auth) {
                        let why = match p {
                            crate::rules::PasswordProblem::TooShort(n) => {
                                lang.format("auth-password-too-short", &Args::new().arg(n))
                            }
                            crate::rules::PasswordProblem::TooLong(n) => {
                                lang.format("auth-password-too-long", &Args::new().arg(n))
                            }
                            crate::rules::PasswordProblem::Common => lang.get("auth-password-unsafe"),
                            crate::rules::PasswordProblem::SameAsName => lang.get("auth-password-name"),
                        };
                        return Outcome::Reply(style::toned(&lang, style::Tone::Error, &why), Vec::new());
                    }
                    let Ok(hash) = hashing::hash(&password, auth.cfg.hashing.argon2()) else {
                        return fail(auth, "command-error", &Args::new());
                    };
                    let Some(store) = auth.accounts() else { return db_error(auth, "no database") };
                    let existing = match store.account(&nick) {
                        Ok(a) => a,
                        Err(e) => return db_error(auth, e),
                    };
                    let acc = match (sub.handler, existing) {
                        (Action::ForceRegister, Some(_)) => return fail(auth, "admin-exists", &Args::new().arg(v)),
                        (Action::ForceRegister, None) => Account {
                            name: nick.clone(),
                            hash: Some(hash),
                            reg_time: now,
                            password_changed: now,
                            ..Account::default()
                        },
                        (_, None) => return fail(auth, "admin-no-account", &Args::new().arg(v)),
                        (_, Some(mut acc)) => {
                            // A password makes the account a normal one again.
                            acc.hash = Some(hash);
                            acc.premium = false;
                            acc.premium_uuid = None;
                            acc.password_changed = now;
                            acc
                        }
                    };
                    let saved = if sub.handler == Action::ForceRegister {
                        store.put_account(&acc)
                    } else {
                        store.delete_account(&acc.name).and_then(|_| store.put_account(&acc))
                    };
                    if let Err(e) = saved {
                        return db_error(auth, e);
                    }
                    let key =
                        if sub.handler == Action::ForceRegister { "admin-registered" } else { "admin-password-set" };
                    ok(auth, key, &Args::new().arg(v))
                }
                Action::Unregister => {
                    let Some(store) = auth.accounts() else { return db_error(auth, "no database") };
                    match store.delete_account(&nick) {
                        Ok(true) => {
                            let fx = auth
                                .online_uuid(&nick)
                                .map(|u| vec![Effect::Kick(u, Text::parse(&lang.get("kick-unregistered")))])
                                .unwrap_or_default();
                            Outcome::Reply(style::success(&lang, "admin-unregistered", &Args::new().arg(v)), fx)
                        }
                        Ok(false) => fail(auth, "admin-no-account", &Args::new().arg(v)),
                        Err(e) => db_error(auth, e),
                    }
                }
                Action::ForceLogin => {
                    let fx = auth.online_uuid(&nick).and_then(|u| auth.force_login(&u, now));
                    match fx {
                        Some(fx) => Outcome::Reply(style::success(&lang, "admin-forcelogin", &Args::new().arg(v)), fx),
                        None => fail(auth, "admin-not-waiting", &Args::new().arg(v)),
                    }
                }
                Action::ForcePremium | Action::ForceOffline | Action::Reset2fa => {
                    let Some(store) = auth.accounts() else { return db_error(auth, "no database") };
                    let mut acc = match store.account(&nick) {
                        Ok(Some(a)) => a,
                        Ok(None) => return fail(auth, "admin-no-account", &Args::new().arg(v)),
                        Err(e) => return db_error(auth, e),
                    };
                    let key = match sub.handler {
                        Action::ForcePremium => {
                            // The next premium join with this nickname claims the account.
                            acc.premium = true;
                            acc.premium_uuid = None;
                            acc.hash = None;
                            acc.totp_secret = None;
                            acc.recovery.clear();
                            "admin-premium-set"
                        }
                        Action::ForceOffline => {
                            acc.premium = false;
                            acc.premium_uuid = None;
                            "admin-offline-set"
                        }
                        _ => {
                            acc.totp_secret = None;
                            acc.totp_last_step = None;
                            acc.recovery.clear();
                            "admin-2fa-reset"
                        }
                    };
                    // Removing first also drops the old premium index entry.
                    if let Err(e) = store.delete_account(&acc.name).and_then(|_| store.put_account(&acc)) {
                        return db_error(auth, e);
                    }
                    let cmd = style::command(format!("{LABEL} forcechangepassword {nick} <password>"));
                    ok(auth, key, &Args::new().arg(v).arg(cmd))
                }
                Action::Unblock => {
                    let Some(store) = auth.accounts() else { return db_error(auth, "no database") };
                    let cleared = store
                        .clear_failures(Counter::Ip, &nick)
                        .and_then(|_| store.clear_failures(Counter::Account, &nick));
                    match cleared {
                        Ok(()) => ok(auth, "admin-unblocked", &Args::new().arg(v)),
                        Err(e) => db_error(auth, e),
                    }
                }
                Action::Logout => {
                    let Some(store) = auth.accounts() else { return db_error(auth, "no database") };
                    match store.delete_sessions(&nick) {
                        Ok(()) => ok(auth, "admin-logged-out", &Args::new().arg(v)),
                        Err(e) => db_error(auth, e),
                    }
                }
            }
        }
    }
}

/// `import authme <file>` with the file's text.
pub fn import(auth: &Auth, text: &str, now: u64) -> Text {
    let parsed = parse_authme_csv(text);
    let accounts: Vec<Account> = parsed
        .rows
        .iter()
        .map(|r| Account { name: r.name.clone(), hash: Some(r.hash.clone()), reg_time: now, ..Account::default() })
        .collect();
    let Some(store) = auth.accounts() else {
        return style::error(&auth.lang, "database-error", &Args::new().arg("no database"));
    };
    match store.import_accounts(&accounts) {
        Ok((added, skipped)) => style::success(
            &auth.lang,
            "admin-imported",
            &Args::new().arg(style::value(added)).arg(style::value(skipped)).arg(style::value(parsed.invalid)),
        ),
        Err(e) => style::error(&auth.lang, "database-error", &Args::new().arg(style::value(e))),
    }
}

fn stats(auth: &Auth) -> Text {
    let lang = &auth.lang;
    let s = auth.stats;
    let n = |v: u64| style::value(style::number(lang, v));
    let accounts = auth.accounts().and_then(|st| st.count_accounts().ok()).unwrap_or(0);
    let mut text = style::info(
        lang,
        "stats-online",
        &Args::new().arg(n(auth.held().len() as u64)).arg(n(auth.online_count() as u64)).arg(n(accounts)),
    );
    let db = match &auth.store_error {
        None => lang.get("stats-ok"),
        Some(e) => e.clone(),
    };
    for l in [
        lang.format(
            "stats-logins",
            &Args::new().arg(n(s.logins)).arg(n(s.registrations)).arg(n(s.sessions)).arg(n(s.premium)).arg(n(s.wrong)),
        ),
        lang.format("stats-database", &Args::new().arg(style::value(db))),
    ] {
        text.push(Line::parse(&format!("{}{l}", style::code(style::INFO))));
    }
    text
}
