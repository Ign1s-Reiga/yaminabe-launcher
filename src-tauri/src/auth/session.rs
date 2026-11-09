use reqwest::Client;
use yaminabe_launcher_shared::error::Error;
use super::flow::{check_session, refresh_account_tokens, Session};
use super::model::{now_epoch_seconds, AccountStore, MinecraftAccount, MinecraftAccountRecord};
use super::store::{save_record, write_account_secret};

/// How bringing a saved sign-in up to date went.
#[derive(Debug)]
pub(crate) enum Renewal {
    /// The token is accepted as saved.
    Current,
    /// The token was renewed, so the account needs saving.
    Updated,
    /// The token is accepted, but the player name changed on minecraft.net, so
    /// the account's record needs saving.
    Renamed,
    /// Minecraft services could not be asked, so the token is used as saved.
    Unchecked(Error),
}

/// Bring `account`'s Minecraft token up to date. One near expiry is refreshed
/// outright; 300 s of headroom covers a launch's library and asset prefetch.
/// Any other is checked first, and refreshed only if Minecraft services turn
/// it down. `step` hears each stage, for a progress log.
///
/// An error means the token could not be renewed. An inconclusive one
/// (`Error::is_inconclusive`) says nothing about the account, and a later try
/// may work. `Error::GameNotOwned` means the account cannot play at all, and
/// any other means the user has to sign in again.
pub(crate) async fn renew_session(
    client: &Client,
    account: &mut MinecraftAccount,
    mut step: impl FnMut(String),
) -> Result<Renewal, Error> {
    let expiring = account.mc_access_token.is_empty() || account.expires_at - now_epoch_seconds() < 300;
    if !expiring {
        step(format!("Checking the Microsoft sign-in for {}...", account.username));
        match check_session(client, &account.mc_access_token).await? {
            Session::Valid(profile) if profile.name == account.username => return Ok(Renewal::Current),
            // The name can change on minecraft.net while the token stays good.
            Session::Valid(profile) => {
                account.username = profile.name;
                return Ok(Renewal::Renamed);
            }
            Session::Unchecked(e) => return Ok(Renewal::Unchecked(e)),
            Session::Rejected => step("Minecraft services no longer accept the saved sign-in.".to_string()),
        }
    }
    step(format!("Refreshing Microsoft credentials for {}...", account.username));
    refresh_account_tokens(client, account).await?;
    Ok(Renewal::Updated)
}

/// Whether `read`, the record a renewal started from, is still the saved one,
/// i.e. the account was neither removed nor signed in again since.
fn still_saved(store: &AccountStore, read: &MinecraftAccountRecord) -> bool {
    store.accounts.iter().any(|a| a.uuid == read.uuid && a.expires_at == read.expires_at)
}

/// What saving a renewal came to.
#[derive(Debug)]
pub(crate) struct RenewalSave {
    /// Whether it applied, which it does only while the record it started
    /// from is still the saved one.
    pub(crate) applied: bool,
    /// Whether the accounts in memory now carry a new player name from it,
    /// written to disk or not.
    pub(crate) renamed: bool,
    /// Why writing it out failed, if it did.
    pub(crate) failure: Option<Error>,
}

/// Save `account` after a renewal that started from `read`. `with_secret`
/// says its tokens changed, which go to the keyring first; a new player name
/// alone changes only the record. Nothing is saved unless `read` is still the
/// saved record, so a late renewal neither brings a removed account back
/// without its keyring entry nor overwrites a newer sign-in.
pub(crate) fn save_renewal(
    store: &mut AccountStore,
    account: &MinecraftAccount,
    read: &MinecraftAccountRecord,
    with_secret: bool,
) -> RenewalSave {
    if !still_saved(store, read) {
        return RenewalSave { applied: false, renamed: false, failure: None };
    }
    if with_secret
        && let Err(e) = write_account_secret(account)
    {
        return RenewalSave { applied: true, renamed: false, failure: Some(e) };
    }
    // The record changes in memory before it is written, so even a failed
    // write leaves the new name in the list the UI fetches.
    let failure = save_record(store, account).err();
    RenewalSave { applied: true, renamed: account.username != read.username, failure }
}

#[cfg(test)]
mod tests {
    use super::{save_renewal, still_saved};
    use crate::auth::model::{AccountStore, MinecraftAccount, MinecraftAccountRecord};

    fn renamed_account() -> MinecraftAccount {
        MinecraftAccount {
            uuid: "abc".into(),
            username: "NewName".into(),
            mc_access_token: "token".into(),
            expires_at: 100,
            ms_refresh_token: "refresh".into(),
            xuid: String::new(),
        }
    }

    #[test]
    fn a_removed_account_is_not_saved_back() {
        let mut store = AccountStore::default();
        let read = MinecraftAccountRecord::from_account(&renamed_account());
        let saved = save_renewal(&mut store, &renamed_account(), &read, false);
        assert!(!saved.applied && !saved.renamed && saved.failure.is_none());
        assert!(store.accounts.is_empty());
    }

    #[test]
    fn a_newer_sign_in_is_not_overwritten() {
        let mut newer = MinecraftAccountRecord::from_account(&renamed_account());
        newer.username = "Fresh".into();
        newer.expires_at = 200;
        let mut store = AccountStore::default();
        store.accounts.push(newer);
        let read = MinecraftAccountRecord::from_account(&renamed_account());
        let saved = save_renewal(&mut store, &renamed_account(), &read, false);
        assert!(!saved.applied && !saved.renamed);
        assert_eq!(store.accounts[0].username, "Fresh");
    }

    #[test]
    fn the_record_a_renewal_started_from_can_be_saved_over() {
        let read = MinecraftAccountRecord::from_account(&renamed_account());
        let mut store = AccountStore::default();
        store.accounts.push(read.clone());
        assert!(still_saved(&store, &read));
    }
    #[test]
    fn a_rename_is_saved_and_reported() {
        let path = crate::ACCOUNTS_PATH.get_or_init(|| {
            std::env::temp_dir().join(format!("yaminabe-accounts-test-{}.json", std::process::id()))
        });
        let mut before = renamed_account();
        before.username = "OldName".into();
        let read = MinecraftAccountRecord::from_account(&before);
        let mut store = AccountStore::default();
        store.accounts.push(read.clone());
        let saved = save_renewal(&mut store, &renamed_account(), &read, false);
        assert!(saved.applied && saved.renamed && saved.failure.is_none(), "got {saved:?}");
        assert_eq!(store.accounts[0].username, "NewName");
        std::fs::remove_file(path).ok();
    }
}
