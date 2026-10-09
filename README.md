<p align="center">
  <img src="assets/logo.png" alt="Pumbo logo" width="160">
</p>

<h1 align="center">PumboAuth</h1>

<p align="center">Accounts and login for Pumpkin servers and PumboProx networks.</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-GPL--3.0-blue" alt="License: GPL-3.0"></a>
  <img src="https://img.shields.io/badge/built%20with-Rust-orange?logo=rust" alt="Built with Rust">
  <img src="https://img.shields.io/badge/plugin-WebAssembly-654FF0?logo=webassembly&logoColor=white" alt="WebAssembly plugin">
  <a href="https://github.com/Pumpkin-MC/Pumpkin"><img src="https://img.shields.io/badge/Pumpkin-0.2.0%20%2826.3%29-F28C28" alt="Pumpkin 0.2.0 (26.3)"></a>
  <a href="https://github.com/Pumpkin-MC/Pumpkin"><img src="https://img.shields.io/badge/Pumpkin-0.1.0--dev%20%2826.2%29-F28C28" alt="Pumpkin 0.1.0-dev (26.2)"></a>
  <a href="https://github.com/PumboMC/PumboProx"><img src="https://img.shields.io/badge/PumboProx-supported-62B47A" alt="PumboProx: supported"></a>
  <img src="https://img.shields.io/badge/status-beta-yellow" alt="Status: beta">
</p>

<p align="center">
  <a href="#features">Features</a> ·
  <a href="#two-builds">Two builds</a> ·
  <a href="#installation">Installation</a> ·
  <a href="#configuration">Configuration</a> ·
  <a href="#commands-and-permissions">Commands</a> ·
  <a href="#building">Building</a>
</p>

---

<p align="center">
  <a href="https://github.com/PumboMC/PumboProx"><img src="assets/pumboprox.webp" alt="PumboProx: everything you need to run a network on Pumpkin" width="100%"></a>
</p>

<p align="center"><b>Running more than one server?</b> <a href="https://github.com/PumboMC/PumboProx">PumboProx</a> is the proxy for Pumpkin networks, with plugins in WebAssembly.<br>PumboAuth runs on it too: players log in once for the whole network, and premium players join without a password.</p>

> [!NOTE]
> PumboAuth is in **beta** (0.1.0-beta.1). Try it on a test server before you put players on it.

## What it does

PumboAuth asks players without a Mojang account to register and log in with a password. Premium players, and players who come back within their session, log in by themselves. On PumboProx one account works on every server of the network, and the login happens before any server sees the player.

## How a player gets in

On PumboProx, PumboFilter and PumboAuth work as two gates in a row. Each also works alone, and on a plain Pumpkin server.

```
Player joins
   │
   ▼
Login on the proxy ──────────────────────── before encryption
   ├── PumboFilter   counts connections, attack mode, auto-ban of addresses that keep failing
   └── PumboAuth     refuses bad nicknames, locked accounts, new players while registrations are closed
   │
   ▼
Gates in a virtual world (PumboVir) ─────── no server sees the player yet
   │
   ├── 1. PumboFilter                       gate "filter"
   │      ├── let through: premium (outside an attack), verified in the last 12 h,
   │      │                whitelist, pumbo.filter.bypass
   │      ├── gravity check      falls for 6.4 s, compared with vanilla physics
   │      ├── client check       brand and settings of the game
   │      ├── CAPTCHA if needed  a code on a map in hand → /captcha <code>
   │      └── passed → next gate in the same connection, no reconnect screen
   │
   └── 2. PumboAuth                         gate "auth"
          ├── premium            logged in by Mojang, no password
          ├── session            joined from here in the last 60 min
          ├── new player         /register <password> <password>
          ├── known player       /login <password>   (+ /2fa <code> with 2FA on)
          ├── countdown bar, lockout after wrong passwords, argon2id
          └── logged in → a server
   │
   ▼
lobby (Pumpkin) ─────────────────────────── /changepassword, /logout, /2fa, /premium
                                            are taken by the proxy and never reach a server
```

## Features

| | Feature | |
| --- | --- | --- |
| 📝 | **Register** | `/register <password> <password>` creates an account. Common passwords and the nickname as a password are refused. |
| 🔑 | **Login** | `/login <password>`. A wrong password shows the attempts left. After 3 wrong ones the player is disconnected. |
| ⚡ | **Auto-login** | Premium players (with PumboProx, or behind a proxy that checks Mojang accounts) and players with a session go straight in, without a prompt. |
| 👑 | **Premium** | Mojang accounts need no password (with PumboProx, or behind a proxy that checks Mojang accounts). An existing password account can switch to premium with `/premium <password>`. |
| 🕒 | **Sessions** | A player who comes back from the same address within 60 minutes skips the password. `/logout` ends the session. |
| 📱 | **Two-factor login** | TOTP codes from any authenticator app, with recovery codes. |
| 🔒 | **Argon2id** | Passwords are stored as argon2id hashes. Imported bcrypt and AuthMe hashes are replaced at the next login. |
| 🚫 | **Lockouts** | Addresses and accounts with too many wrong passwords are blocked for some hours. New accounts per address are limited too. |
| 🏷️ | **Nickname protection** | Length, allowed characters, blocked names and letter case. Offline players cannot take the nickname of a premium account. |
| ⏳ | **Countdown** | A boss bar shows the time left to log in (60 seconds by default). |
| 🙈 | **Passwords stay private** | Password commands never reach the server log. On PumboProx they never reach a server at all. |
| 📥 | **AuthMe import** | `/pumboauth import authme <file>` takes over accounts from an AuthMe CSV export. |
| 🌍 | **Multilingual** | English and Polish built in, any other language from `lang/<code>.yml`. On PumboProx each player gets the language of their game. |

## Two builds

The login rules live in one shared core. You pick the build that fits your setup.

| Build | File | Where it goes | What is different |
| --- | --- | --- | --- |
| 🌐 **PumboProx** (whole network) | `PumboAuth-Proxy-<version>.wasm` | `plugins/` of the proxy | One account database for every server. The player logs in in the proxy's virtual world, before any server sees them, so `/server` cannot skip the login. |
| 🎃 **Pumpkin** (one server) | `PumboAuth-Pumpkin-26.3-<version>.wasm` or `PumboAuth-Pumpkin-26.2-<version>.wasm` | `plugins/` of the server | The player waits at their own place: dark screen, hidden from others, cannot move or be hurt. Each server has its own accounts. |

> [!IMPORTANT]
> Premium auto-login needs PumboProx, or another proxy that checks Mojang accounts and forwards them (Velocity modern forwarding). On a plain Pumpkin server in offline mode the server cannot tell premium players apart, so every player registers and logs in with a password.

## Installation

> [!TIP]
> Download the files from [Releases](https://github.com/PumboMC/PumboAuth/releases/latest), or [build from source](#building).

**On PumboProx**

1. Put `PumboAuth-Proxy-<version>.wasm` into the proxy's `plugins/` folder.
2. Start the proxy. The first start creates `plugins/pumbo-auth/config.yml` with comments.
3. Recommended in `pumboprox.yml`, so nobody gets in without a login:

   ```yaml
   plugins:
     required-gates: [auth]
   ```

Whether a player is premium is decided by the proxy (`login.online-mode` in `pumboprox.yml`).

**On Pumpkin**

1. Put the file that matches your Pumpkin version into the server's `plugins/` folder.
2. Start the server. The first start creates `plugins/data/pumboauth/config.yml` and the language files.

The plugin only needs to read and write its own data folder. It has no network access.

## Configuration

The config file is written on the first start, with a short comment on every option. Reload it with `/pumboauth reload`. A wrong value is reported in the log with the option name and replaced by the default. The most used options:

| Option | Default | What it does |
| --- | --- | --- |
| `language` | `en` | Language of the messages: `en`, `pl` or any code with a `lang/<code>.yml` file. |
| `auth.timeout-seconds` | `60` | Time to log in or register, shown as a countdown bar. |
| `auth.login-attempts` | `3` | Wrong passwords per connection before the player is disconnected. |
| `auth.min-password-length` | `4` | Shortest password allowed. |
| `auth.register-repeat-password` | `true` | Ask for the password twice in `/register`. |
| `auth.disable-registrations` | `false` | New players cannot create accounts. |
| `auth.ip-registrations-max` | `3` | Accounts per address within `ip-registrations-hours` (6). |
| `auth.ip-bruteforce-max` | `10` | Wrong passwords before an address is blocked. |
| `sessions.ttl-minutes` | `60` | How long a session lasts. |
| `premium.forwarded-profiles` | `true` | Premium players log in by themselves. Turn it off behind BungeeCord forwarding without BungeeGuard. |
| `nickname.blocked` | none | Nicknames that are refused, `*` matches anything (`["admin*"]`). |
| `nickname.premium-protection` | `true` | Offline players cannot join with a premium account's nickname. |
| `two-factor.issuer` | `Minecraft` | The name shown in the authenticator app. |

Every section also has `enabled`.

## Commands and permissions

Player commands, for everyone:

| Command | Aliases | What it does |
| --- | --- | --- |
| `/register <password> <password>` | `/reg` | Create an account |
| `/login <password> [code]` | `/l`, `/log` | Log in |
| `/2fa <code>` | `/totp` | The second step of the login |
| `/2fa enable\|confirm\|disable` | | Turn two-factor login on or off |
| `/changepassword <old> <new>` | `/changepass`, `/cp` | Change the password |
| `/unregister <password>` | `/unreg` | Remove the account |
| `/logout` | `/destroysession` | End the session |
| `/premium <password>` | `/license` | Switch the account to premium login |

Admin commands, `/pumboauth <command>` or the short `/pa <command>`, permission `pumbo.auth.<command>`:

| Command | What it does |
| --- | --- |
| `stats` | Players logging in, accounts, logins |
| `forceregister <nick> <password>` | Create an account |
| `forcechangepassword <nick> <password>` | Set a password |
| `unregister <nick>` | Remove an account |
| `forcelogin <nick>` | Log in a player who is at the prompt |
| `forcepremium <nick>` / `forceoffline <nick>` | Switch an account to premium or back to a password |
| `reset2fa <nick>` | Turn off two-factor login |
| `unblock <ip\|nick>` | Clear failed logins |
| `logout <nick>` | End the sessions of an account |
| `import authme <file>` | Import accounts from an AuthMe CSV export |
| `reload` / `version` | Reload the config, show the version |

On PumboProx the admin commands also work as `/pumbo auth <command>` (`/pumbo auth` alone shows the help) and from the proxy console. On Pumpkin `/pa` and `/auth` are aliases of `/pumboauth` (`commands.admin-aliases` in `config.yml`), the permissions are named `pumboauth:<command>` and default to operators.

## Works with other Pumbo plugins

PumboAuth works on its own. When it finds other Pumbo plugins, it works with them:

| Plugin | Together |
| --- | --- |
| 🛡️ [PumboFilter](https://github.com/PumboMC/PumboFilter) | The player passes the bot check first, then logs in, on the same connection. |
| 🚫 [PumboBans](https://github.com/PumboMC/PumboBans) | Banned players are refused before PumboAuth asks them anything. On PumboProx a ban also ends the player's sessions. |
| 🌐 [PumboProx](https://github.com/PumboMC/PumboProx) | One login for the whole network. Other proxy plugins get the placeholders `%pumboauth_logged_in%`, `%pumboauth_premium%` and `%pumboauth_registered%`. |

## Building

You need Rust stable with the WebAssembly target:

```sh
rustup target add wasm32-wasip2
```

Cargo fetches the shared Pumbo libraries (`pumbo-common`, `pumbo-sdk`) from the [PumboProx](https://github.com/PumboMC/PumboProx) repository on the first build.

Pumpkin build, both versions (`dist/PumboAuth-26.3.wasm` and `dist/PumboAuth-26.2.wasm`):

```sh
plugins/pumbo-auth-pumpkin/build.sh
```

PumboProx build (`plugins/pumbo-auth-prox/dist/pumbo-auth.wasm`):

```sh
plugins/pumbo-auth-prox/build.sh
```

Tests of the core and both builds:

```sh
cargo test
```

## License

PumboAuth (the core and both builds) is licensed under the [GNU General Public License v3.0](LICENSE). The shared library for Pumbo plugins (`pumbo-common`) is dual-licensed under MIT and Apache-2.0.

PumboAuth is not affiliated with Mojang, Microsoft or the Pumpkin project.

---

<p align="center">
  Part of <a href="https://github.com/PumboMC/PumboProx"><b>PumboProx</b></a>. Everything you need to run a network on Pumpkin.
</p>
