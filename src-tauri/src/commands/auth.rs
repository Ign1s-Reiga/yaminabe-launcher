use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use log::warn;
use qrcode::render::svg;
use qrcode::QrCode;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tokio::time::sleep;
use yaminabe_launcher_shared::datatypes::AccountSummary;
use yaminabe_launcher_shared::error::Error;
use yaminabe_launcher_shared::ipc::{MsLoginPrompt, MsLoginResult};

use crate::{accounts_path, AppState};

/// Build-time Azure Application (client) ID for the OAuth 2.0 device-code
/// grant. `start_microsoft_login` refuses up-front with a clear error if the
/// env var was unset at build time.
const AZURE_CLIENT_ID: Option<&str> = option_env!("YAMINABE_AZURE_CLIENT_ID");

/// `XboxLive.signin` is mandatory to exchange a Microsoft token for an Xbox
/// Live token; `offline_access` returns a refresh_token for silent renewal.
const SCOPE: &str = "XboxLive.signin offline_access";

const DEVICE_CODE_URL: &str =
    "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode";
const TOKEN_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
const XBL_AUTH_URL: &str = "https://user.auth.xboxlive.com/user/authenticate";
const XSTS_AUTH_URL: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
const MC_LOGIN_URL: &str = "https://api.minecraftservices.com/authentication/login_with_xbox";
const MC_PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";

const EVT_PROMPT: &str = "ms-login-prompt";
const EVT_RESULT: &str = "ms-login-result";

/// One persisted Microsoft / Minecraft account. The launch command refreshes
/// `mc_access_token` via `ms_refresh_token` once `mc_expires_at` is reached.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MinecraftAccount {
    pub uuid: String,
    pub username: String,
    pub mc_access_token: String,
    /// Epoch seconds (UTC) at which `mc_access_token` ceases to be valid.
    pub mc_expires_at: i64,
    pub ms_refresh_token: String,
    /// XUID surfaced to the game as `${auth_xuid}`. Optional because the XSTS
    /// `DisplayClaims.xui[0].xid` field is only populated for Xbox Live users.
    #[serde(default)]
    pub xuid: String,
}

impl MinecraftAccount {
    /// `${auth_uuid}` needs a dashed UUID; the profile endpoint returns it raw.
    pub(crate) fn uuid_dashed(&self) -> String {
        format_uuid_dashed(&self.uuid)
    }

    fn summary(&self) -> AccountSummary {
        AccountSummary {
            uuid: self.uuid_dashed(),
            username: self.username.clone(),
        }
    }
}

/// File-backed container for the accounts list plus the currently selected
/// account. Wrapped in a `Mutex` on `AppState`; persisted as JSON.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountStore {
    #[serde(default)]
    pub accounts: Vec<MinecraftAccount>,
    /// Stored as the dash-stripped UUID (same format as Mojang returns).
    #[serde(default)]
    pub selected: Option<String>,
}

impl AccountStore {
    pub(crate) fn save(&self) -> Result<(), Error> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(accounts_path(), json)?;
        Ok(())
    }

    fn upsert(&mut self, account: MinecraftAccount) {
        if let Some(existing) = self.accounts.iter_mut().find(|a| a.uuid == account.uuid) {
            *existing = account;
        } else {
            // A new account becomes selected if nothing else is — first login
            // immediately becomes the launch identity.
            if self.selected.is_none() {
                self.selected = Some(account.uuid.clone());
            }
            self.accounts.push(account);
        }
    }
}

fn now_epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn format_uuid_dashed(undashed: &str) -> String {
    if undashed.len() != 32 || undashed.contains('-') {
        return undashed.to_string();
    }
    format!(
        "{}-{}-{}-{}-{}",
        &undashed[0..8],
        &undashed[8..12],
        &undashed[12..16],
        &undashed[16..20],
        &undashed[20..32]
    )
}

fn make_qr_svg(text: &str) -> Result<String, Error> {
    let code = QrCode::new(text.as_bytes())
        .map_err(|e| Error::Auth(format!("QR generation: {e}")))?;
    Ok(code
        .render::<svg::Color>()
        .min_dimensions(220, 220)
        .quiet_zone(true)
        .dark_color(svg::Color("#111111"))
        .light_color(svg::Color("#ffffff"))
        .build())
}

fn require_client_id() -> Result<&'static str, Error> {
    AZURE_CLIENT_ID
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            Error::Auth(
                "YAMINABE_AZURE_CLIENT_ID was not set at build time — \
                 register an Azure app, then rebuild with the env var set."
                    .to_string(),
            )
        })
}

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

#[derive(Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u32,
    interval: u32,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
}

#[derive(Deserialize)]
struct TokenErrorResponse {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

#[derive(Deserialize)]
struct XboxAuthResponse {
    #[serde(rename = "Token")]
    token: String,
    #[serde(rename = "DisplayClaims")]
    display_claims: XboxDisplayClaims,
}

#[derive(Deserialize)]
struct XboxDisplayClaims {
    xui: Vec<XboxXui>,
}

#[derive(Deserialize)]
struct XboxXui {
    uhs: String,
    #[serde(default)]
    xid: Option<String>,
}

#[derive(Deserialize)]
struct XstsErrorResponse {
    #[serde(rename = "XErr")]
    xerr: Option<i64>,
    #[serde(rename = "Message")]
    message: Option<String>,
}

#[derive(Deserialize)]
struct McLoginResponse {
    access_token: String,
    expires_in: u32,
}

#[derive(Deserialize)]
struct McProfile {
    id: String,
    name: String,
}

async fn request_device_code(client: &Client, client_id: &str) -> Result<DeviceCodeResponse, Error> {
    let resp = client
        .post(DEVICE_CODE_URL)
        .form(&[("client_id", client_id), ("scope", SCOPE)])
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        return Err(Error::Auth(format!(
            "device code request rejected ({status}): {body}"
        )));
    }
    resp.json::<DeviceCodeResponse>()
        .await
        .map_err(Error::InvalidResponse)
}

/// Poll the token endpoint. `Ok(Some)` is a real grant, `Ok(None)` means MS
/// asked us to keep waiting, `Err` is non-recoverable.
async fn poll_token(
    client: &Client,
    client_id: &str,
    device_code: &str,
) -> Result<Option<TokenResponse>, Error> {
    let resp = client
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("client_id", client_id),
            ("device_code", device_code),
        ])
        .send()
        .await?;

    if resp.status().is_success() {
        return Ok(Some(
            resp.json::<TokenResponse>()
                .await
                .map_err(Error::InvalidResponse)?,
        ));
    }

    let err = resp
        .json::<TokenErrorResponse>()
        .await
        .map_err(Error::InvalidResponse)?;
    match err.error.as_str() {
        // RFC 8628 §3.5: keep polling.
        "authorization_pending" | "slow_down" => Ok(None),
        "authorization_declined" | "access_denied" => Err(Error::Auth(
            "User declined the sign-in request.".to_string(),
        )),
        "expired_token" | "code_expired" => Err(Error::Auth(
            "The login request expired before authentication completed.".to_string(),
        )),
        other => Err(Error::Auth(format!(
            "Microsoft token endpoint returned {other}: {}",
            err.error_description.unwrap_or_default()
        ))),
    }
}

async fn refresh_access_token(
    client: &Client,
    client_id: &str,
    refresh_token: &str,
) -> Result<TokenResponse, Error> {
    let resp = client
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", client_id),
            ("scope", SCOPE),
            ("refresh_token", refresh_token),
        ])
        .send()
        .await?;
    if !resp.status().is_success() {
        let err = resp
            .json::<TokenErrorResponse>()
            .await
            .map_err(Error::InvalidResponse)?;
        return Err(Error::Auth(format!(
            "refresh_token rejected ({}): {}",
            err.error,
            err.error_description.unwrap_or_default()
        )));
    }
    resp.json::<TokenResponse>()
        .await
        .map_err(Error::InvalidResponse)
}

/// XBL/XSTS request body — both endpoints share the outer shape; only the
/// inner `Properties` payload differs.
#[derive(Serialize)]
struct XboxAuthRequest<'a, P: Serialize> {
    #[serde(rename = "Properties")]
    properties: P,
    #[serde(rename = "RelyingParty")]
    relying_party: &'a str,
    #[serde(rename = "TokenType")]
    token_type: &'a str,
}

#[derive(Serialize)]
struct XblProperties<'a> {
    #[serde(rename = "AuthMethod")]
    auth_method: &'a str,
    #[serde(rename = "SiteName")]
    site_name: &'a str,
    #[serde(rename = "RpsTicket")]
    rps_ticket: String,
}

#[derive(Serialize)]
struct XstsProperties<'a> {
    #[serde(rename = "SandboxId")]
    sandbox_id: &'a str,
    #[serde(rename = "UserTokens")]
    user_tokens: Vec<String>,
}

async fn xbl_authenticate(client: &Client, ms_access_token: &str) -> Result<XboxAuthResponse, Error> {
    let body = XboxAuthRequest {
        properties: XblProperties {
            auth_method: "RPS",
            site_name: "user.auth.xboxlive.com",
            rps_ticket: format!("d={ms_access_token}"),
        },
        relying_party: "http://auth.xboxlive.com",
        token_type: "JWT",
    };
    let resp = client
        .post(XBL_AUTH_URL)
        .header("Accept", "application/json")
        .json(&body)
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        return Err(Error::Auth(format!(
            "Xbox Live authentication rejected ({status}): {text}"
        )));
    }
    resp.json::<XboxAuthResponse>()
        .await
        .map_err(Error::InvalidResponse)
}

async fn xsts_authorize(client: &Client, xbl_token: &str) -> Result<XboxAuthResponse, Error> {
    let body = XboxAuthRequest {
        properties: XstsProperties {
            sandbox_id: "RETAIL",
            user_tokens: vec![xbl_token.to_string()],
        },
        relying_party: "rp://api.minecraftservices.com/",
        token_type: "JWT",
    };
    let resp = client
        .post(XSTS_AUTH_URL)
        .header("Accept", "application/json")
        .json(&body)
        .send()
        .await?;
    // XSTS uses HTTP 401 to signal specific user-facing problems via XErr.
    if resp.status().as_u16() == 401 {
        let err = resp
            .json::<XstsErrorResponse>()
            .await
            .map_err(Error::InvalidResponse)?;
        return Err(Error::Auth(xsts_error_message(err.xerr, err.message)));
    }
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        return Err(Error::Auth(format!(
            "XSTS authorization rejected ({status}): {text}"
        )));
    }
    resp.json::<XboxAuthResponse>()
        .await
        .map_err(Error::InvalidResponse)
}

fn xsts_error_message(xerr: Option<i64>, raw_message: Option<String>) -> String {
    match xerr {
        Some(2148916233) => "This Microsoft account has no Xbox profile. Create one at https://xbox.com first, then try again.".into(),
        Some(2148916235) => "Xbox Live is not available in your account's country/region.".into(),
        Some(2148916236) | Some(2148916237) => "Your account needs adult verification before it can sign in.".into(),
        Some(2148916238) => "This account is a child profile and must be added to a Family by an adult.".into(),
        Some(code) => format!(
            "Xbox Live rejected the account (XErr {code}{}).",
            raw_message.map(|m| format!(": {m}")).unwrap_or_default()
        ),
        None => "Xbox Live rejected the account.".into(),
    }
}

async fn mc_login_with_xbox(
    client: &Client,
    user_hash: &str,
    xsts_token: &str,
) -> Result<McLoginResponse, Error> {
    #[derive(Serialize)]
    struct Body {
        #[serde(rename = "identityToken")]
        identity_token: String,
    }
    let resp = client
        .post(MC_LOGIN_URL)
        .json(&Body {
            identity_token: format!("XBL3.0 x={user_hash};{xsts_token}"),
        })
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        return Err(Error::Auth(format!(
            "Minecraft services rejected the Xbox token ({status}): {text}"
        )));
    }
    resp.json::<McLoginResponse>()
        .await
        .map_err(Error::InvalidResponse)
}

async fn mc_fetch_profile(client: &Client, mc_access_token: &str) -> Result<McProfile, Error> {
    let resp = client
        .get(MC_PROFILE_URL)
        .bearer_auth(mc_access_token)
        .send()
        .await?;
    if resp.status().as_u16() == 404 {
        return Err(Error::Auth(
            "Signed-in Microsoft account does not own Minecraft: Java Edition.".into(),
        ));
    }
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        return Err(Error::Auth(format!(
            "Profile lookup rejected ({status}): {text}"
        )));
    }
    resp.json::<McProfile>()
        .await
        .map_err(Error::InvalidResponse)
}

/// Run the XBL → XSTS → MC → profile chain for an MS access token. Used by
/// both the initial device-code flow and the refresh path on launch.
async fn finalize_minecraft_login(
    client: &Client,
    ms_access_token: &str,
) -> Result<(McProfile, McLoginResponse, String), Error> {
    let xbl = xbl_authenticate(client, ms_access_token).await?;
    let user_hash = xbl
        .display_claims
        .xui
        .first()
        .map(|x| x.uhs.clone())
        .ok_or_else(|| Error::Auth("XBL response missing user hash".into()))?;

    let xsts = xsts_authorize(client, &xbl.token).await?;
    let xuid = xsts
        .display_claims
        .xui
        .first()
        .and_then(|x| x.xid.clone())
        .unwrap_or_default();

    let mc = mc_login_with_xbox(client, &user_hash, &xsts.token).await?;
    let profile = mc_fetch_profile(client, &mc.access_token).await?;
    Ok((profile, mc, xuid))
}

/// Refresh + chain-exchange an existing account in place. Called from
/// `launch_instance` when the MC token has expired (or is about to).
pub async fn refresh_account_tokens(
    client: &Client,
    account: &mut MinecraftAccount,
) -> Result<(), Error> {
    let client_id = require_client_id()?;
    let token = refresh_access_token(client, client_id, &account.ms_refresh_token).await?;
    let (profile, mc, xuid) = finalize_minecraft_login(client, &token.access_token).await?;
    account.uuid = profile.id;
    account.username = profile.name;
    account.mc_access_token = mc.access_token;
    account.mc_expires_at = now_epoch_seconds() + mc.expires_in as i64;
    account.ms_refresh_token = token.refresh_token;
    account.xuid = xuid;
    Ok(())
}

#[tauri::command]
pub fn get_accounts(state: State<'_, AppState>) -> Vec<AccountSummary> {
    state
        .accounts
        .lock()
        .unwrap()
        .accounts
        .iter()
        .map(MinecraftAccount::summary)
        .collect()
}

#[tauri::command]
pub fn get_selected_account(state: State<'_, AppState>) -> Option<String> {
    state
        .accounts
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
    let mut store = state.accounts.lock().unwrap();
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
    let mut store = state.accounts.lock().unwrap();
    let normalised = uuid.replace('-', "");
    store.accounts.retain(|a| a.uuid != normalised);
    if store.selected.as_deref() == Some(normalised.as_str()) {
        // Fall back to the first remaining account rather than `None` so a
        // user who deleted the selected entry still has a launch identity.
        store.selected = store.accounts.first().map(|a| a.uuid.clone());
    }
    store.save()
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
    // device code expires. `interval` is honoured plus bumps on `slow_down`.
    let started = SystemTime::now();
    let mut interval = device.interval.max(1) as u64;
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
            let msg = "The login request expired before authentication completed.";
            emit_result(&app, "expired", msg, None);
            return Err(Error::Auth(msg.into()));
        }
        sleep(Duration::from_secs(interval)).await;
        match poll_token(&client, client_id, &device.device_code).await {
            Ok(Some(t)) => break t,
            Ok(None) => {
                interval = interval.saturating_add(1).min(30);
                continue;
            }
            Err(e) => {
                emit_result(&app, "error", e.to_string(), None);
                return Err(e);
            }
        }
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
        mc_expires_at: now_epoch_seconds() + mc.expires_in as i64,
        ms_refresh_token: token.refresh_token,
        xuid,
    };
    let summary = account.summary();

    {
        let mut store = state.accounts.lock().unwrap();
        store.upsert(account);
        if let Err(e) = store.save() {
            warn!("failed to persist accounts.json: {e}");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dashes_undashed_uuid() {
        assert_eq!(
            format_uuid_dashed("12345678abcd1234abcd1234567890ab"),
            "12345678-abcd-1234-abcd-1234567890ab"
        );
    }

    #[test]
    fn passes_through_already_dashed() {
        let already = "12345678-abcd-1234-abcd-1234567890ab";
        assert_eq!(format_uuid_dashed(already), already);
    }
}