//! The parts of the Pumpkin plugin API this plugin uses (the same in Pumpkin
//! 0.2.0 and 0.1.0-dev), text conversion, and the hold: a dark screen on the
//! client only and hidden from other players. Nothing of it is written to the
//! player's data file.

pub use crate::papi::boss_bar::{BossBar, BossBarColor, BossBarDivision};
pub use crate::papi::command::{
    Arg, ArgumentType, Command, CommandError, CommandNode, CommandSender, ConsumedArgs, StringType,
};
pub use crate::papi::commands::CommandHandler;
pub use crate::papi::common::{NamedColor, RgbColor};
pub use crate::papi::events::{
    AsyncPlayerPreLoginEvent, BlockBreakEvent, BlockPlaceEvent, EntityDamageEvent, EventData, EventHandler,
    EventPriority, InventoryClickEvent, InventoryCreativeEvent, PlayerChatEvent, PlayerCommandSendEvent,
    PlayerDropItemEvent, PlayerInteractEntityEvent, PlayerInteractEvent, PlayerItemConsumeEvent, PlayerItemHeldEvent,
    PlayerJoinEvent, PlayerLeaveEvent, PlayerMoveEvent, PlayerSwapHandsEvent,
};
pub use crate::papi::java_packets::{CRemoveMobEffect, CUpdateMobEffect, ClientboundPacket};
pub use crate::papi::logging::{LogLevel, log};
pub use crate::papi::permission::{Permission, PermissionDefault, PermissionLevel};
pub use crate::papi::player::{BedrockDisconnectReason, BedrockKickOptions, JavaKickOptions, SocketTeardownPolicy};
pub use crate::papi::scheduler::SchedulerExt;
pub use crate::papi::text::TextComponent;
pub use crate::papi::{Context, Player, Plugin, PluginMetadata, Server, ipc, permissions};

use pumbo_common::id::Uuid;
use pumbo_common::rich::{Click, Segment, Text};
use pumbo_common::text::{Color, Named};

#[cfg(feature = "mc263")]
pub const PLATFORM: &str = "Pumpkin 0.2.0 (MC 26.3)";
#[cfg(feature = "mc262")]
pub const PLATFORM: &str = "Pumpkin 0.1.0-dev (MC 26.2)";

/// Registry id of `minecraft:blindness` (the same in 26.2 and 26.3).
const BLINDNESS: i32 = 14;

pub fn info(msg: &str) {
    log(LogLevel::Info, msg);
}

pub fn warn(msg: &str) {
    log(LogLevel::Warn, msg);
}

pub fn error(msg: &str) {
    log(LogLevel::Error, msg);
}

/// `xxxxxxxx-xxxx-...` in lowercase.
pub fn uuid_of(player: &Player) -> String {
    let id = player.get_id();
    Uuid::from_high_low(id.high, id.low).to_string()
}

pub fn api_uuid(uuid: &str) -> Option<crate::papi::uuid::Uuid> {
    let (high, low) = Uuid::parse(uuid)?.high_low();
    Some(crate::papi::uuid::Uuid { high, low })
}

/// The id of the player's entity (what `EntityDamageEvent` names).
pub fn entity_id(player: &Player) -> i32 {
    i32::try_from(player.as_entity().get_id()).unwrap_or(i32::MAX)
}

/// Dark screen on this client only: the server does not have the effect, so it
/// never reaches the data file.
pub fn blind(player: &Player, on: bool) {
    let Some(java) = player.as_java() else { return };
    let entity_id = entity_id(player);
    if on {
        java.send_packet(&ClientboundPacket::CUpdateMobEffect(CUpdateMobEffect {
            entity_id,
            effect_id: BLINDNESS,
            amplifier: 0,
            duration: -1,
            flags: 0,
        }));
    } else {
        java.send_packet(&ClientboundPacket::CRemoveMobEffect(CRemoveMobEffect { entity_id, effect_id: BLINDNESS }));
    }
}

/// Shows or hides `player` for every other player (a fresh handle per call:
/// the API takes the player resource by value).
fn visible(server: &Server, player: &Player, show: bool) {
    let me = uuid_of(player);
    for other in server.get_all_players() {
        if uuid_of(&other) == me {
            continue;
        }
        if let Some(handle) = server.get_player_by_uuid(player.get_id()) {
            if show {
                other.show_player(handle);
            } else {
                other.hide_player(handle);
            }
        }
    }
}

/// Holds a player: dark screen, hidden from everybody else.
pub fn hold(server: &Server, player: &Player) {
    blind(player, true);
    visible(server, player, false);
}

/// Undoes [`hold`].
pub fn release(server: &Server, player: &Player) {
    blind(player, false);
    visible(server, player, true);
}

fn named(n: Named) -> NamedColor {
    match n {
        Named::Black => NamedColor::Black,
        Named::DarkBlue => NamedColor::DarkBlue,
        Named::DarkGreen => NamedColor::DarkGreen,
        Named::DarkAqua => NamedColor::DarkAqua,
        Named::DarkRed => NamedColor::DarkRed,
        Named::DarkPurple => NamedColor::DarkPurple,
        Named::Gold => NamedColor::Gold,
        Named::Gray => NamedColor::Gray,
        Named::DarkGray => NamedColor::DarkGray,
        Named::Blue => NamedColor::Blue,
        Named::Green => NamedColor::Green,
        Named::Aqua => NamedColor::Aqua,
        Named::Red => NamedColor::Red,
        Named::LightPurple => NamedColor::LightPurple,
        Named::Yellow => NamedColor::Yellow,
        Named::White => NamedColor::White,
    }
}

fn segment(seg: &Segment) -> TextComponent {
    let mut c = TextComponent::text(&seg.text);
    match seg.style.color {
        Some(Color::Named(n)) => c = c.color_named(named(n)),
        Some(Color::Rgb(rgb)) => {
            let [_, r, g, b] = rgb.to_be_bytes();
            c = c.color_rgb(RgbColor { r, g, b });
        }
        None => {}
    }
    let st = seg.style;
    if st.bold {
        c = c.bold(true);
    }
    if st.italic {
        c = c.italic(true);
    }
    if st.underlined {
        c = c.underlined(true);
    }
    if st.strikethrough {
        c = c.strikethrough(true);
    }
    if st.obfuscated {
        c = c.obfuscated(true);
    }
    match &seg.click {
        Some(Click::Suggest(s)) => c = c.click_suggest_command(s),
        Some(Click::Run(s)) => c = c.click_run_command(s),
        Some(Click::Copy(s)) => c = c.click_copy_to_clipboard(s),
        Some(Click::Url(s)) => c = c.click_open_url(s),
        None => {}
    }
    if let Some(h) = &seg.hover {
        c = c.hover_show_text(component(h));
    }
    c
}

/// Rich text as one component: lines joined by line breaks.
pub fn component(t: &Text) -> TextComponent {
    let mut root = TextComponent::text("");
    for (i, line) in t.lines.iter().enumerate() {
        if i > 0 {
            root = root.add_child(TextComponent::text("\n"));
        }
        for seg in &line.segments {
            root = root.add_child(segment(seg));
        }
    }
    root
}

pub fn say(player: &Player, t: &Text) {
    if !t.is_empty() {
        player.send_system_message(component(t), false);
    }
}

/// Players get components, the console plain text.
pub fn reply(sender: &CommandSender, t: &Text) {
    if t.is_empty() {
        return;
    }
    match sender.as_player() {
        Some(p) => say(&p, t),
        None => sender.send_message(TextComponent::text(&t.plain())),
    }
}

pub fn kick(player: &Player, screen: &Text) {
    if let Some(java) = player.as_java() {
        java.kick(JavaKickOptions {
            reason: component(screen),
            log_to_console: false,
            teardown_policy: SocketTeardownPolicy::Graceful,
        });
    } else if let Some(bedrock) = player.as_bedrock() {
        let plain = screen.plain();
        bedrock.kick(&BedrockKickOptions {
            reason: BedrockDisconnectReason::Kicked,
            message: plain.clone(),
            skip_message: false,
            filtered_message: plain,
            log_to_console: false,
            teardown_policy: SocketTeardownPolicy::Graceful,
        });
    }
}
