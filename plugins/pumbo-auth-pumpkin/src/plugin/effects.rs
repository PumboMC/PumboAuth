//! Carrying out the core's effects and the release rule.

use pumbo_auth_core::commands::{self, Outcome, Sender};
use pumbo_auth_core::flow::Effect;
use pumbo_common::clock::now_ms;
use pumbo_common::gate::{self, Hold};
use pumbo_common::rich::Text;

use super::api::{self, BossBar, BossBarColor, BossBarDivision, Player, Server};
use super::state;
use crate::{PARTNER, pumpkin_node};

/// Runs `f` on the player an effect is for: the event's own player first (in
/// the leave handler the server no longer finds them by UUID), then the server.
fn with_player(server: &Server, current: Option<&Player>, uuid: &str, f: impl FnOnce(&Player)) {
    if let Some(p) = current
        && api::uuid_of(p) == uuid
    {
        f(p);
    } else if let Some(p) = api::api_uuid(uuid).and_then(|u| server.get_player_by_uuid(u)) {
        f(&p);
    }
}

/// What PumboFilter says about a player; `None` when it is not there.
pub fn filter_holding(uuid: &str) -> Option<Hold> {
    match api::ipc::send_ipc_message(PARTNER, &gate::holding_request(uuid)) {
        Ok(Ok(answer)) => Some(gate::parse_holding(&answer)),
        _ => None,
    }
}

pub fn apply(server: &Server, current: Option<&Player>, effects: Vec<Effect>) {
    for e in effects {
        match e {
            Effect::Log(line) => api::info(&format!("PumboAuth: {line}")),
            Effect::Hold(uuid) => with_player(server, current, &uuid, |p| api::hold(server, p)),
            Effect::Finish(uuid) => {
                // The core marked the player free before this; release them
                // unless PumboFilter still holds them (then it releases).
                if filter_holding(&uuid) != Some(Hold::Holding) {
                    with_player(server, current, &uuid, |p| api::release(server, p));
                }
            }
            Effect::Unhold(uuid) => with_player(server, current, &uuid, |p| api::release(server, p)),
            Effect::Bossbar(uuid, bar) => bossbar(server, &uuid, bar),
            Effect::Admin { uuid, args } => {
                let mut reply = Text::default();
                with_player(server, current, &uuid, |p| reply = admin(server, Some(p), &args));
                with_player(server, current, &uuid, |p| api::say(p, &reply));
            }
            Effect::Teleport(uuid, (x, y, z)) => {
                with_player(server, current, &uuid, |p| p.teleport((x, y, z), None, None, p.get_world()));
            }
            Effect::Say(uuid, t) => with_player(server, current, &uuid, |p| api::say(p, &t)),
            Effect::Title { uuid, title, subtitle, stay_ticks } => with_player(server, current, &uuid, |p| {
                p.send_title_animation(5, stay_ticks, 10);
                p.show_subtitle(api::component(&subtitle));
                p.show_title(api::component(&title));
            }),
            Effect::Kick(uuid, t) => with_player(server, current, &uuid, |p| api::kick(p, &t)),
        }
    }
}

fn bossbar(server: &Server, uuid: &str, bar: Option<(Text, f32)>) {
    match bar {
        None => {
            if let Some(Some(old)) = state::with_bars(|bars| bars.remove(uuid)) {
                old.remove_all();
            }
        }
        Some((title, progress)) => {
            let existing = state::with_bars(|bars| {
                bars.get(uuid).map(|b| {
                    b.set_title(api::component(&title));
                    b.set_health(progress);
                })
            })
            .flatten();
            if existing.is_none() {
                let b = BossBar::new(api::component(&title), BossBarColor::Yellow, BossBarDivision::NoDivision);
                b.set_health(progress);
                if let Some(p) = api::api_uuid(uuid).and_then(|u| server.get_player_by_uuid(u)) {
                    b.add_player(p);
                }
                state::with_bars(|bars| bars.insert(uuid.to_string(), b));
            }
        }
    }
}

/// Runs `/pumboauth <args>` for a player (permissions read from the host
/// first) or the console, and returns the reply.
pub fn admin(server: &Server, player: Option<&Player>, args: &[String]) -> Text {
    let nodes: Vec<(String, bool)> = commands::tree()
        .subs()
        .iter()
        .map(|s| {
            let node = commands::tree().permission(s);
            let ok = player.is_none_or(|p| p.has_permission(&pumpkin_node(&node)));
            (node, ok)
        })
        .collect();
    let allowed = |n: &str| nodes.iter().any(|(node, ok)| node == n && *ok);
    let sender = Sender { console: player.is_none(), allowed: &allowed };
    let now = now_ms();
    let out = state::with(|rt| match commands::run(&mut rt.auth, &sender, args, api::PLATFORM, now) {
        Outcome::Reply(t, fx) => (t, fx),
        Outcome::Reload => (super::reload(rt), Vec::new()),
        Outcome::Import(file) => {
            let path = format!("{}/{file}", rt.dir);
            match std::fs::read_to_string(&path) {
                Ok(text) => (commands::import(&rt.auth, &text, now), Vec::new()),
                Err(e) => (
                    pumbo_common::style::error(
                        &rt.auth.lang,
                        "admin-file-error",
                        &pumbo_common::text::Args::new()
                            .arg(pumbo_common::style::value(&file))
                            .arg(pumbo_common::style::value(e)),
                    ),
                    Vec::new(),
                ),
            }
        }
    });
    let Some((reply, fx)) = out else { return Text::parse("&cPumboAuth is busy, try again.") };
    apply(server, player, fx);
    reply
}
