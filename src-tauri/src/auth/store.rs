use log::warn;
use yaminabe_launcher_shared::error::Error;
use super::model::{AccountStore, MinecraftAccount, MinecraftAccountRecord, MinecraftAccountSecret};

/// Keyring service name used for every per-account credential. Each account
/// has two entries under this service — `<uuid>:mc_access` and
/// `<uuid>:ms_refresh` — because Windows DPAPI caps a single credential
/// blob at 2560 UTF-16 chars, which a combined JSON of the two tokens
/// frequently exceeds.
const KEYRING_SERVICE: &str = "yaminabe-launcher";
const MC_ACCESS_SUFFIX: &str = ":mc_access";
const MS_REFRESH_SUFFIX: &str = ":ms_refresh";

/// Read `accounts.json` in the current keyring-era shape. A missing or
/// malformed file falls back to an empty store so a corrupted record doesn't
/// block app launch.
pub fn load_account_store() -> AccountStore {
    let Ok(text) = std::fs::read_to_string(crate::accounts_path()) else {
        return AccountStore::default();
    };
    match serde_json::from_str(&text) {
        Ok(store) => store,
        Err(e) => {
            warn!("accounts.json is malformed ({e}); starting with empty account list");
            AccountStore::default()
        }
    }
}

fn keyring_entry(user: &str) -> Result<keyring_core::Entry, Error> {
    keyring_core::Entry::new(KEYRING_SERVICE, user)
        .map_err(|e| Error::Keyring(format!("entry for {user}: {e}")))
}

fn read_one(user: &str) -> Result<String, Error> {
    let entry = keyring_entry(user)?;
    entry.get_password().map_err(|e| match e {
        keyring_core::Error::NoEntry => Error::NotExists(format!("stored credential '{user}'")),
        other => Error::Keyring(format!("read for {user}: {other}")),
    })
}

fn write_one(user: &str, value: &str) -> Result<(), Error> {
    let entry = keyring_entry(user)?;
    entry
        .set_password(value)
        .map_err(|e| Error::Keyring(format!("write for {user}: {e}")))
}

fn delete_one(user: &str) -> Result<(), Error> {
    let entry = keyring_entry(user)?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        // Already gone — treat as success so a stale UI re-issue doesn't fail.
        Err(keyring_core::Error::NoEntry) => Ok(()),
        Err(e) => Err(Error::Keyring(format!("delete for {user}: {e}"))),
    }
}

fn read_secret(uuid: &str) -> Result<MinecraftAccountSecret, Error> {
    let mc = read_one(&format!("{uuid}{MC_ACCESS_SUFFIX}"))?;
    let ms = read_one(&format!("{uuid}{MS_REFRESH_SUFFIX}"))?;
    Ok(MinecraftAccountSecret {
        mc_access_token: mc,
        ms_refresh_token: ms,
    })
}

pub(crate) fn write_secret(uuid: &str, secret: &MinecraftAccountSecret) -> Result<(), Error> {
    write_one(&format!("{uuid}{MC_ACCESS_SUFFIX}"), &secret.mc_access_token)?;
    write_one(&format!("{uuid}{MS_REFRESH_SUFFIX}"), &secret.ms_refresh_token)?;
    Ok(())
}

pub(crate) fn delete_secret(uuid: &str) -> Result<(), Error> {
    delete_one(&format!("{uuid}{MC_ACCESS_SUFFIX}"))?;
    delete_one(&format!("{uuid}{MS_REFRESH_SUFFIX}"))?;
    Ok(())
}

/// Compose a full `MinecraftAccount` by reading the secret payload for the
/// record's UUID from the OS keyring.
pub fn hydrate_account(record: &MinecraftAccountRecord) -> Result<MinecraftAccount, Error> {
    let secret = read_secret(&record.uuid)?;
    Ok(MinecraftAccount {
        uuid: record.uuid.clone(),
        username: record.username.clone(),
        mc_access_token: secret.mc_access_token,
        expires_at: record.expires_at,
        ms_refresh_token: secret.ms_refresh_token,
        xuid: record.xuid.clone(),
    })
}

/// Write a fully-formed `MinecraftAccount` back to disk + keyring. The secret
/// is written first so a failure leaves no orphaned record pointing at a
/// missing keyring entry.
pub fn persist_account(store: &mut AccountStore, account: &MinecraftAccount) -> Result<(), Error> {
    write_secret(
        &account.uuid,
        &MinecraftAccountSecret {
            mc_access_token: account.mc_access_token.clone(),
            ms_refresh_token: account.ms_refresh_token.clone(),
        },
    )?;
    store.upsert(MinecraftAccountRecord::from_account(account));
    store.save()
}

#[cfg(test)]
mod tests {
    use super::{delete_secret, read_secret, write_secret};
    use super::super::model::MinecraftAccountSecret;
    use yaminabe_launcher_shared::error::Error;
    use std::sync::Once;

    static MOCK_STORE_INIT: Once = Once::new();

    fn init_test_keyring() {
        MOCK_STORE_INIT.call_once(|| {
            let store = keyring_core::mock::Store::new().expect("mock keyring store");
            keyring_core::set_default_store(store);
        });
    }

    #[test]
    fn secret_round_trip() {
        init_test_keyring();
        let uuid = "round-trip-uuid";
        let secret = MinecraftAccountSecret {
            mc_access_token: "access-token-value".into(),
            ms_refresh_token: "refresh-token-value".into(),
        };
        write_secret(uuid, &secret).expect("write_secret");
        let got = read_secret(uuid).expect("read_secret");
        assert_eq!(got.mc_access_token, "access-token-value");
        assert_eq!(got.ms_refresh_token, "refresh-token-value");
        delete_secret(uuid).ok();
    }

    #[test]
    fn read_missing_is_not_exists() {
        init_test_keyring();
        let err = read_secret("never-written-uuid-read").expect_err("expected NotExists");
        assert!(matches!(err, Error::NotExists(_)), "got {err:?}");
    }

    #[test]
    fn delete_idempotent_on_missing() {
        init_test_keyring();
        delete_secret("never-written-uuid-del").expect("delete missing should succeed");
    }

    #[test]
    fn delete_then_read_is_not_exists() {
        init_test_keyring();
        let uuid = "delete-then-read-uuid";
        let secret = MinecraftAccountSecret {
            mc_access_token: "a".into(),
            ms_refresh_token: "r".into(),
        };
        write_secret(uuid, &secret).expect("write");
        delete_secret(uuid).expect("delete");
        let err = read_secret(uuid).expect_err("expected NotExists after delete");
        assert!(matches!(err, Error::NotExists(_)));
    }

    #[test]
    fn split_entries_handle_long_values() {
        // Each token is stored as its own keyring entry, so the combined
        // length doesn't have to fit any single-credential ceiling.
        init_test_keyring();
        let uuid = "split-entries-uuid";
        let secret = MinecraftAccountSecret {
            mc_access_token: "a".repeat(3000),
            ms_refresh_token: "r".repeat(2000),
        };
        write_secret(uuid, &secret).expect("write");
        let got = read_secret(uuid).expect("read");
        assert_eq!(got.mc_access_token.len(), 3000);
        assert_eq!(got.ms_refresh_token.len(), 2000);
        delete_secret(uuid).ok();
    }

    #[test]
    fn rewrite_replaces_value() {
        init_test_keyring();
        let uuid = "rewrite-uuid";
        let first = MinecraftAccountSecret {
            mc_access_token: "first-access".into(),
            ms_refresh_token: "first-refresh".into(),
        };
        let second = MinecraftAccountSecret {
            mc_access_token: "second-access".into(),
            ms_refresh_token: "second-refresh".into(),
        };
        write_secret(uuid, &first).expect("first write");
        write_secret(uuid, &second).expect("second write");
        let got = read_secret(uuid).expect("read");
        assert_eq!(got.mc_access_token, "second-access");
        assert_eq!(got.ms_refresh_token, "second-refresh");
        delete_secret(uuid).ok();
    }
}
