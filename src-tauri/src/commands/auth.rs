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

/// Keyring service name used for every per-account credential. Each account
/// has two entries under this service — `<uuid>:mc_access` and
/// `<uuid>:ms_refresh` — because Windows DPAPI caps a single credential
/// blob at 2560 UTF-16 chars, which a combined JSON of the two tokens
/// frequently exceeds.
const KEYRING_SERVICE: &str = "yaminabe-launcher";
const MC_ACCESS_SUFFIX: &str = ":mc_access";
const MS_REFRESH_SUFFIX: &str = ":ms_refresh";

/// In-memory composite an account is hydrated to whenever the auth code needs
/// to actually mint or use tokens. Secrets live in the OS keyring; the
/// non-secret fields live in `accounts.json` via `MinecraftAccountRecord`.
#[derive(Debug, Clone)]
pub struct MinecraftAccount {
    pub uuid: String,
    pub username: String,
    pub mc_access_token: String,
    /// Epoch seconds (UTC) at which `mc_access_token` ceases to be valid.
    /// The MS refresh token is rotated by Microsoft on each refresh and has
    /// its own (~90-day) lifetime tracked server-side, so we don't store an
    /// expiry for it here.
    pub expires_at: i64,
    pub ms_refresh_token: String,
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

/// Non-secret portion of an account; this is what gets serialized to
/// `accounts.json`. The two token fields move into the OS keyring under
/// `service=KEYRING_SERVICE, user=<uuid>` as a small JSON payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MinecraftAccountRecord {
    pub uuid: String,
    pub username: String,
    #[serde(alias = "mcExpiresAt")]
    pub expires_at: i64,
    #[serde(default)]
    pub xuid: String,
}

impl MinecraftAccountRecord {
    fn from_account(account: &MinecraftAccount) -> Self {
        Self {
            uuid: account.uuid.clone(),
            username: account.username.clone(),
            expires_at: account.expires_at,
            xuid: account.xuid.clone(),
        }
    }

    pub(crate) fn summary(&self) -> AccountSummary {
        AccountSummary {
            uuid: format_uuid_dashed(&self.uuid),
            username: self.username.clone(),
        }
    }
}

/// Secret payload written into the OS keyring as a single JSON blob.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct MinecraftAccountSecret {
    pub mc_access_token: String,
    pub ms_refresh_token: String,
}

/// File-backed container for the records list plus the currently selected
/// account. Wrapped in a `Mutex` on `AppState`; persisted as JSON.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountStore {
    #[serde(default)]
    pub accounts: Vec<MinecraftAccountRecord>,
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

    fn upsert(&mut self, record: MinecraftAccountRecord) {
        if let Some(existing) = self.accounts.iter_mut().find(|a| a.uuid == record.uuid) {
            *existing = record;
        } else {
            // A new account becomes selected if nothing else is — first login
            // immediately becomes the launch identity.
            if self.selected.is_none() {
                self.selected = Some(record.uuid.clone());
            }
            self.accounts.push(record);
        }
    }
}

/// Pre-keyring on-disk shape; only used during the one-shot migration that
/// moves tokens out of `accounts.json` into the OS keyring.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyAccountStore {
    #[serde(default)]
    accounts: Vec<LegacyMinecraftAccount>,
    #[serde(default)]
    selected: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyMinecraftAccount {
    uuid: String,
    username: String,
    #[serde(default)]
    mc_access_token: String,
    #[serde(default)]
    mc_expires_at: i64,
    #[serde(default)]
    ms_refresh_token: String,
    #[serde(default)]
    xuid: String,
}

/// Read `accounts.json` and migrate any inline tokens to the keyring. Called
/// once at startup by `lib.rs`. A missing or malformed file falls back to an
/// empty store so a corrupted record doesn't block app launch.
pub fn load_account_store() -> AccountStore {
    let Ok(text) = std::fs::read_to_string(accounts_path()) else {
        return AccountStore::default();
    };
    let legacy: LegacyAccountStore = match serde_json::from_str(&text) {
        Ok(s) => s,
        Err(e) => {
            warn!("accounts.json is malformed ({e}); starting with empty account list");
            return AccountStore::default();
        }
    };

    let mut migrated_any = false;
    let mut accounts = Vec::with_capacity(legacy.accounts.len());
    for la in legacy.accounts {
        if !la.mc_access_token.is_empty() || !la.ms_refresh_token.is_empty() {
            let secret = MinecraftAccountSecret {
                mc_access_token: la.mc_access_token,
                ms_refresh_token: la.ms_refresh_token,
            };
            match write_secret(&la.uuid, &secret) {
                Ok(()) => migrated_any = true,
                Err(e) => warn!("failed to migrate tokens for {}: {e}", la.uuid),
            }
        }
        accounts.push(MinecraftAccountRecord {
            uuid: la.uuid,
            username: la.username,
            expires_at: la.mc_expires_at,
            xuid: la.xuid,
        });
    }
    let store = AccountStore { accounts, selected: legacy.selected };
    if migrated_any
        && let Err(e) = store.save()
    {
        warn!("failed to rewrite accounts.json after token migration: {e}");
    }
    store
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

fn write_secret(uuid: &str, secret: &MinecraftAccountSecret) -> Result<(), Error> {
    write_one(&format!("{uuid}{MC_ACCESS_SUFFIX}"), &secret.mc_access_token)?;
    write_one(&format!("{uuid}{MS_REFRESH_SUFFIX}"), &secret.ms_refresh_token)?;
    Ok(())
}

fn delete_secret(uuid: &str) -> Result<(), Error> {
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
    account.expires_at = now_epoch_seconds() + mc.expires_in as i64;
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
        .map(MinecraftAccountRecord::summary)
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
        expires_at: now_epoch_seconds() + mc.expires_in as i64,
        ms_refresh_token: token.refresh_token,
        xuid,
    };
    let summary = account.summary();

    // Login is considered complete the moment the Minecraft profile fetch
    // returns: upsert the record, save accounts.json, and emit the success
    // event so the settings UI reflects the new account immediately. The
    // keyring write below is allowed to fail without rolling back any of
    // this — a missing keyring entry on next launch surfaces a clear
    // "Re-add the account" message via `hydrate_account`.
    {
        let mut store = state.accounts.lock().unwrap();
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

    let secret = MinecraftAccountSecret {
        mc_access_token: account.mc_access_token,
        ms_refresh_token: account.ms_refresh_token,
    };
    if let Err(e) = write_secret(&account.uuid, &secret) {
        warn!("failed to write keyring secret for {}: {e}", account.uuid);
    }
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