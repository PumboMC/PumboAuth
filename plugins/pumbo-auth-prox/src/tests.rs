//! The plugin on the fake host of the SDK (`pumbo_sdk::testing`), virtual
//! world included.

use pumbo_sdk::testing::{self, Sent, Virtual, block_on};
use pumbo_sdk::{Connection, Plugin};

use super::*;

/// The virtual world calls so far, emptied.
fn take() -> Vec<(PlayerId, Virtual)> {
    testing::with(|h| std::mem::take(&mut h.virtual_calls))
}

const MANIFEST: &str = include_str!("../pumbo-auth.yml");
/// Cheap hashing for tests.
const FAST: &str = "hashing:\n  argon2-memory-kib: 1024\n  argon2-iterations: 1\n";

fn start(config: &str) -> PumboAuth {
    testing::reset();
    let dir = std::env::temp_dir().join(format!(
        "pumbo-auth-prox-test-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(dir.join("config.yml"), format!("{FAST}{config}")).unwrap();
    let d = dir.to_string_lossy().to_string();
    let p = PumboAuth { state: RefCell::new(None), config_dir: d.clone(), data_dir: d };
    p.start(Ok(Store::in_memory())).unwrap();
    p
}

/// A player at the gates (no server yet), the client's language known.
fn joining(name: &str, locale: &str) -> PlayerId {
    let id = testing::add_player(name, None);
    testing::with(|h| h.players.get_mut(&id).unwrap().settings = Some(testing::settings(locale)));
    id
}

/// The same account again (a new connection gets a new id).
fn again(p: &PumboAuth, old: PlayerId) -> PlayerId {
    let info = players::get(old).unwrap();
    block_on(p.on_disconnect(old));
    testing::with(|h| h.players.remove(&old));
    let id = joining(&info.profile.name, "en_us");
    testing::with(|h| h.players.get_mut(&id).unwrap().profile.id = info.profile.id);
    id
}

fn info(id: PlayerId) -> PlayerInfo {
    players::get(id).unwrap()
}

fn plain(t: &Text) -> String {
    fn walk(v: &serde_json::Value, out: &mut String) {
        if let Some(s) = v["text"].as_str() {
            out.push_str(s);
        }
        for e in v["extra"].as_array().into_iter().flatten() {
            walk(e, out);
        }
    }
    match t {
        Text::Json(s) => {
            let mut out = String::new();
            walk(&serde_json::from_str(s).unwrap(), &mut out);
            out
        }
        other => testing::render(other),
    }
}

fn messages(id: PlayerId) -> Vec<String> {
    testing::sent_to(id)
        .into_iter()
        .filter_map(|s| match s {
            Sent::Message(t) => Some(plain(&t)),
            _ => None,
        })
        .collect()
}

fn last(id: PlayerId) -> String {
    messages(id).pop().unwrap_or_default()
}

fn input(p: &PumboAuth, batch: Vec<Input>) {
    block_on(p.on_virtual_input(batch));
}

fn typed(p: &PumboAuth, id: PlayerId, line: &str) {
    input(p, vec![Input::Command((id, line.into()))]);
}

fn published(topic: &str) -> usize {
    testing::with(|h| h.published.iter().filter(|(t, _)| t.starts_with(topic)).count())
}

#[test]
fn manifest_declares_the_gate_commands_and_topics() {
    let m = pumbo_common::config::parse_yaml(MANIFEST).unwrap();
    assert_eq!(m["id"].as_str(), Some(setup::PLUGIN_ID));
    assert_eq!(m["gate"]["priority"].as_i64(), Some(200));
    assert_eq!(m["short-alias"].as_str(), Some(pumbo_auth_core::commands::ALIAS));
    let sensitive: Vec<&str> = m["sensitive-commands"].as_array().unwrap().iter().filter_map(|v| v.as_str()).collect();
    for (n, aliases) in PLAYER_COMMANDS {
        assert!(sensitive.contains(n), "{n}");
        for a in *aliases {
            assert!(sensitive.contains(a), "{a}");
        }
    }
    let nodes: Vec<&str> = m["permissions"].as_array().unwrap().iter().filter_map(|p| p["node"].as_str()).collect();
    for s in cmds::tree().subs() {
        assert!(nodes.contains(&cmds::tree().permission(s).as_str()), "{}", s.name);
    }
    assert_eq!(m["subscribes"][0].as_str(), Some("pumbo:player-punished@1.0"));
}

#[test]
fn offline_player_registers_in_the_virtual_world_and_goes_on() {
    let p = start("auth:\n  command-cooldown-ms: 0\n");
    let id = joining("Steve", "en_us");
    assert_eq!(block_on(p.on_gate(info(id))), GateReply::Hold);
    let calls = take();
    assert!(
        matches!(&calls[..], [(i, Virtual::Enter { commands, .. })] if *i == id && commands.contains(&"register".to_string())),
        "{calls:?}"
    );
    // At once: the proxy keeps the prompt and the bar until the world shows.
    assert!(messages(id).iter().any(|m| m.contains("/register")), "{:?}", messages(id));
    assert!(testing::sent_to(id).iter().any(|s| matches!(s, Sent::BarShown(_))));
    // /server is no proxy command here: it reaches PumboAuth, which refuses it.
    typed(&p, id, "server lobby");
    assert!(last(id).contains("/register"), "{}", last(id));
    typed(&p, id, "register secret77 other77");
    assert!(last(id).contains("differ"), "{}", last(id));
    typed(&p, id, "register secret77 secret77");
    assert!(take().contains(&(id, Virtual::Release)));
    assert!(testing::sent_to(id).iter().any(|s| matches!(s, Sent::BarHidden(_))));
    assert_eq!((published("pumbo:player-registered"), published("pumbo:player-authenticated")), (1, 1));
    let logged = testing::with(|h| h.placeholders.get(&("logged_in".into(), Some(id), "global".into())).cloned());
    assert_eq!(logged.map(|t| testing::render(&t)), Some("true".into()));
    // On a server: the player's commands go through the proxy to PumboAuth.
    testing::with(|h| h.players.get_mut(&id).unwrap().server = Some("lobby".into()));
    block_on(p.on_command(testing::command(id, "changepassword", &["secret77", "better88"])));
    assert!(last(id).contains("changed"), "{}", last(id));
    // Next join from the same address: the session logs in.
    let id2 = again(&p, id);
    assert_eq!(block_on(p.on_gate(info(id2))), GateReply::Pass);
    assert!(messages(id2).iter().any(|m| m.contains("session")), "{:?}", messages(id2));
}

#[test]
fn login_with_the_password_after_logout() {
    let p = start("auth:\n  command-cooldown-ms: 0\n");
    let id = joining("Alex", "en_us");
    block_on(p.on_gate(info(id)));
    input(&p, vec![Input::Loaded(id)]);
    typed(&p, id, "register secret77 secret77");
    block_on(p.on_command(testing::command(id, "logout", &[])));
    let id2 = again(&p, id);
    assert_eq!(block_on(p.on_gate(info(id2))), GateReply::Hold);
    input(&p, vec![Input::Loaded(id2)]);
    typed(&p, id2, "login wrong99");
    assert!(last(id2).contains("Attempts left") || last(id2).contains("Wrong"), "{}", last(id2));
    typed(&p, id2, "l secret77");
    assert!(take().contains(&(id2, Virtual::Release)));
}

#[test]
fn the_prompt_is_in_the_clients_language_at_once() {
    let p = start("");
    let pl = joining("Polak", "pl_pl");
    assert_eq!(block_on(p.on_gate(info(pl))), GateReply::Hold);
    assert!(messages(pl).iter().any(|m| m.contains("Załóż konto")), "{:?}", messages(pl));
    // No settings (a bot, ViaProxy): the configured language.
    let bot = joining("Bot", "pl_pl");
    testing::with(|h| h.players.get_mut(&bot).unwrap().settings = None);
    block_on(p.on_gate(info(bot)));
    assert!(messages(bot).iter().any(|m| m.contains("Create an account")), "{:?}", messages(bot));
}

#[test]
fn premium_players_need_no_password() {
    let p = start("");
    let id = joining("Notch", "pl_pl");
    testing::with(|h| h.players.get_mut(&id).unwrap().online_mode = true);
    assert_eq!(block_on(p.on_gate(info(id))), GateReply::Pass);
    assert!(take().is_empty());
    assert!(messages(id).iter().any(|m| m.contains("premium")), "{:?}", messages(id));
    let premium = testing::with(|h| h.placeholders.get(&("premium".into(), Some(id), "global".into())).cloned());
    assert_eq!(premium.map(|t| testing::render(&t)), Some("true".into()));
}

#[test]
fn a_ban_ends_the_sessions() {
    let p = start("auth:\n  command-cooldown-ms: 0\n");
    let id = joining("Eve", "en_us");
    block_on(p.on_gate(info(id)));
    input(&p, vec![Input::Loaded(id)]);
    typed(&p, id, "register secret77 secret77");
    let simple = Uuid::parse(&uuid_of(&info(id))).unwrap().simple();
    let ban =
        Punishment { id: 7, kind: PunishmentKind::Ban, target: simple, until: None, author: "x".into(), reason: None };
    let payload = pumbo_sdk::service::to_cbor(&ban).unwrap();
    block_on(p.on_bus_event("pumbo:player-punished".into(), 1, 0, "pumbo-bans".into(), payload));
    let id2 = again(&p, id);
    assert_eq!(block_on(p.on_gate(info(id2))), GateReply::Hold, "no session after the ban");
}

#[test]
fn admin_commands_and_refusals() {
    let p = start("nickname:\n  blocked: [admin*]\n");
    let mut e = testing::command(0, "admin-forceregister", &["Bob", "secret77"]);
    e.player = None;
    block_on(p.on_command(e));
    let logs = testing::with(|h| h.logs.clone());
    assert!(logs.iter().any(|(_, l)| l.contains("Bob")), "{logs:?}");
    assert!(!logs.iter().any(|(_, l)| l.contains("secret77")), "{logs:?}");
    let names: Vec<String> = testing::with(|h| h.commands.iter().map(|c| c.name.clone()).collect());
    assert!(names.contains(&"admin-unregister".to_string()) && names.contains(&"unregister".to_string()));
    let login = testing::with(|h| h.commands.iter().find(|c| c.name == "login").cloned()).unwrap();
    assert!(login.sensitive && login.virtual_only);
    let conn = info(joining("X", "en_us")).connection;
    let e = PreLoginEvent { connection: Connection { ..conn }, name: "admin1".into(), claimed_uuid: None };
    assert!(matches!(block_on(p.on_pre_login(e)), PreLoginReply::Deny(_)));
}
