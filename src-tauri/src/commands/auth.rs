use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use log::warn;
use tauri::{AppHandle, Emitter, State};
use tokio::time::sleep;
use yaminabe_launcher_shared::datamodels::{AccountSession, AccountSummary};
use yaminabe_launcher_shared::error::Error;
use yaminabe_launcher_shared::ipc::{MsLoginPrompt, MsLoginResult, EVT_MS_LOGIN_PROMPT, EVT_MS_LOGIN_RESULT};
use crate::AppState;

use crate::auth::flow::{
    make_qr_svg, minecraft_sign_in, poll_token, request_device_code, require_client_id, Poll, PollPace,
};
use crate::auth::model::{format_uuid_dashed, MinecraftAccountRecord};
use crate::auth::session::{renew_session, save_renewal, Renewal};
use crate::auth::store::{delete_secret, save_record, write_account_secret};
use crate::auth::hydrate_account;

fn emit_result(app: &AppHandle, kind: &str, message: impl Into<String>, account: Option<AccountSummary>) {
    let payload = MsLoginResult {
        kind: kind.to_string(),
        message: message.into(),
        account,
    };
    if let Err(e) = app.emit(EVT_MS_LOGIN_RESULT, payload) {
        warn!("failed to emit {EVT_MS_LOGIN_RESULT}: {e}");
    }
}

#[tauri::command]
pub fn get_accounts(state: State<'_, AppState>) -> Vec<AccountSummary> {
    state
        .account_store
        .lock()
        .unwrap()
        .accounts
        .iter()
        .map(MinecraftAccountRecord::summary)
        .collect()
}

#[tauri::command]
pub fn get_selected_account(state: State<'_, AppState>) -> Option<String> {
    state
        .account_store
        .lock()
        .unwrap()
        .selected
        .as_ref()
        .map(|s| format_uuid_dashed(s))
}

#[tauri::command]
pub fn set_selected_account(
    uuid: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), Error> {
    let mut store = state.account_store.lock().unwrap();
    let normalised = uuid.map(|u| u.replace('-', ""));
    if let Some(target) = &normalised
        && !store.accounts.iter().any(|a| &a.uuid == target)
    {
        return Err(Error::NotExists(format!("account '{target}'")));
    }
    store.selected = normalised;
    store.save()
}

#[tauri::command]
pub fn remove_account(uuid: String, state: State<'_, AppState>) -> Result<(), Error> {
    let mut store = state.account_store.lock().unwrap();
    let normalised = uuid.replace('-', "");
    let had_record = store.accounts.iter().any(|a| a.uuid == normalised);
    store.accounts.retain(|a| a.uuid != normalised);
    if store.selected.as_deref() == Some(normalised.as_str()) {
        // Fall back to the first remaining account rather than `None` so a
        // user who deleted the selected entry still has a launch identity.
        store.selected = store.accounts.first().map(|a| a.uuid.clone());
    }
    if had_record
        && let Err(e) = delete_secret(&normalised)
    {
        warn!("failed to delete keyring entry for {normalised}: {e}");
    }
    store.save()
}

/// Bring a saved account's sign-in up to date, as a launch would, and say
/// whether the user has to sign in again. Renewed tokens and a new player name
/// are saved, unless the account was removed or signed in again meanwhile.
#[tauri::command]
pub async fn check_account_session(
    uuid: String,
    state: State<'_, AppState>,
) -> Result<AccountSession, Error> {
    let normalised = uuid.replace('-', "");
    let record = state
        .account_store
        .lock()
        .unwrap()
        .accounts
        .iter()
        .find(|a| a.uuid == normalised)
        .cloned()
        .ok_or_else(|| Error::NotExists(format!("account '{normalised}'")))?;
    let mut account = match hydrate_account(&record) {
        Ok(account) => account,
        Err(e) => return Ok(AccountSession::NeedsSignIn(format!("The saved sign-in could not be read: {e}"))),
    };
    match renew_session(&state.http_client, &mut account, |_| {}).await {
        Ok(Renewal::Current) => Ok(AccountSession::Valid),
        Ok(renewal @ (Renewal::Updated | Renewal::Renamed)) => {
            let with_secret = matches!(renewal, Renewal::Updated);
            let saved = save_renewal(&mut state.account_store.lock().unwrap(), &account, &record, with_secret);
            if let Some(e) = saved.failure {
                warn!("failed to save the renewed sign-in for {normalised}: {e}");
            }
            Ok(if saved.renamed { AccountSession::Renamed } else { AccountSession::Valid })
        }
        Ok(Renewal::Unchecked(e)) => {
            warn!("could not check the sign-in for {normalised}: {e}");
            Ok(AccountSession::Unchecked)
        }
        Err(e @ Error::GameNotOwned) => Ok(AccountSession::NotOwned(e.to_string())),
        Err(e) if e.is_inconclusive() => {
            warn!("could not renew the sign-in for {normalised}: {e}");
            Ok(AccountSession::Unchecked)
        }
        Err(e) => Ok(AccountSession::NeedsSignIn(e.to_string())),
    }
}

#[tauri::command]
pub async fn cancel_microsoft_login(state: State<'_, AppState>) -> Result<(), Error> {
    state.ms_login_cancel.store(true, Ordering::SeqCst);
    Ok(())
}

/// How a device-code sign-in ended, other than with an error.
enum LoginEnd {
    SignedIn(AccountSummary),
    Cancelled,
    Expired,
}

/// Drive the device-code grant from start to finish. Emits `ms-login-prompt`
/// once the device code has been registered and `ms-login-result` exactly
/// once at the end regardless of outcome.
#[tauri::command]
pub async fn start_microsoft_login(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), Error> {
    match device_code_sign_in(&app, &state).await {
        Ok(LoginEnd::SignedIn(summary)) => {
            emit_result(&app, "success", format!("Signed in as {}.", summary.username), Some(summary));
            Ok(())
        }
        Ok(LoginEnd::Cancelled) => {
            emit_result(&app, "cancelled", "Login cancelled.", None);
            Ok(())
        }
        Ok(LoginEnd::Expired) => {
            let msg = "The login request expired before authentication completed.";
            emit_result(&app, "expired", msg, None);
            Err(Error::Auth(msg.into()))
        }
        Err(e) => {
            emit_result(&app, "error", e.to_string(), None);
            Err(e)
        }
    }
}

async fn device_code_sign_in(app: &AppHandle, state: &AppState) -> Result<LoginEnd, Error> {
    let client_id = require_client_id()?;
    state.ms_login_cancel.store(false, Ordering::SeqCst);
    let client = &state.http_client;

    let device = request_device_code(client, client_id).await?;
    let qr_svg = make_qr_svg(&device.verification_uri)?;
    if let Err(e) = app.emit(
        EVT_MS_LOGIN_PROMPT,
        MsLoginPrompt {
            verification_uri: device.verification_uri.clone(),
            user_code: device.user_code.clone(),
            qr_svg,
            expires_in: device.expires_in,
            interval: device.interval,
        },
    ) {
        warn!("failed to emit {EVT_MS_LOGIN_PROMPT}: {e}");
    }

    // Poll until the token endpoint returns a grant, the user cancels, or the
    // device code expires, at the pace `PollPace` sets. `Instant` keeps a
    // change to the system clock from ending the sign-in early.
    let expires_at = Instant::now() + Duration::from_secs(u64::from(device.expires_in));
    let mut pace = PollPace::new(device.interval);
    let mut waited = 0u64;
    let token = loop {
        if state.ms_login_cancel.load(Ordering::SeqCst) {
            return Ok(LoginEnd::Cancelled);
        }
        if Instant::now() >= expires_at {
            return Ok(LoginEnd::Expired);
        }
        // Wait in one-second steps, so a cancel or the code's expiry is seen
        // within a second rather than after a whole wait.
        if waited < pace.wait() {
            sleep(Duration::from_secs(1)).await;
            waited += 1;
            continue;
        }
        waited = 0;
        match poll_token(client, client_id, &device.device_code).await {
            Ok(Poll::Granted(t)) => break t,
            Ok(Poll::Pending) => pace.answered(),
            Ok(Poll::SlowDown) => pace.slow_down(),
            Ok(Poll::Throttled(retry_after)) => {
                warn!("token poll turned away for coming too often, resting {retry_after:?} s");
                pace.unclear(retry_after);
            }
            Ok(Poll::Expired) => return Ok(LoginEnd::Expired),
            // One poll that gets no clear answer does not end the sign-in;
            // the code expiring does.
            Err(e) if e.is_inconclusive() => {
                warn!("token poll got no clear answer, polling again later: {e}");
                pace.unclear(None);
            }
            Err(e) => return Err(e),
        }
    };

    let account = minecraft_sign_in(client, token).await?;
    // Keyring write must succeed before login is reported as successful:
    // otherwise the account is listed but its next launch finds no tokens.
    if let Err(e) = write_account_secret(&account) {
        warn!("keyring write failed for {}: {e}", account.uuid);
        return Err(e);
    }
    if let Err(e) = save_record(&mut state.account_store.lock().unwrap(), &account) {
        warn!("failed to save accounts.json after login: {e}");
    }
    Ok(LoginEnd::SignedIn(account.summary()))
}
