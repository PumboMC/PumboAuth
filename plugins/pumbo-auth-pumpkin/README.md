# PumboAuth

PumboAuth is the login of a [Pumpkin](https://github.com/Pumpkin-MC/Pumpkin) Minecraft server: accounts with passwords, automatic login for premium players and returning players, two-factor login and protection of nicknames. It is written in Rust and runs as a WebAssembly plugin on the official Pumpkin releases. The rules live in `pumbo-auth-core`, so the same login will run on the PumboProx proxy later.

PumboAuth works on its own. When [PumboFilter](../pumbo-filter-pumpkin/README.md) is installed too, a player first passes PumboFilter's checks and then logs in, on the same connection.

## Features

- **Register and log in**: `/register`, `/login`, `/changepassword`, `/unregister`, argon2id hashes, attempt limits per connection, address and account lockouts, a limit of new accounts per address, a time limit with a countdown bar.
- **Clear answers**: every reply says what happened: a wrong password (with the attempts left), a missing argument (with the usage), no account yet, an account that exists already, success.
- **Passwords stay out of the log**: the commands are registered (the client shows them in white and completes them with Tab), but PumboAuth handles and cancels them before the server runs them, and Pumpkin writes a command to its log only when it is not cancelled. Chat of players who are not logged in is cancelled too, so a password typed without the slash is not logged either.
- **Premium login**: players authenticated by Mojang (a version 4 UUID with a signed skin) log in by themselves; offline players cannot use the nickname of a premium account.
- **Sessions**: a player who comes back from the same address with the same UUID within `ttl-minutes` skips the password; `/logout` ends it.
- **Two-factor login**: TOTP codes from any authenticator app, recovery codes.
- **Nickname rules**: length, characters, blocked names, letter case of registered nicknames.
- **AuthMe import**: `/pumboauth import authme <file>` (bcrypt and `$SHA$` hashes, replaced by argon2id at the next login).
- **Nothing changes in the player's data**: PumboAuth never moves a player and never changes their game mode, effects, inventory or anything else that the server saves.
- English and Polish messages, the shared Pumbo look.

## Compatibility

| File | Pumpkin | Minecraft |
| --- | --- | --- |
| `PumboAuth-26.3.wasm` | release `0.2.0+26.3-26.51` | 26.3 |
| `PumboAuth-26.2.wasm` | release `0.1.0-dev+26.2-26.45` | 26.2 |

Pumpkin checks the plugin API strictly: a file only loads on the server version it was built for. No source build or server hooks are needed.

This is a new plugin. Accounts of the older single-plugin PumboAuth (`pumboauth.redb` from `pumbo-limbo`) are not migrated: it had no users yet. Start with a fresh `plugins/data/pumboauth/` folder.

## How the login works

1. Before the player enters the world: nickname rules, the premium protection, address lockouts and refused second connections (a player never plays on two connections at once).
2. A premium player, or a player with a session, is logged in at once and plays.
3. Anybody else is held at their own place: the screen is dark (a blindness effect on their client only, the server does not have it), other players do not see them, they cannot be hurt and cannot use, move or drop anything, chat or use other commands. Walking away from where they stand puts them back. A countdown bar shows the time left.
4. After `/register` or `/login` (and the code with two-factor login) everything is back to normal: the screen gets light and they play.

With PumboFilter, PumboAuth waits until PumboFilter's checks are done (it asks PumboFilter over Pumpkin's `ipc` every five ticks), then asks the player to log in. Whichever plugin is done last releases the player.

### Limitations

- The player's real inventory stays visible until they log in (the hotbar and the inventory screen): released Pumpkin lets plugins send no inventory packets. Nothing in it can be used, moved or dropped.
- On a standalone server in offline mode nobody is premium: without a proxy Pumpkin cannot check a single player with Mojang. Use online mode, or a proxy with authenticated forwarding.
- `premium.forwarded-profiles` trusts the profile the server got. That is safe in online mode and behind Velocity modern forwarding, PumboProx, BungeeGuard or Vine; turn it off behind BungeeCord forwarding without BungeeGuard, where a client can send any profile.
- Behind a proxy every server with PumboAuth has its own accounts. Install it on the server players join first, and do not let players switch servers through the proxy (`/server`) before they log in; the login on the proxy itself comes with PumboProx.
- The two-factor setup shows the key and a link to copy in chat; there is no QR code on a map (released Pumpkin would send the map to every player).
- If the plugin is unloaded (`plugin unload`), Pumpkin lets it disconnect nobody: players who have not logged in are released and asked to join again. While it is unloaded or after it crashed, nothing cancels the player commands: Pumpkin then writes `/login`, `/changepassword` and the others to its log with their arguments (also as unknown commands), and the players it held stay in the dark until they reconnect. Reload it right away (`plugin load`), and check the log for `Loaded pumboauth` after every update.

## Installation

1. Put the matching `.wasm` file into the `plugins/` folder of the server.
2. Start the server. PumboAuth writes `plugins/data/pumboauth/config.yml`, `lang/en.yml` and `lang/pl.yml`, and keeps its data in `plugins/data/pumboauth/auth.redb`.

The plugin needs the `fs.read.data` and `fs.write.data` permissions of Pumpkin's plugin sandbox and nothing else (no network).

Note: Pumpkin 0.2.0 and 0.1.0-dev hang on "Starting save." when they get SIGINT (also without plugins); stop them with the `stop` command.

## Building

```sh
./build.sh          # dist/PumboAuth-26.3.wasm and dist/PumboAuth-26.2.wasm (at the workspace root)
cargo test -p pumbo-auth-core -p pumbo-auth-pumpkin
```

Rust stable with the `wasm32-wasip2` target is required.

## Commands

Player commands (permission `pumboauth:player`, everyone):

| Command | Aliases | Description |
| --- | --- | --- |
| `/register <password> <password>` | `/reg` | create an account (`/register <password>` with `register-repeat-password = false`) |
| `/login <password> [code]` | `/l`, `/log` | log in |
| `/2fa <code>` | `/totp` | the second step of the login |
| `/2fa enable <password>`, `/2fa confirm <code>`, `/2fa disable <code>` | | two-factor login |
| `/changepassword <old> <new>` | `/changepass`, `/cp` | change the password (also logs in at the prompt) |
| `/unregister <password>` | `/unreg` | remove the account |
| `/logout` | `/destroysession` | end the session |
| `/premium <password>`, then `/premium confirm` | `/license` | switch the account to premium login (join with the premium account first) |

Admin commands: `/pumboauth <subcommand>`, also `/pa` and `/auth` (`[commands] admin-aliases`, read at start). `/pumboauth` or `/pumboauth help [page]` lists what you may use. Permissions are `pumboauth:<subcommand>` (operators of level 3 by default) and `pumboauth:command` to see the command. A player's `/pumboauth` is handled in the command event as well, so `forceregister` passwords stay out of the log.

| Subcommand | Description |
| --- | --- |
| `reload` | reload the config and messages |
| `stats` | players logging in, accounts, logins |
| `forceregister <nick> <password>` | create an account |
| `forcechangepassword <nick> <password>` | set a password (also makes a premium account a normal one) |
| `unregister <nick>` | remove an account (an online player is disconnected) |
| `forcelogin <nick>` | log in a player who is at the prompt |
| `forcepremium <nick>` | the next premium join with this nickname claims the account |
| `forceoffline <nick>` | the account logs in with a password again |
| `reset2fa <nick>` | turn off two-factor login |
| `unblock <ip\|nick>` | clear failed logins |
| `logout <nick>` | remove the sessions of an account |
| `import authme <file>` | import a CSV export (nickname,hash) from the plugin data folder |
| `version` | version and platform |

## Configuration

`plugins/data/pumboauth/config.yml` (reload with `/pumboauth reload`; `commands` changes after a restart). The file is written with short comments on first start, the most used options at the top; the details are here.

A player who has to log in is held at their own place: the screen is dark (on the client only), they are hidden from others, cannot move, be hurt or use anything until they log in. Premium players and players with a session log in by themselves. With PumboFilter installed, the login starts once its checks are done, on the same connection.

| Option | Default | Meaning |
| --- | --- | --- |
| `language` | `en` | language of the messages: `en`, `pl` or any code with a `lang/<code>.yml` file |
| `auth.timeout-seconds` | `60` | time to log in or register; a countdown bar shows it (`auth.bossbar`) |
| `auth.login-attempts` | `3` | wrong passwords per connection before the player is disconnected |
| `auth.min-password-length` / `max-password-length` | `4` / `71` | password length |
| `auth.check-password-strength` | `true` | refuse the most common passwords and the nickname as the password |
| `auth.register-repeat-password` | `true` | `/register <password> <password>` (`true`) or `/register <password>` (`false`) |
| `auth.disable-registrations` | `false` | new players cannot create accounts |
| `auth.ip-registrations-max` / `ip-registrations-hours` | `3` / `6` | at most this many accounts per address within the hours (0: no limit) |
| `auth.ip-bruteforce-max` / `ip-bruteforce-hours` | `10` / `8` | an address with this many wrong passwords within the hours is blocked (0: off) |
| `auth.account-bruteforce-max` / `account-bruteforce-hours` | `20` / `8` | an account with this many wrong passwords within the hours is locked (0: off) |
| `auth.command-cooldown-ms` | `750` | least time between two commands that check a password |
| `auth.confirm-keyword` | `confirm` | the word that confirms `/premium` |
| `sessions.enabled` / `ttl-minutes` | `true` / `60` | a player who comes back from the same address with the same UUID within the time skips the password |
| `premium.forwarded-profiles` | `true` | players authenticated by Mojang log in by themselves. They are recognised by a version 4 UUID with a signed skin, which only a server in online mode or authenticated proxy forwarding (Velocity modern, PumboProx, BungeeGuard, Vine) gives; an offline player always has a version 3 UUID. Turn this off behind BungeeCord forwarding without BungeeGuard, where a client can send any profile |
| `premium.save-accounts` | `true` | store an account for a premium player at the first join (protects the nickname) |
| `nickname.min-length` / `max-length` / `allowed-chars` | `3` / `16` / letters, digits, `_` | allowed nicknames |
| `nickname.blocked` | none | nicknames that are refused; `*` matches anything (`["admin*"]`) |
| `nickname.case-protection` | `true` | the nickname of an account has to be typed in the same letter case |
| `nickname.premium-protection` | `true` | the nickname of a premium account cannot join as an offline player |
| `two-factor.issuer` / `recovery-codes` | `Minecraft` / `8` | the name shown in the authenticator app, number of recovery codes |
| `commands.admin-aliases` | `[pa, auth]` | other names of `/pumboauth`, read at start (a reload does not change them); leave out a name another plugin uses, `[]` turns them off |
| `import.enabled` | `true` | `/pumboauth import authme <file>` |
| `hashing.argon2-memory-kib` / `argon2-iterations` / `argon2-parallelism` | `19456` / `2` / `1` | argon2id cost for new passwords |
| `hashing.rehash-legacy` | `true` | replace imported hashes (bcrypt, AuthMe SHA) with argon2id at the next login |

Every section has `enabled` (default `true`). A wrong value is reported in the server log with the option name and replaced by the default.

## Importing from AuthMe

Export name and hash as CSV into `plugins/data/pumboauth/`:

```sh
sqlite3 -csv authme.db "SELECT realname, password FROM authme" > authme.csv
```

Then run `/pumboauth import authme authme.csv`. Existing accounts are skipped.

## Working with other plugins

- **PumboFilter**: see above; each works on its own when the other is missing, unloaded or reloaded.
- **PumboBans**: banned players are refused before they enter the world, before PumboAuth asks them anything.

## License

GPL-3.0-only, see [LICENSE](../../LICENSE).
