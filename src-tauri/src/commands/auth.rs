use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime};

use log::warn;
use tauri::{AppHandle, Emitter, State};
use tokio::time::sleep;
use yaminabe_launcher_shared::datamodels::{AccountSession, AccountSummary};
use yaminabe_launcher_shared::error::Error;
use yaminabe_launcher_shared::ipc::{MsLoginPrompt, MsLoginResult};
use crate::AppState;

use crate::auth::flow::{finalize_minecraft_login, make_qr_svg, poll_token, request_device_code, require_client_id, Poll};
use crate::auth::model::{format_uuid_dashed, now_epoch_seconds, MinecraftAccount, MinecraftAccountRecord, MinecraftAccountSecret};
use crate::auth::session::{renew_session, save_renewal, Renewal};
use crate::auth::store::{delete_secret, write_secret};
use crate::auth::hydrate_account;

const EVT_PROMPT: &str = "ms-login-prompt";
const EVT_RESULT: &str = "ms-login-result";

/// The longest the sign-in dialog waits between two polls of the token
/// endpoint, whatever `slow_down` replies and unclear answers add up to.
const MAX_POLL_WAIT: u64 = 60;

fn emit_result(app: &AppHandle, kind: &str, message: impl Into<String>, account: Option<AccountSummary>) {
    let payload = MsLoginResult {
        kind: kind.to_string(),
        message: message.into(),
        account,
    };
    if let Err(e) = app.emit(EVT_RESULT, payload) {
        warn!("failed to emit {EVT_RESULT}: {e}");
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
            let saved = save_renewal(&mut state.account_store.lock().unwrap(), &account, &record, &renewal);
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

/// Drive the device-code grant from start to finish. Emits `ms-login-prompt`
/// once the device code has been registered and `ms-login-result` exactly
/// once at the end regardless of outcome.
#[tauri::command]
pub async fn start_microsoft_login(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), Error> {
    let client_id = match require_client_id() {
        Ok(id) => id,
        Err(e) => {
            emit_result(&app, "error", e.to_string(), None);
            return Err(e);
        }
    };

    state.ms_login_cancel.store(false, Ordering::SeqCst);
    let cancel = state.ms_login_cancel.clone();
    let client = state.http_client.clone();

    let device = match request_device_code(&client, client_id).await {
        Ok(d) => d,
        Err(e) => {
            emit_result(&app, "error", e.to_string(), None);
            return Err(e);
        }
    };

    let qr_svg = match make_qr_svg(&device.verification_uri) {
        Ok(svg) => svg,
        Err(e) => {
            emit_result(&app, "error", e.to_string(), None);
            return Err(e);
        }
    };

    if let Err(e) = app.emit(
        EVT_PROMPT,
        MsLoginPrompt {
            verification_uri: device.verification_uri.clone(),
            user_code: device.user_code.clone(),
            qr_svg,
            expires_in: device.expires_in,
            interval: device.interval,
        },
    ) {
        warn!("failed to emit {EVT_PROMPT}: {e}");
    }

    // Poll until the token endpoint returns a grant, the user cancels, or the
    // device code expires. `interval` is honoured, plus 5 s per `slow_down`;
    // `backoff` spaces out polls that get no clear answer, such as a 429.
    let expired_msg = "The login request expired before authentication completed.";
    let started = SystemTime::now();
    let mut interval = device.interval.max(1) as u64;
    let mut backoff = 0u64;
    let mut wait = interval;
    let mut waited = 0u64;
    let token = loop {
        if cancel.load(Ordering::SeqCst) {
            emit_result(&app, "cancelled", "Login cancelled.", None);
            return Ok(());
        }
        if SystemTime::now()
            .duration_since(started)
            .map(|d| d.as_secs() >= device.expires_in as u64)
            .unwrap_or(true)
        {
            emit_result(&app, "expired", expired_msg, None);
            return Err(Error::Auth(expired_msg.into()));
        }
        // Wait in one-second steps, so a cancel or the code's expiry is seen
        // within a second rather than after a whole wait.
        if waited < wait {
            sleep(Duration::from_secs(1)).await;
            waited += 1;
            continue;
        }
        waited = 0;
        match poll_token(&client, client_id, &device.device_code).await {
            Ok(Poll::Granted(t)) => break t,
            Ok(Poll::Pending) => backoff = 0,
            // RFC 8628 §3.5: a slow_down adds 5 s to every later poll.
            Ok(Poll::SlowDown) => {
                backoff = 0;
                interval = interval.saturating_add(5);
            }
            Ok(Poll::Expired) => {
                emit_result(&app, "expired", expired_msg, None);
                return Err(Error::Auth(expired_msg.into()));
            }
            // One poll that gets no clear answer does not end the sign-in;
            // the code expiring does.
            Err(e) if e.is_inconclusive() => {
                warn!("token poll got no clear answer, polling again later: {e}");
                backoff = backoff.saturating_add(5).min(MAX_POLL_WAIT);
            }
            Err(e) => {
                emit_result(&app, "error", e.to_string(), None);
                return Err(e);
            }
        }
        wait = interval.saturating_add(backoff).min(MAX_POLL_WAIT);
    };

    let (profile, mc, xuid) = match finalize_minecraft_login(&client, &token.access_token).await {
        Ok(v) => v,
        Err(e) => {
            emit_result(&app, "error", e.to_string(), None);
            return Err(e);
        }
    };

    let account = MinecraftAccount {
        uuid: profile.id,
        username: profile.name,
        mc_access_token: mc.access_token,
        expires_at: now_epoch_seconds() + mc.expires_in as i64,
        ms_refresh_token: token.refresh_token,
        xuid,
    };
    let summary = account.summary();

    // Keyring write must succeed before login is reported as successful:
    // otherwise the user sees a selectable account whose next launch fails
    // in `hydrate_account` because no tokens are persisted.
    let secret = MinecraftAccountSecret {
        mc_access_token: account.mc_access_token.clone(),
        ms_refresh_token: account.ms_refresh_token.clone(),
    };
    if let Err(e) = write_secret(&account.uuid, &secret) {
        warn!("keyring write failed for {}: {e}", account.uuid);
        emit_result(&app, "error", e.to_string(), None);
        return Err(e);
    }

    {
        let mut store = state.account_store.lock().unwrap();
        store.upsert(MinecraftAccountRecord::from_account(&account));
        if let Err(e) = store.save() {
            warn!("failed to save accounts.json after login: {e}");
        }
    }
    emit_result(
        &app,
        "success",
        format!("Signed in as {}.", summary.username),
        Some(summary),
    );
    Ok(())
}
