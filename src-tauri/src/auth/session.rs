use reqwest::Client;
use yaminabe_launcher_shared::error::Error;
use super::flow::{check_session, refresh_account_tokens, Session};
use super::model::{now_epoch_seconds, MinecraftAccount};

/// How bringing a saved sign-in up to date went.
#[derive(Debug)]
pub(crate) enum Renewal {
    /// The token is accepted as saved.
    Current,
    /// The token was renewed or the player name changed, so the account needs
    /// saving.
    Updated,
    /// Minecraft services could not be asked, so the token is used as saved.
    Unchecked(Error),
}

/// Bring `account`'s Minecraft token up to date. One near expiry is refreshed
/// outright; 300 s of headroom covers a launch's library and asset prefetch.
/// Any other is checked first, and refreshed only if Minecraft services turn
/// it down. `step` hears each stage, for a progress log.
///
/// An error means the token could not be renewed: a network error says
/// nothing about the account, any other means the user has to sign in again.
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
                return Ok(Renewal::Updated);
            }
            Session::Unchecked(e) => return Ok(Renewal::Unchecked(e)),
            Session::Rejected => step("Minecraft services no longer accept the saved sign-in.".to_string()),
        }
    }
    step(format!("Refreshing Microsoft credentials for {}...", account.username));
    refresh_account_tokens(client, account).await?;
    Ok(Renewal::Updated)
}

/// Whether a failed renewal leaves the account's state unknown, because the
/// services could not be reached or did not answer properly.
pub(crate) fn is_unreachable(e: &Error) -> bool {
    matches!(
        e,
        Error::NetworkError(_)
            | Error::HttpRequestRejected(..)
            | Error::RateLimitExceeded(_)
            | Error::InvalidResponse(_)
    )
}

#[cfg(test)]
mod tests {
    use super::is_unreachable;
    use yaminabe_launcher_shared::error::Error;

    #[test]
    fn a_refused_renewal_needs_a_new_sign_in() {
        assert!(!is_unreachable(&Error::Auth("refresh_token rejected (invalid_grant)".into())));
        assert!(!is_unreachable(&Error::Keyring("read failed".into())));
    }

    #[test]
    fn a_server_that_could_not_answer_leaves_the_account_unknown() {
        assert!(is_unreachable(&Error::HttpRequestRejected(503, String::new())));
        assert!(is_unreachable(&Error::RateLimitExceeded("api.minecraftservices.com".into())));
    }
}
