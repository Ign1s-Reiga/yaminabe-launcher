use reqwest::Client;
use yaminabe_launcher_shared::error::Error;
use super::flow::{check_session, refresh_account_tokens, Session};
use super::model::{now_epoch_seconds, AccountStore, MinecraftAccount};
use super::store::{persist_account, save_record};

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

/// Save what `renewal` changed in `account`, whose record held `saved_expiry`
/// when it was read. Nothing is saved when the account was removed or signed
/// in again since, so a late renewal neither brings a removed account back
/// without its keyring entry nor overwrites a newer sign-in. Returns whether it
/// saved.
pub(crate) fn save_renewal(
    store: &mut AccountStore,
    account: &MinecraftAccount,
    renewal: &Renewal,
    saved_expiry: i64,
) -> Result<bool, Error> {
    if !store.accounts.iter().any(|a| a.uuid == account.uuid && a.expires_at == saved_expiry) {
        return Ok(false);
    }
    match renewal {
        Renewal::Updated => persist_account(store, account)?,
        Renewal::Renamed => save_record(store, account)?,
        Renewal::Current | Renewal::Unchecked(_) => return Ok(false),
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::{save_renewal, Renewal};
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
        let saved = save_renewal(&mut store, &renamed_account(), &Renewal::Renamed, 100).expect("no save attempted");
        assert!(!saved);
        assert!(store.accounts.is_empty());
    }

    #[test]
    fn a_newer_sign_in_is_not_overwritten() {
        let mut newer = MinecraftAccountRecord::from_account(&renamed_account());
        newer.username = "Fresh".into();
        newer.expires_at = 200;
        let mut store = AccountStore::default();
        store.accounts.push(newer);
        let saved = save_renewal(&mut store, &renamed_account(), &Renewal::Renamed, 100).expect("no save attempted");
        assert!(!saved);
        assert_eq!(store.accounts[0].username, "Fresh");
    }
}
