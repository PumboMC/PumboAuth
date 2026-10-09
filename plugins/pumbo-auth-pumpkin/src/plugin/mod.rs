//! The WebAssembly plugin: loading, registration with Pumpkin, other plugins.

mod api;
mod commands;
mod effects;
mod events;
mod state;

use std::cell::Cell;
use std::collections::HashMap;

use pumbo_auth_core::commands::{reloaded, tree};
use pumbo_auth_core::flow::{Auth, PLAYER_COMMANDS};
use pumbo_common::clock::now_ms;
use pumbo_common::config::{self, Warning};
use pumbo_common::gate::{self, Hold};
use pumbo_common::lang::Lang;
use pumbo_common::rich::Text;
use pumbo_common::store::Store;
use pumbo_common::style;
use pumbo_common::text::Args;

use api::{
    ArgumentType, Command, CommandNode, Context, EventPriority, Permission, PermissionDefault, PermissionLevel, Plugin,
    PluginMetadata, SchedulerExt, StringType,
};

use crate::config::{Config, TEMPLATE};
use crate::{BUNDLES, PLUGIN_NAME, pumpkin_node};

/// PumboFilter is asked about waiting players every this many ticks.
const FILTER_POLL_TICKS: u64 = 5;

fn load_config(dir: &str) -> (Config, Vec<Warning>) {
    let (text, warning) = config::read_or_create(&format!("{dir}/config.yml"), TEMPLATE);
    let (cfg, mut warnings) = config::load::<Config>(&text);
    warnings.extend(warning);
    warnings.extend(config::old_files(dir));
    (cfg, warnings)
}

fn load_lang(dir: &str, code: &str) -> (Lang, Vec<Warning>) {
    let template = Lang::template(&BUNDLES, code);
    let (text, warning) = config::read_or_create(&format!("{dir}/lang/{code}.yml"), &template);
    let (lang, warnings) = Lang::load(&BUNDLES, code, Some(&text));
    let mut warnings: Vec<Warning> =
        warnings.into_iter().map(|w| Warning { message: format!("lang/{code}.yml: {}", w.message), ..w }).collect();
    warnings.extend(warning);
    (lang, warnings)
}

/// `/pumboauth reload`: config and messages (the database stays; `commands`
/// changes after a restart).
fn reload(rt: &mut state::Rt) -> Text {
    let (cfg, mut warnings) = load_config(&rt.dir);
    let (lang, more) = load_lang(&rt.dir, &cfg.language);
    warnings.extend(more);
    if let Some(w) = warnings.iter().find(|w| w.fatal) {
        api::warn(&format!("PumboAuth: reload refused, the current settings stay: {w}"));
        return style::error(&rt.auth.lang, "command-reload-failed", &Args::new().arg(style::value(&w.message)));
    }
    if cfg.admin_names() != rt.auth.admin_names {
        warnings.push(Warning::new(Some("commands.admin-aliases"), "changes after a server restart"));
    }
    for w in &warnings {
        api::warn(&format!("PumboAuth: config: {w}"));
    }
    rt.auth.reload(cfg.settings(), lang);
    reloaded(&rt.auth, warnings.len())
}

thread_local! {
    static TICKS: Cell<u64> = const { Cell::new(0) };
}

pub struct PumboAuth;

impl Plugin for PumboAuth {
    fn new() -> Self {
        PumboAuth
    }

    fn metadata(&self) -> PluginMetadata {
        PluginMetadata {
            name: PLUGIN_NAME.into(),
            version: env!("CARGO_PKG_VERSION").into(),
            authors: vec!["Patryk Skoczylas".into()],
            description: "Accounts: register, log in, premium, two-factor login, sessions".into(),
            dependencies: vec![],
            permissions: vec![api::permissions::FS_READ_DATA.into(), api::permissions::FS_WRITE_DATA.into()],
        }
    }

    fn on_load(&self, context: Context) -> Result<(), String> {
        let dir = context.get_data_folder();
        let _ = std::fs::create_dir_all(format!("{dir}/lang"));
        let (cfg, mut warnings) = load_config(&dir);
        let (lang, more) = load_lang(&dir, &cfg.language);
        warnings.extend(more);
        for w in &warnings {
            api::warn(&format!("PumboAuth: config: {w}"));
        }
        let store = Store::open(format!("{dir}/auth.redb"));
        let mut auth = Auth::new(cfg.settings(), lang, store);
        if let Some(why) = &auth.store_error {
            api::error(&format!("PumboAuth: the database is not available, nobody can log in: {why}"));
        }
        let admin_names = cfg.admin_names();
        auth.admin_names = admin_names.clone();

        // Players already online (a reload of the plugin) are logged in: the
        // ones who were not were disconnected when the plugin was unloaded.
        let server = context.get_server();
        let mut entities = HashMap::new();
        for p in server.get_all_players() {
            let uuid = api::uuid_of(&p);
            entities.insert(api::entity_id(&p), uuid.clone());
            auth.adopt(&uuid, &p.get_name(), &pumbo_common::id::strip_port(&p.get_ip()));
        }
        state::install(state::Rt { dir, auth, entities });

        // A second registration of the same node (after a reload) is an error
        // in Pumpkin; the node is there either way.
        let admin = PermissionDefault::Op(PermissionLevel::Three);
        let mut nodes: Vec<(String, String, PermissionDefault)> = tree()
            .subs()
            .iter()
            .map(|s| (pumpkin_node(&tree().permission(s)), format!("PumboAuth: /pumboauth {}", s.name), admin))
            .collect();
        nodes.push(("pumboauth:command".into(), "PumboAuth: see /pumboauth".into(), admin));
        nodes.push((
            "pumboauth:player".into(),
            "PumboAuth: /login, /register and the other player commands".into(),
            PermissionDefault::Allow,
        ));
        for (node, description, default) in nodes {
            let _ = context.register_permission(&Permission { node, description, default, children: vec![] });
        }

        // Player commands are registered so that clients show and complete them
        // (white, with Tab); a player's command is handled and cancelled in the
        // command event before the server runs or logs it.
        for (name, aliases) in PLAYER_COMMANDS {
            let mut names = vec![(*name).to_string()];
            names.extend(aliases.iter().map(|a| (*a).to_string()));
            let arg = match *name {
                "register" | "changepassword" => "passwords",
                "2fa" | "premium" => "code-or-action",
                _ => "password",
            };
            let cmd = Command::new(&names, "PumboAuth")
                .then(
                    CommandNode::argument(arg, &ArgumentType::String(StringType::Greedy))
                        .execute(commands::PlayersOnly),
                )
                .execute(commands::PlayersOnly);
            context.register_command(cmd, "pumboauth:player");
        }
        let admin_cmd = Command::new(&admin_names, "PumboAuth: accounts")
            .then(
                CommandNode::argument("args", &ArgumentType::String(StringType::Greedy))
                    .execute(commands::Admin { with_args: true }),
            )
            .execute(commands::Admin { with_args: false });
        context.register_command(admin_cmd, "pumboauth:command");

        context.register_event_handler::<api::AsyncPlayerPreLoginEvent, _>(
            events::PreLogin,
            EventPriority::Highest,
            true,
        )?;
        context.register_event_handler::<api::PlayerJoinEvent, _>(events::Join, EventPriority::Highest, true)?;
        context.register_event_handler::<api::PlayerLeaveEvent, _>(events::Leave, EventPriority::Highest, true)?;
        context.register_event_handler::<api::PlayerMoveEvent, _>(events::Move, EventPriority::Normal, false)?;
        context.register_event_handler::<api::PlayerChatEvent, _>(events::Chat, EventPriority::Highest, true)?;
        context.register_event_handler::<api::PlayerCommandSendEvent, _>(
            events::CommandSend,
            EventPriority::Highest,
            true,
        )?;
        context.register_event_handler::<api::EntityDamageEvent, _>(events::Damage, EventPriority::Highest, true)?;
        context.register_event_handler::<api::PlayerInteractEvent, _>(
            events::Interact,
            EventPriority::Highest,
            true,
        )?;
        context.register_event_handler::<api::PlayerInteractEntityEvent, _>(
            events::InteractEntity,
            EventPriority::Highest,
            true,
        )?;
        context.register_event_handler::<api::BlockBreakEvent, _>(events::Break, EventPriority::Highest, true)?;
        context.register_event_handler::<api::BlockPlaceEvent, _>(events::Place, EventPriority::Highest, true)?;
        context.register_event_handler::<api::PlayerDropItemEvent, _>(events::Drop, EventPriority::Highest, true)?;
        context.register_event_handler::<api::InventoryClickEvent, _>(events::Click, EventPriority::Highest, true)?;
        context.register_event_handler::<api::InventoryCreativeEvent, _>(
            events::Creative,
            EventPriority::Highest,
            true,
        )?;
        context.register_event_handler::<api::PlayerItemHeldEvent, _>(
            events::HeldSlot,
            EventPriority::Highest,
            true,
        )?;
        context.register_event_handler::<api::PlayerItemConsumeEvent, _>(
            events::Consume,
            EventPriority::Highest,
            true,
        )?;
        context.register_event_handler::<api::PlayerSwapHandsEvent, _>(
            events::SwapHands,
            EventPriority::Highest,
            true,
        )?;

        // One repeating task for every timer (the API never frees task closures).
        context.schedule_repeating_task(1, 1, |server| {
            let tick = TICKS.with(|t| {
                t.set(t.get() + 1);
                t.get()
            });
            let now = now_ms();
            if tick.is_multiple_of(FILTER_POLL_TICKS) {
                let waiting = state::with(|rt| rt.auth.waiting()).unwrap_or_default();
                for uuid in waiting {
                    let filter = effects::filter_holding(&uuid);
                    let fx = state::with(|rt| rt.auth.filter_state(&uuid, filter, now)).unwrap_or_default();
                    effects::apply(&server, None, fx);
                }
            }
            let fx = state::with(|rt| rt.auth.tick(now)).unwrap_or_default();
            effects::apply(&server, None, fx);
        });

        api::info(&format!(
            "PumboAuth {} loaded for {} (commands /{})",
            env!("CARGO_PKG_VERSION"),
            api::PLATFORM,
            admin_names.join(", /")
        ));
        Ok(())
    }

    fn on_unload(&self, context: Context) -> Result<(), String> {
        // Pumpkin lets no plugin disconnect players while it is being unloaded:
        // players who have not logged in are released and asked to join again.
        // Then the database is closed.
        let fx = state::with(|rt| rt.auth.unload()).unwrap_or_default();
        let held = fx.iter().filter(|e| matches!(e, pumbo_auth_core::flow::Effect::Unhold(_))).count();
        effects::apply(&context.get_server(), None, fx);
        state::with_bars(|bars| {
            for (_, b) in bars.drain() {
                b.remove_all();
            }
        });
        state::uninstall();
        api::info(&format!("PumboAuth: unloaded, {held} players who had not logged in were released"));
        Ok(())
    }

    fn handle_ipc_message(&self, _sender: String, message: Vec<u8>) -> Result<Vec<u8>, String> {
        Ok(gate::answer(&message, "PumboAuth", env!("CARGO_PKG_VERSION"), |uuid| {
            state::with(|rt| rt.auth.holding(uuid)).unwrap_or(Hold::Unknown)
        }))
    }
}

crate::papi::register_plugin!(PumboAuth);
