# PumboAuth

Accounts and login for Pumpkin servers in offline mode. New players register with `/register`, returning ones log in with `/login`. Until then they wait at their spawn on a dark screen, hidden from others, unable to move or be hurt.

![Register, log in, sessions and two-factor login](https://raw.githubusercontent.com/PumboMC/PumboAuth/main/assets/market/demo.gif)

## Features

- **Register and login.** Common passwords and the nickname as a password are refused. After 3 wrong passwords the player is disconnected.
- **Sessions.** A player who comes back from the same address within 60 minutes skips the password.
- **Two-factor login.** TOTP codes from any authenticator app, with recovery codes.
- **Safe passwords.** Stored as argon2id and never written to the server log.
- **Lockouts.** Too many wrong passwords block the address and the account for a few hours. New accounts per address are limited.
- **Nickname protection.** Length, allowed characters, blocked names and letter case.
- **Countdown.** A boss bar shows the time left to log in.
- **AuthMe import.** Takes over accounts from an AuthMe CSV export.
- **Languages.** English and Polish built in, any other from `lang/<code>.yml`.

## Commands

| Command | What it does |
| --- | --- |
| `/register <password> <password>` | Create an account |
| `/login <password> [code]` | Log in |
| `/2fa enable\|confirm\|disable` | Turn two-factor login on or off |
| `/changepassword <old> <new>` | Change the password |
| `/logout` | End the session |
| `/unregister <password>` | Remove the account |
| `/premium <password>` | Switch the account to premium login |

| Admin command | What it does |
| --- | --- |
| `/pa forceregister <nick> <password>` | Create an account |
| `/pa forcechangepassword <nick> <password>` | Set a password |
| `/pa unregister <nick>` | Remove an account |
| `/pa forcelogin <nick>` | Log in a player who is waiting |
| `/pa unblock <ip\|nick>` | Clear failed logins |
| `/pa reset2fa <nick>` | Turn off two-factor login |
| `/pa import authme <file>` | Import accounts from AuthMe |
| `/pa stats` / `reload` / `version` | Numbers, reload the config, show the version |

`/pa help` shows every command you may use. `/pa` is short for `/pumboauth`. Admin permissions are named `pumboauth:<command>` and default to operators.

## Screenshots

![Welcome screen with /register](https://raw.githubusercontent.com/PumboMC/PumboAuth/main/assets/market/register.png)
![Welcome back screen with /login](https://raw.githubusercontent.com/PumboMC/PumboAuth/main/assets/market/login.png)
![Account created](https://raw.githubusercontent.com/PumboMC/PumboAuth/main/assets/market/account-created.png)
![Two-factor login setup](https://raw.githubusercontent.com/PumboMC/PumboAuth/main/assets/market/2fa.png)

## Installation

Drop the file into `plugins/` and start the server. The config is created in `plugins/data/pumboauth/`. Works with Pumpkin 0.2.0 (Minecraft 26.3).

## Premium players

On a plain offline-mode server everyone logs in with a password, because the server can't tell premium players apart. Premium players skip the password behind PumboProx, or behind another proxy that checks Mojang accounts (Velocity modern forwarding).

## Running a network?

PumboAuth also runs on [PumboProx](https://github.com/PumboMC/PumboProx): one account for every server, login before any server sees the player, and premium players join without a password.

---

PumboAuth is in beta. Try it on a test server before you put players on it.
Source, documentation and issues: https://github.com/PumboMC/PumboAuth (GPL-3.0)
