//! Registered commands. Players' commands are handled and cancelled in the
//! command event (so passwords never reach the server log); the executors here
//! only run for the console (and never for a player while PumboAuth is loaded).

use super::api::{self, Arg, CommandError, CommandHandler, CommandSender, ConsumedArgs, Server};
use super::effects;

pub struct Admin {
    pub with_args: bool,
}

impl CommandHandler for Admin {
    fn handle(&self, sender: CommandSender, server: Server, args: ConsumedArgs) -> Result<i32, CommandError> {
        let raw = if self.with_args {
            match args.get_value("args") {
                Arg::Simple(s) | Arg::Msg(s) => s,
                _ => String::new(),
            }
        } else {
            String::new()
        };
        let args: Vec<String> = raw.split_whitespace().map(String::from).collect();
        let player = sender.as_player();
        let reply = effects::admin(&server, player.as_ref(), &args);
        api::reply(&sender, &reply);
        Ok(1)
    }
}

/// `/login` and the other player commands from the console.
pub struct PlayersOnly;

impl CommandHandler for PlayersOnly {
    fn handle(&self, sender: CommandSender, _server: Server, _args: ConsumedArgs) -> Result<i32, CommandError> {
        let text = super::state::with(|rt| {
            pumbo_common::style::error(&rt.auth.lang, "command-players-only", &pumbo_common::text::Args::new())
        })
        .unwrap_or_default();
        api::reply(&sender, &text);
        Ok(1)
    }
}
