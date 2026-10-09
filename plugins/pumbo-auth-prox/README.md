# PumboAuth for PumboProx

PumboAuth on the PumboProx proxy: one account database for the whole network behind the proxy, and the login before any server sees the player. The rules are the same as in PumboAuth for Pumpkin (`pumbo-auth-core`).

## What happens where

| When | What PumboAuth does |
| --- | --- |
| Login, before encryption (`on-pre-login`) | refuses invalid or blocked nicknames, addresses and accounts locked after wrong passwords, and new players while registrations are closed |
| After login (gate `auth`, priority 200, after PumboFilter) | premium players and players with a valid session go on; the others wait in the virtual world for `/register` or `/login` (and `/2fa`), with a countdown bar |
| In the virtual world | no proxy command runs there, so `/server` cannot skip the login; no server sees the player yet |
| On a server | `/changepassword`, `/unregister`, `/logout`, `/2fa`, `/premium` are taken by the proxy before the server |

Every player command is sensitive: its arguments never reach a log and the command never reaches a server, even while PumboAuth is not running (the proxy then says that the login is unavailable).

**Premium.** Whether a player is premium is decided by the proxy (`login.online-mode` in `pumboprox.yml`; with `per-player` the proxy asks Mojang about every nickname and logs premium nicknames in with Mojang). A premium player needs no password: PumboAuth stores a premium account at the first join, which protects the nickname. An existing password account with that nickname stays a password account until its owner switches it with `/premium <password>`.

Messages follow the language of the player's game (`en`, `pl` and files in `lang/`), with `per-player-language = true`.

## Installation

1. Copy `pumbo-auth.wasm` into the proxy's `plugins/` folder (under any name, e.g. with the version): one file, the manifest is built in. The first start creates `plugins/pumbo-auth/config.yml` (the defaults with comments), `plugins/pumbo-auth/lang/en.yml` and `lang/pl.yml` (every message) and `plugins/data/pumbo-auth/`; a file you changed is never overwritten (one you did not change gets the new texts of an update), and options or messages a newer version adds or changes are named in the log. A `pumbo-auth.yml` left from an older version overrides the built-in manifest (the log says so): delete it.
2. Optional: edit `plugins/pumbo-auth/config.yml` (see [Configuration](#configuration)) and the messages in `plugins/pumbo-auth/lang/<code>.yml`; another language is one more file there. An AuthMe CSV export for `/pumboauth import authme <file>` goes to `plugins/data/pumbo-auth/`.
3. Recommended in `pumboprox.yml`, so that nobody gets in without a login:

   ```yaml
   plugins:
     required-gates: [auth]
   ```

The database is `plugins/data/pumbo-auth/auth.redb`. If it cannot be opened, nobody logs in.

## With other plugins

- PumboFilter: its gate (priority 100) comes first; the player goes from its world to the login in the same connection.
- PumboBans: a ban (`pumbo:player-punished@1.0`) ends the player's sessions, so after the ban is lifted they log in with the password again.
- Topics for others: `pumbo:player-authenticated@1.0` (method: password, premium, session) and `pumbo:player-registered@1.0`; placeholders `%pumboauth_logged_in%`, `%pumboauth_premium%`, `%pumboauth_registered%`.

## Configuration

`plugins/pumbo-auth/config.yml` (reload with `/pumbo auth reload`; a file that is not valid YAML is reported with the line and column and the current settings stay). One account database serves the whole network behind the proxy. A player who has to log in waits in the proxy's virtual world, before any server sees them: passwords never reach a server or a log. Premium players and players with a session go on by themselves.

| Option | Default | Meaning |
| --- | --- | --- |
| `language` | `en` | language of the messages: `en`, `pl` or any code with a `lang/<code>.yml` file |
| `per-player-language` | `true` | players whose game is set to another language with messages get that language |
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
| `premium.forwarded-profiles` | `true` | players the proxy logged in with Mojang (`login.online-mode: per-player` or `true` in `pumboprox.yml`) log in by themselves, without a password |
| `premium.save-accounts` | `true` | store an account for a premium player at the first join (protects the nickname) |
| `nickname.min-length` / `max-length` / `allowed-chars` | `3` / `16` / letters, digits, `_` | allowed nicknames |
| `nickname.blocked` | none | nicknames that are refused; `*` matches anything (`["admin*"]`) |
| `nickname.case-protection` | `true` | the nickname of an account has to be typed in the same letter case |
| `nickname.premium-protection` | `true` | the nickname of a premium account cannot join as an offline player |
| `two-factor.issuer` / `recovery-codes` | `Minecraft` / `8` | the name shown in the authenticator app, number of recovery codes |
| `import.enabled` | `true` | `/pumboauth import authme <file>`, the file in `plugins/data/pumbo-auth/` |
| `hashing.argon2-memory-kib` / `argon2-iterations` / `argon2-parallelism` | `19456` / `2` / `1` | argon2id cost for new passwords |
| `hashing.rehash-legacy` | `true` | replace imported hashes (bcrypt, AuthMe SHA) with argon2id at the next login |

Every section has `enabled` (default `true`). A wrong value is reported in the proxy log with the option name and replaced by the default.

## Commands and permissions

Player commands (everyone): `/register <password> <password>` (`/reg`), `/login <password> [code]` (`/l`, `/log`), `/2fa <code>` and `/2fa enable|confirm|disable` (`/totp`), `/changepassword <old> <new>` (`/changepass`, `/cp`), `/unregister <password>` (`/unreg`), `/logout` (`/destroysession`), `/premium <password>` (`/license`).

Admin: `/pumboauth <command>` (also `/pa <command>`, `/pumbo auth <command>` and the proxy console; `/pa` alone shows the help), permission `pumbo.auth.<command>`: `stats`, `forceregister <nick> <password>`, `forcechangepassword <nick> <password>`, `unregister <nick>`, `forcelogin <nick>`, `forcepremium <nick>`, `forceoffline <nick>`, `reset2fa <nick>`, `unblock <ip|nick>`, `logout <nick>`, `import authme <file>`; `reload`, `version`, `debug` are run by the proxy. In the list of `/pumbo auth` the admin commands show as `admin-<command>`.

## Building

`./build.sh` builds `dist/`.
