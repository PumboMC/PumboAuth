//! Account data on top of a plugin's [`Store`]: accounts with a premium UUID
//! index, sessions, brute-force counters and registrations per address.
//!
//! Tables (keys in the canonical forms of `pumbo_common::id`, values JSON unless
//! noted):
//!
//! | table | key | value |
//! | --- | --- | --- |
//! | `accounts` | name key | [`Account`] |
//! | `premium_uuids` | UUID | name key (text) |
//! | `sessions` | `name|ip` | `{uuid, expires}` |
//! | `ip_failures`, `account_failures` | ip key / name key | `{count, since}` |
//! | `registrations` | ip key | registration times |
//!
//! Accounts are written durably; sessions, counters and caches with eventual
//! durability. A damaged account record is an error, never "no account".

use pumbo_common::id::{Uuid, ip_key_str, name_key};
use pumbo_common::store::{Durability, ReadExt, Result, Store, StoreError, WriteExt, WriteOps, json_or_default};
use serde::{Deserialize, Serialize};

pub const ACCOUNTS: &str = "accounts";
pub const PREMIUM: &str = "premium_uuids";
pub const SESSIONS: &str = "sessions";
pub const IP_FAILS: &str = "ip_failures";
pub const ACCOUNT_FAILS: &str = "account_failures";
pub const REGISTRATIONS: &str = "registrations";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Account {
    /// Nickname in its registered letter case.
    pub name: String,
    /// Password hash (PHC argon2id or an imported format). `None` for premium accounts.
    pub hash: Option<String>,
    pub premium: bool,
    pub premium_uuid: Option<String>,
    pub uuid: Option<String>,
    pub reg_ip: Option<String>,
    pub reg_time: u64,
    pub last_ip: Option<String>,
    pub last_login: u64,
    /// Base32 TOTP secret when two-factor login is enabled.
    pub totp_secret: Option<String>,
    pub totp_last_step: Option<u64>,
    /// SHA-256 hashes of unused recovery codes.
    pub recovery: Vec<String>,
    pub password_changed: u64,
}

/// Which failure counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Counter {
    /// By address (`ip_key`).
    Ip,
    /// By nickname.
    Account,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Failures {
    count: u32,
    since: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Session {
    uuid: String,
    expires: u64,
}

/// Key of a UUID in the premium index: hyphenated lowercase when it parses.
pub fn uuid_key(uuid: &str) -> String {
    Uuid::parse(uuid).map_or_else(|| uuid.trim().to_lowercase(), |u| u.to_string())
}

/// Key of a nickname + address pair.
pub fn session_key(name: &str, ip: &str) -> String {
    format!("{}|{}", name_key(name), ip_key_str(ip))
}

fn counter_table(c: Counter) -> &'static str {
    match c {
        Counter::Ip => IP_FAILS,
        Counter::Account => ACCOUNT_FAILS,
    }
}

fn counter_key(c: Counter, key: &str) -> String {
    match c {
        Counter::Ip => ip_key_str(key),
        Counter::Account => name_key(key),
    }
}

/// Account storage on top of a plugin's store.
#[derive(Debug, Clone, Copy)]
pub struct AuthStore<'a> {
    store: &'a Store,
}

impl<'a> AuthStore<'a> {
    pub fn new(store: &'a Store) -> Self {
        Self { store }
    }

    // ----- accounts -----

    pub fn account(&self, name: &str) -> Result<Option<Account>> {
        self.store.get_json(ACCOUNTS, &name_key(name))
    }

    pub fn put_account(&self, acc: &Account) -> Result<()> {
        self.store.write(|tx| put_account(tx, acc))
    }

    /// Removes an account with its premium index entry and sessions. True if it existed.
    pub fn delete_account(&self, name: &str) -> Result<bool> {
        let key = name_key(name);
        self.store.write(|tx| delete_account(tx, &key))
    }

    /// Moves an account to a new nickname (premium name change), in one transaction.
    pub fn rename_account(&self, old: &str, new_name: &str) -> Result<()> {
        let old_key = name_key(old);
        self.store.write(|tx| {
            let mut acc: Account =
                tx.get_json(ACCOUNTS, &old_key)?.ok_or_else(|| StoreError::new("account not found"))?;
            delete_account(tx, &old_key)?;
            acc.name = new_name.to_string();
            put_account(tx, &acc)
        })
    }

    /// Adds accounts that do not exist yet, in one transaction. Returns (added, skipped).
    pub fn import_accounts(&self, accounts: &[Account]) -> Result<(u32, u32)> {
        self.store.write(|tx| {
            let (mut added, mut skipped) = (0u32, 0u32);
            for acc in accounts {
                if tx.get(ACCOUNTS, &name_key(&acc.name))?.is_some() {
                    skipped += 1;
                    continue;
                }
                put_account(tx, acc)?;
                added += 1;
            }
            Ok((added, skipped))
        })
    }

    pub fn account_by_premium_uuid(&self, uuid: &str) -> Result<Option<Account>> {
        let name = self.store.read(|tx| tx.get(PREMIUM, &uuid_key(uuid)))?;
        match name {
            Some(bytes) => self.account(&String::from_utf8_lossy(&bytes)),
            None => Ok(None),
        }
    }

    pub fn count_accounts(&self) -> Result<u64> {
        self.store.read(|tx| tx.len(ACCOUNTS))
    }

    // ----- sessions -----

    pub fn create_session(&self, name: &str, ip: &str, uuid: &str, expires_ms: u64) -> Result<()> {
        let session = Session { uuid: uuid.to_string(), expires: expires_ms };
        let key = session_key(name, ip);
        self.store.write_with(Durability::Eventual, |tx| tx.put_json(SESSIONS, &key, &session))
    }

    /// Whether the player has an unexpired session from this address with this UUID.
    pub fn session_valid(&self, name: &str, ip: &str, uuid: &str, now_ms: u64) -> Result<bool> {
        let key = session_key(name, ip);
        let raw = self.store.read(|tx| tx.get(SESSIONS, &key))?;
        Ok(raw.map(|b| json_or_default::<Session>(&b)).is_some_and(|s| s.expires > now_ms && s.uuid == uuid))
    }

    pub fn delete_sessions(&self, name: &str) -> Result<()> {
        let prefix = format!("{}|", name_key(name));
        self.store.write(|tx| tx.retain(SESSIONS, &mut |k, _| !k.starts_with(&prefix)).map(|_| ()))
    }

    /// Removes every session of a player UUID (for example after a ban).
    /// Returns how many went.
    pub fn delete_sessions_of(&self, uuid: &str) -> Result<u64> {
        let uuid = uuid_key(uuid);
        self.store.write(|tx| tx.retain(SESSIONS, &mut |_, v| json_or_default::<Session>(v).uuid != uuid))
    }

    // ----- brute-force counters -----

    /// Failures recorded within the current window.
    pub fn failures(&self, c: Counter, key: &str, now_ms: u64, window_ms: u64) -> Result<u32> {
        let key = counter_key(c, key);
        let raw = self.store.read(|tx| tx.get(counter_table(c), &key))?;
        let f: Failures = match raw {
            Some(b) => json_or_default(&b),
            None => return Ok(0),
        };
        Ok(if now_ms.saturating_sub(f.since) > window_ms { 0 } else { f.count })
    }

    /// Counts one more failure and returns the count in the current window.
    pub fn record_failure(&self, c: Counter, key: &str, now_ms: u64, window_ms: u64) -> Result<u32> {
        let key = counter_key(c, key);
        let table = counter_table(c);
        self.store.write_with(Durability::Eventual, |tx| {
            let mut f: Failures = tx.get(table, &key)?.map(|b| json_or_default(&b)).unwrap_or_default();
            if now_ms.saturating_sub(f.since) > window_ms || f.count == 0 {
                f = Failures { count: 0, since: now_ms };
            }
            f.count = f.count.saturating_add(1);
            tx.put_json(table, &key, &f)?;
            Ok(f.count)
        })
    }

    pub fn clear_failures(&self, c: Counter, key: &str) -> Result<()> {
        let key = counter_key(c, key);
        self.store.write_with(Durability::Eventual, |tx| tx.remove(counter_table(c), &key).map(|_| ()))
    }

    // ----- registrations per address -----

    pub fn registrations(&self, ip: &str, now_ms: u64, window_ms: u64) -> Result<u32> {
        let key = ip_key_str(ip);
        let list: Vec<u64> = match self.store.read(|tx| tx.get(REGISTRATIONS, &key))? {
            Some(b) => json_or_default(&b),
            None => return Ok(0),
        };
        let n = list.iter().filter(|t| now_ms.saturating_sub(**t) <= window_ms).count();
        Ok(u32::try_from(n).unwrap_or(u32::MAX))
    }

    pub fn add_registration(&self, ip: &str, now_ms: u64, window_ms: u64) -> Result<()> {
        let key = ip_key_str(ip);
        self.store.write(|tx| {
            let mut list: Vec<u64> = tx.get(REGISTRATIONS, &key)?.map(|b| json_or_default(&b)).unwrap_or_default();
            list.retain(|t| now_ms.saturating_sub(*t) <= window_ms);
            list.push(now_ms);
            tx.put_json(REGISTRATIONS, &key, &list)
        })
    }

    /// Removes expired sessions, counters and registrations.
    /// Writes durably, so earlier eventual commits reach the disk as well.
    pub fn purge(&self, now_ms: u64, counter_window_ms: u64, registration_window_ms: u64) -> Result<()> {
        self.store.write(|tx| {
            tx.retain(SESSIONS, &mut |_, v| json_or_default::<Session>(v).expires > now_ms)?;
            for table in [IP_FAILS, ACCOUNT_FAILS] {
                tx.retain(table, &mut |_, v| {
                    now_ms.saturating_sub(json_or_default::<Failures>(v).since) <= counter_window_ms
                })?;
            }
            tx.retain(REGISTRATIONS, &mut |_, v| {
                json_or_default::<Vec<u64>>(v).iter().any(|t| now_ms.saturating_sub(*t) <= registration_window_ms)
            })?;
            Ok(())
        })
    }
}

fn put_account(tx: &mut dyn WriteOps, acc: &Account) -> Result<()> {
    let key = name_key(&acc.name);
    tx.put_json(ACCOUNTS, &key, acc)?;
    if let Some(u) = acc.premium_uuid.as_deref().filter(|_| acc.premium) {
        tx.put(PREMIUM, &uuid_key(u), key.as_bytes())?;
    }
    Ok(())
}

fn delete_account(tx: &mut dyn WriteOps, key: &str) -> Result<bool> {
    // A damaged record is removed as well; its premium entry cannot be found then.
    let existing = tx.get(ACCOUNTS, key)?.map(|b| json_or_default::<Account>(&b));
    let removed = tx.remove(ACCOUNTS, key)?;
    if let Some(u) = existing.and_then(|a| a.premium_uuid) {
        tx.remove(PREMIUM, &uuid_key(&u))?;
    }
    let prefix = format!("{key}|");
    tx.retain(SESSIONS, &mut |k, _| !k.starts_with(&prefix))?;
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTCH: &str = "069a79f4-44e9-4726-a5be-fca90e38aaf5";

    #[test]
    fn accounts_and_premium_index() {
        let store = Store::in_memory();
        let s = AuthStore::new(&store);
        assert!(s.account("Steve").unwrap().is_none());
        let acc = Account { name: "Steve".into(), hash: Some("h".into()), ..Account::default() };
        s.put_account(&acc).unwrap();
        assert_eq!(s.account("STEVE").unwrap().unwrap().name, "Steve");
        assert_eq!(s.count_accounts().unwrap(), 1);

        let prem =
            Account { name: "Notch".into(), premium: true, premium_uuid: Some(NOTCH.into()), ..Account::default() };
        s.put_account(&prem).unwrap();
        // the index finds the account by either UUID form
        assert_eq!(s.account_by_premium_uuid("069a79f444e94726a5befca90e38aaf5").unwrap().unwrap().name, "Notch");
        s.rename_account("notch", "Notch2").unwrap();
        assert!(s.account("notch").unwrap().is_none());
        assert_eq!(s.account_by_premium_uuid(NOTCH).unwrap().unwrap().name, "Notch2");
        assert!(s.rename_account("nobody", "x").is_err());

        let batch = vec![
            Account { name: "Imported".into(), hash: Some("$SHA$a$b".into()), ..Account::default() },
            Account { name: "STEVE".into(), ..Account::default() },
        ];
        assert_eq!(s.import_accounts(&batch).unwrap(), (1, 1));
        assert_eq!(s.account("imported").unwrap().unwrap().hash.as_deref(), Some("$SHA$a$b"));
        assert!(s.delete_account("steve").unwrap());
        assert!(!s.delete_account("steve").unwrap());
        assert!(s.delete_account("notch2").unwrap());
        assert!(s.account_by_premium_uuid(NOTCH).unwrap().is_none());
    }

    #[test]
    fn damaged_account_is_an_error() {
        let store = Store::in_memory();
        store.write(|tx| tx.put(ACCOUNTS, "bob", b"{broken")).unwrap();
        let s = AuthStore::new(&store);
        assert!(s.account("Bob").is_err());
        // but it can still be deleted
        assert!(s.delete_account("Bob").unwrap());
    }

    #[test]
    fn sessions_expire_and_bind_to_uuid() {
        let store = Store::in_memory();
        let s = AuthStore::new(&store);
        s.create_session("Bob", "1.2.3.4", "uuid-a", 1000).unwrap();
        assert!(s.session_valid("bob", "1.2.3.4", "uuid-a", 999).unwrap());
        assert!(s.session_valid("bob", "1.2.3.4:4321", "uuid-a", 999).unwrap());
        assert!(!s.session_valid("bob", "1.2.3.4", "uuid-a", 1000).unwrap());
        assert!(!s.session_valid("bob", "1.2.3.4", "uuid-b", 500).unwrap());
        assert!(!s.session_valid("bob", "5.6.7.8", "uuid-a", 500).unwrap());
        s.delete_sessions("BOB").unwrap();
        assert!(!s.session_valid("bob", "1.2.3.4", "uuid-a", 500).unwrap());
        let u = "069a79f4-44e9-4726-a5be-fca90e38aaf5";
        s.create_session("Ann", "1.2.3.4", u, 9000).unwrap();
        s.create_session("Ann", "5.6.7.8", u, 9000).unwrap();
        s.create_session("Bob", "1.2.3.4", "uuid-a", 9000).unwrap();
        assert_eq!(s.delete_sessions_of("069a79f444e94726a5befca90e38aaf5").unwrap(), 2);
        assert!(s.session_valid("bob", "1.2.3.4", "uuid-a", 500).unwrap());
        s.delete_sessions("bob").unwrap();
        s.create_session("Bob", "1.2.3.4", "uuid-a", 1000).unwrap();
        s.purge(5000, 1000, 1000).unwrap();
        assert_eq!(store.read(|tx| tx.len(SESSIONS)).unwrap(), 0);
        // deleting the account removes its sessions
        s.put_account(&Account { name: "Bob".into(), ..Account::default() }).unwrap();
        s.create_session("Bob", "1.2.3.4", "uuid-a", 9000).unwrap();
        s.delete_account("bob").unwrap();
        assert!(!s.session_valid("bob", "1.2.3.4", "uuid-a", 500).unwrap());
    }

    #[test]
    fn failure_windows() {
        let store = Store::in_memory();
        let s = AuthStore::new(&store);
        for i in 1..=3 {
            assert_eq!(s.record_failure(Counter::Ip, "1.2.3.4", 100, 1000).unwrap(), i);
        }
        assert_eq!(s.failures(Counter::Ip, "1.2.3.4", 500, 1000).unwrap(), 3);
        // the window ran out
        assert_eq!(s.failures(Counter::Ip, "1.2.3.4", 1200, 1000).unwrap(), 0);
        assert_eq!(s.record_failure(Counter::Ip, "1.2.3.4", 1200, 1000).unwrap(), 1);
        assert_eq!(s.failures(Counter::Account, "1.2.3.4", 1200, 1000).unwrap(), 0);
        s.clear_failures(Counter::Ip, "1.2.3.4").unwrap();
        assert_eq!(s.failures(Counter::Ip, "1.2.3.4", 1200, 1000).unwrap(), 0);
        // an IPv6 /64 counts as one address
        s.record_failure(Counter::Ip, "2001:db8:1:2::1", 0, 1000).unwrap();
        assert_eq!(s.record_failure(Counter::Ip, "2001:db8:1:2::ffff", 0, 1000).unwrap(), 2);
        assert_eq!(s.record_failure(Counter::Account, "Steve", 0, 1000).unwrap(), 1);
        assert_eq!(s.failures(Counter::Account, "STEVE", 0, 1000).unwrap(), 1);

        s.add_registration("9.9.9.9", 0, 100).unwrap();
        s.add_registration("9.9.9.9", 50, 100).unwrap();
        assert_eq!(s.registrations("9.9.9.9", 60, 100).unwrap(), 2);
        assert_eq!(s.registrations("9.9.9.9", 120, 100).unwrap(), 1);
        s.purge(10_000, 1000, 100).unwrap();
        assert_eq!(s.registrations("9.9.9.9", 120, 100).unwrap(), 0);
        assert_eq!(store.read(|tx| tx.len(IP_FAILS)).unwrap(), 0);
    }

    #[test]
    fn works_on_redb() {
        let path = std::env::temp_dir().join(format!("pumbo-auth-core-{}.redb", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let store = Store::open(&path).unwrap();
            let s = AuthStore::new(&store);
            s.put_account(&Account { name: "Alex".into(), hash: Some("h".into()), ..Account::default() }).unwrap();
            s.create_session("Alex", "10.0.0.1", "u", 50).unwrap();
        }
        let store = Store::open(&path).unwrap();
        let s = AuthStore::new(&store);
        assert_eq!(s.account("alex").unwrap().unwrap().hash.as_deref(), Some("h"));
        assert!(s.session_valid("alex", "10.0.0.1", "u", 10).unwrap());
        drop(store);
        let _ = std::fs::remove_file(&path);
    }
}
