//! Event handlers. Everything that must happen before the server goes on
//! (cancelling, refusing a connection) is blocking.

use pumbo_auth_core::flow::{Entry, JoinInfo};
use pumbo_common::clock::now_ms;
use pumbo_common::id::{is_authenticated_profile, strip_port};

use super::api::{
    self, AsyncPlayerPreLoginEvent, BlockBreakEvent, BlockPlaceEvent, EntityDamageEvent, EventData, EventHandler,
    InventoryClickEvent, InventoryCreativeEvent, Player, PlayerChatEvent, PlayerCommandSendEvent, PlayerDropItemEvent,
    PlayerInteractEntityEvent, PlayerInteractEvent, PlayerItemConsumeEvent, PlayerItemHeldEvent, PlayerJoinEvent,
    PlayerLeaveEvent, PlayerMoveEvent, PlayerSwapHandsEvent, Server,
};
use super::effects::{self, apply};
use super::state;

pub struct PreLogin;

impl EventHandler<AsyncPlayerPreLoginEvent> for PreLogin {
    fn handle(
        &self,
        server: Server,
        mut e: EventData<AsyncPlayerPreLoginEvent>,
    ) -> EventData<AsyncPlayerPreLoginEvent> {
        let name = e.player_name.clone();
        let uuid = e.player_uuid.clone();
        let ip = strip_port(&e.ip_address);
        let online = api::api_uuid(&uuid)
            .and_then(|u| server.get_player_by_uuid(u))
            .or_else(|| server.get_player_by_name(&name))
            .map(|p| api::uuid_of(&p));
        let result = state::with(|rt| rt.auth.pre_login(&name, &uuid, &ip, online.as_deref(), now_ms()));
        let Some((entry, fx)) = result else {
            // Busy (a nested call): refusing is the safe answer for a login plugin.
            e.cancelled = true;
            e.kick_message =
                api::component(&pumbo_common::rich::Text::parse("&cPumboAuth is busy, please join again."));
            return e;
        };
        apply(&server, None, fx);
        if let Entry::Refuse(screen) = entry {
            e.cancelled = true;
            e.kick_message = api::component(&screen);
        }
        e
    }
}

fn signed_skin(player: &Player) -> bool {
    player.get_skin().is_some_and(|s| s.signature.is_some())
}

pub struct Join;

impl EventHandler<PlayerJoinEvent> for Join {
    fn handle(&self, server: Server, e: EventData<PlayerJoinEvent>) -> EventData<PlayerJoinEvent> {
        let player = &e.player;
        let uuid = api::uuid_of(player);
        let info = JoinInfo {
            uuid: uuid.clone(),
            name: player.get_name(),
            ip: strip_port(&player.get_ip()),
            authenticated: is_authenticated_profile(&uuid, signed_skin(player)),
            locale: None,
        };
        // Asked before the state is taken (the answer is a host call).
        let filter = effects::filter_holding(&uuid);
        let entity = api::entity_id(player);
        let result = state::with(|rt| {
            rt.entities.insert(entity, uuid.clone());
            let held = rt.auth.held();
            (held, rt.auth.join(info, filter, now_ms()))
        });
        let Some((held, fx)) = result else {
            api::kick(player, &pumbo_common::rich::Text::parse("&cPumboAuth is busy, please join again."));
            return e;
        };
        // Players held right now stay hidden from the newcomer too.
        for h in held {
            if let Some(other) = api::api_uuid(&h).and_then(|u| server.get_player_by_uuid(u)) {
                player.hide_player(other);
            }
        }
        apply(&server, Some(player), fx);
        e
    }
}

pub struct Leave;

impl EventHandler<PlayerLeaveEvent> for Leave {
    fn handle(&self, server: Server, e: EventData<PlayerLeaveEvent>) -> EventData<PlayerLeaveEvent> {
        let uuid = api::uuid_of(&e.player);
        state::with(|rt| {
            rt.entities.retain(|_, u| *u != uuid);
            rt.auth.leave(&uuid);
        });
        apply(&server, Some(&e.player), vec![pumbo_auth_core::flow::Effect::Bossbar(uuid, None)]);
        e
    }
}

pub struct Move;

impl EventHandler<PlayerMoveEvent> for Move {
    fn handle(&self, server: Server, e: EventData<PlayerMoveEvent>) -> EventData<PlayerMoveEvent> {
        let uuid = api::uuid_of(&e.player);
        let (x, y, z) = e.to_position;
        let fx = state::with(|rt| rt.auth.moved(&uuid, x, y, z)).unwrap_or_default();
        apply(&server, Some(&e.player), fx);
        e
    }
}

pub struct Chat;

impl EventHandler<PlayerChatEvent> for Chat {
    fn handle(&self, server: Server, mut e: EventData<PlayerChatEvent>) -> EventData<PlayerChatEvent> {
        let uuid = api::uuid_of(&e.player);
        let Some((cancel, fx)) = state::with(|rt| rt.auth.chat(&uuid)) else {
            e.cancelled = true;
            return e;
        };
        if cancel {
            // A cancelled message is neither sent nor written to the server log.
            e.cancelled = true;
        }
        apply(&server, Some(&e.player), fx);
        e
    }
}

pub struct CommandSend;

impl EventHandler<PlayerCommandSendEvent> for CommandSend {
    fn handle(&self, server: Server, mut e: EventData<PlayerCommandSendEvent>) -> EventData<PlayerCommandSendEvent> {
        let uuid = api::uuid_of(&e.player);
        let line = e.command.clone();
        let Some((cancel, fx)) = state::with(|rt| rt.auth.command(&uuid, &line, now_ms())) else {
            // Busy: never let a command with a password reach the server log.
            e.cancelled = true;
            return e;
        };
        if cancel {
            // Cancelled commands are not run and not written to the server log.
            e.cancelled = true;
        }
        apply(&server, Some(&e.player), fx);
        e
    }
}

pub struct Damage;

impl EventHandler<EntityDamageEvent> for Damage {
    fn handle(&self, _server: Server, mut e: EventData<EntityDamageEvent>) -> EventData<EntityDamageEvent> {
        // Only players this plugin holds right now, matched by the entity id of
        // their current connection.
        let held = state::with(|rt| rt.entities.get(&e.entity_id).is_some_and(|u| rt.auth.is_held(u))).unwrap_or(false);
        if held {
            e.cancelled = true;
        }
        e
    }
}

fn held(player: &Player) -> bool {
    let uuid = api::uuid_of(player);
    state::with(|rt| rt.auth.is_held(&uuid)).unwrap_or(false)
}

/// Handlers that cancel an action of a held player.
macro_rules! block_held {
    ($($name:ident: $event:ty),* $(,)?) => {$(
        pub struct $name;

        impl EventHandler<$event> for $name {
            fn handle(&self, _server: Server, mut e: EventData<$event>) -> EventData<$event> {
                if held(&e.player) {
                    e.cancelled = true;
                }
                e
            }
        }
    )*};
}

block_held!(
    Interact: PlayerInteractEvent,
    InteractEntity: PlayerInteractEntityEvent,
    Place: BlockPlaceEvent,
    Drop: PlayerDropItemEvent,
    Click: InventoryClickEvent,
    Creative: InventoryCreativeEvent,
    HeldSlot: PlayerItemHeldEvent,
    Consume: PlayerItemConsumeEvent,
    SwapHands: PlayerSwapHandsEvent,
);

pub struct Break;

impl EventHandler<BlockBreakEvent> for Break {
    fn handle(&self, _server: Server, mut e: EventData<BlockBreakEvent>) -> EventData<BlockBreakEvent> {
        if e.player.as_ref().is_some_and(held) {
            e.cancelled = true;
        }
        e
    }
}
