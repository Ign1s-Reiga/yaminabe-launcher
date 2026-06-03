use qrcode::render::svg;
use qrcode::QrCode;
use reqwest::{Client, Method};
use serde::{Deserialize, Serialize};
use yaminabe_launcher_shared::error::Error;
use crate::http_utils::fetch_json;
use super::model::{now_epoch_seconds, MinecraftAccount};

/// Build-time Azure Application (client) ID for the OAuth 2.0 device-code
/// grant. `start_microsoft_login` refuses up-front with a clear error if the
/// env var was unset at build time.
const AZURE_CLIENT_ID: Option<&str> = option_env!("YAMINABE_AZURE_CLIENT_ID");

/// `XboxLive.signin` is mandatory to exchange a Microsoft token for an Xbox
/// Live token; `offline_access` returns a refresh_token for silent renewal.
const SCOPE: &str = "XboxLive.signin offline_access";

const DEVICE_CODE_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode";
const TOKEN_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
const XBL_AUTH_URL: &str = "https://user.auth.xboxlive.com/user/authenticate";
const XSTS_AUTH_URL: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
const MC_LOGIN_URL: &str = "https://api.minecraftservices.com/authentication/login_with_xbox";
const MC_PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";

#[derive(Deserialize)]
pub(crate) struct DeviceCodeResponse {
    pub(crate) device_code: String,
    pub(crate) user_code: String,
    pub(crate) verification_uri: String,
    pub(crate) expires_in: u32,
    pub(crate) interval: u32,
}

#[derive(Deserialize)]
pub(crate) struct TokenResponse {
    pub(crate) access_token: String,
    pub(crate) refresh_token: String,
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
}

#[derive(Deserialize)]
struct XstsErrorResponse {
    #[serde(rename = "XErr")]
    xerr: Option<i64>,
    #[serde(rename = "Message")]
    message: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct McLoginResponse {
    pub(crate) access_token: String,
    pub(crate) expires_in: u32,
}

#[derive(Deserialize)]
pub(crate) struct McProfile {
    pub(crate) id: String,
    pub(crate) name: String,
}

pub(crate) fn require_client_id() -> Result<&'static str, Error> {
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

pub(crate) fn make_qr_svg(text: &str) -> Result<String, Error> {
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

pub(crate) async fn request_device_code(client: &Client, client_id: &str) -> Result<DeviceCodeResponse, Error> {
    fetch_json(client, DEVICE_CODE_URL)
        .method(Method::POST)
        .form(&[("client_id", client_id), ("scope", SCOPE)])
        .send::<DeviceCodeResponse>()
        .await
        .map_err(|e| Error::Auth(format!("device code request: {e}")))
}

/// Poll the token endpoint. `Ok(Some)` is a real grant, `Ok(None)` means MS
/// asked us to keep waiting, `Err` is non-recoverable.
pub(crate) async fn poll_token(
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
    fetch_json(client, XBL_AUTH_URL)
        .method(Method::POST)
        .header("Accept", "application/json")
        .payload(&body)
        .send::<XboxAuthResponse>()
        .await
        .map_err(|e| Error::Auth(format!("Xbox Live authentication: {e}")))
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
    fetch_json(client, MC_LOGIN_URL)
        .method(Method::POST)
        .payload(&Body {
            identity_token: format!("XBL3.0 x={user_hash};{xsts_token}"),
        })
        .send::<McLoginResponse>()
        .await
        .map_err(|e| Error::Auth(format!("Minecraft services rejected the Xbox token: {e}")))
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
pub(crate) async fn finalize_minecraft_login(
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
    let mc = mc_login_with_xbox(client, &user_hash, &xsts.token).await?;
    let profile = mc_fetch_profile(client, &mc.access_token).await?;
    // The XSTS response's DisplayClaims.xui[0].xid is unreliable for the
    // Minecraft relying party (frequently absent), so the authoritative
    // source for ${auth_xuid} is the MC access token's JWT payload.
    let xuid = xuid_from_mc_token(&mc.access_token).unwrap_or_default();
    Ok((profile, mc, xuid))
}

/// Decode the unsigned middle segment of the Minecraft access token JWT and
/// pull the `xuid` claim out of it. Signature verification is the upstream
/// service's job; we only need to read public claims.
fn xuid_from_mc_token(token: &str) -> Option<String> {
    use base64::Engine;
    let payload_b64 = token.split('.').nth(1)?;
    let payload_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .ok()?;
    #[derive(Deserialize)]
    struct McTokenPayload {
        xuid: Option<String>,
    }
    serde_json::from_slice::<McTokenPayload>(&payload_bytes)
        .ok()?
        .xuid
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