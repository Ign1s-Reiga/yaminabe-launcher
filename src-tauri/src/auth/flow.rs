use qrcode::render::svg;
use qrcode::QrCode;
use std::time::Duration;
use reqwest::{Client, Method, Response};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use yaminabe_launcher_shared::error::Error;
use crate::http_utils::{fetch_json, rejected_status};
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

/// How long a sign-in request may take. The shared client has no timeout of its
/// own, so a network that never answers would otherwise hold a launch or a
/// check for good.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a profile lookup may take. Every launch makes one, so it gets less
/// time than the other requests.
const PROFILE_TIMEOUT: Duration = Duration::from_secs(10);

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

#[derive(Debug, Deserialize)]
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
        .timeout(REQUEST_TIMEOUT)
        .send::<DeviceCodeResponse>()
        .await
        .map_err(|e| refused("device code request", e))
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
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await?;

    if resp.status().is_success() {
        return Ok(Some(
            resp.json::<TokenResponse>()
                .await
                .map_err(Error::InvalidResponse)?,
        ));
    }

    let err = service_error::<TokenErrorResponse>(resp, TOKEN_URL).await?;
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
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await?;
    if !resp.status().is_success() {
        let err = service_error::<TokenErrorResponse>(resp, TOKEN_URL).await?;
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

/// Read a failed reply from an endpoint that reports its errors as a `T`.
/// A status that says the server could not answer is passed on first. A body
/// that is not a `T`, such as a proxy's page or one cut off, did not come from
/// the service, so it is inconclusive too.
async fn service_error<T: DeserializeOwned>(resp: Response, url: &str) -> Result<T, Error> {
    let status = resp.status();
    if status.is_server_error() || matches!(status.as_u16(), 407 | 408 | 429) {
        return Err(rejected_status(status, url));
    }
    resp.json::<T>().await.map_err(Error::InvalidResponse)
}

/// A sign-in step the server refused is an authentication failure. An
/// inconclusive error is passed on as it is, since it says nothing about the
/// account.
fn refused(step: &str, e: Error) -> Error {
    if e.is_inconclusive() {
        e
    } else {
        Error::Auth(format!("{step}: {e}"))
    }
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
        .timeout(REQUEST_TIMEOUT)
        .send::<XboxAuthResponse>()
        .await
        .map_err(|e| refused("Xbox Live authentication", e))
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
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await?;
    // XSTS uses HTTP 401 to signal specific user-facing problems via XErr.
    if resp.status().as_u16() == 401 {
        let err = service_error::<XstsErrorResponse>(resp, XSTS_AUTH_URL).await?;
        return Err(Error::Auth(xsts_error_message(err.xerr, err.message)));
    }
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        return Err(refused("XSTS authorization", Error::HttpRequestRejected(status, text)));
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
        .timeout(REQUEST_TIMEOUT)
        .send::<McLoginResponse>()
        .await
        .map_err(|e| refused("Minecraft services rejected the Xbox token", e))
}

/// Look up the profile `mc_access_token` belongs to. `Error::GameNotOwned`
/// means the account cannot play at all; a refusal comes back as
/// `HttpRequestRejected`, carrying the server's status and reason.
async fn mc_fetch_profile(client: &Client, mc_access_token: &str) -> Result<McProfile, Error> {
    profile_lookup(client, MC_PROFILE_URL, mc_access_token, PROFILE_TIMEOUT).await
}

async fn profile_lookup(
    client: &Client,
    url: &str,
    mc_access_token: &str,
    timeout: Duration,
) -> Result<McProfile, Error> {
    let resp = client
        .get(url)
        .bearer_auth(mc_access_token)
        .timeout(timeout)
        .send()
        .await?;
    if resp.status().as_u16() == 404 {
        return Err(Error::GameNotOwned);
    }
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        return Err(Error::HttpRequestRejected(status, text));
    }
    resp.json::<McProfile>()
        .await
        .map_err(Error::InvalidResponse)
}

/// What Minecraft services make of a saved access token.
#[derive(Debug)]
pub(crate) enum Session {
    /// The token works, for the profile it carries.
    Valid(McProfile),
    /// The token was turned down, so it has to be refreshed before it is used.
    Rejected,
    /// The services could not be asked, so the token is used as saved.
    Unchecked(Error),
}

/// Ask Minecraft services whether `mc_access_token` still works. A token can
/// stop working before its saved expiry, e.g. when the account's sign-ins are
/// revoked, and the game would then start without being able to join servers.
pub(crate) async fn check_session(client: &Client, mc_access_token: &str) -> Result<Session, Error> {
    session_from_lookup(mc_fetch_profile(client, mc_access_token).await)
}

fn session_from_lookup(lookup: Result<McProfile, Error>) -> Result<Session, Error> {
    match lookup {
        Ok(profile) => Ok(Session::Valid(profile)),
        Err(Error::HttpRequestRejected(401 | 403, _)) => Ok(Session::Rejected),
        Err(e @ Error::GameNotOwned) => Err(e),
        // Any other answer says nothing sure about the token.
        Err(e) => Ok(Session::Unchecked(e)),
    }
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
    let profile = mc_fetch_profile(client, &mc.access_token)
        .await
        .map_err(login_profile_error)?;
    // The XSTS response's DisplayClaims.xui[0].xid is unreliable for the
    // Minecraft relying party (frequently absent), so the authoritative
    // source for ${auth_xuid} is the MC access token's JWT payload.
    let xuid = xuid_from_mc_token(&mc.access_token).unwrap_or_default();
    Ok((profile, mc, xuid))
}

/// A profile lookup refused during login or refresh, with a 401 or 403, is an
/// authentication failure, reported with the server's status and reason.
/// Anything else is passed on as it is.
fn login_profile_error(e: Error) -> Error {
    match e {
        Error::HttpRequestRejected(status @ (401 | 403), text) => {
            Error::Auth(format!("Profile lookup rejected ({status}): {text}"))
        }
        other => other,
    }
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

#[cfg(test)]
mod tests {
    use super::{
        login_profile_error, profile_lookup, refused, service_error, session_from_lookup, McProfile, Session,
        TokenErrorResponse,
    };
    use std::time::{Duration, Instant};
    use tokio::net::TcpListener;
    use yaminabe_launcher_shared::error::Error;

    #[test]
    fn a_turned_down_token_is_rejected() {
        for status in [401, 403] {
            let lookup = Err(Error::HttpRequestRejected(status, String::new()));
            assert!(matches!(session_from_lookup(lookup), Ok(Session::Rejected)));
        }
    }

    #[test]
    fn an_unexpected_answer_leaves_the_token_unchecked() {
        for status in [400, 405, 429] {
            let lookup = Err(Error::HttpRequestRejected(status, String::new()));
            assert!(matches!(session_from_lookup(lookup), Ok(Session::Unchecked(_))), "{status}");
        }
    }

    #[test]
    fn a_working_token_carries_its_profile() {
        let profile = McProfile { id: "abc".into(), name: "Steve".into() };
        let session = session_from_lookup(Ok(profile));
        assert!(matches!(session, Ok(Session::Valid(p)) if p.name == "Steve"));
    }

    #[test]
    fn a_server_failure_leaves_the_token_unchecked() {
        let lookup = Err(Error::HttpRequestRejected(503, String::new()));
        assert!(matches!(session_from_lookup(lookup), Ok(Session::Unchecked(_))));
    }

    #[test]
    fn an_account_without_the_game_fails() {
        let lookup = Err(Error::GameNotOwned);
        assert!(matches!(session_from_lookup(lookup), Err(Error::GameNotOwned)));
    }

    #[test]
    fn a_refused_login_keeps_the_server_reason() {
        let e = login_profile_error(Error::HttpRequestRejected(403, "account restricted".into()));
        assert!(matches!(&e, Error::Auth(m) if m == "Profile lookup rejected (403): account restricted"), "got {e:?}");
    }

    #[test]
    fn a_login_the_server_could_not_answer_is_not_a_refusal() {
        for status in [400, 405, 429, 503] {
            let e = login_profile_error(Error::HttpRequestRejected(status, String::new()));
            assert!(matches!(e, Error::HttpRequestRejected(..)), "got {e:?}");
        }
    }

    #[test]
    fn a_refused_step_is_an_authentication_failure() {
        let e = refused("Xbox Live authentication", Error::HttpRequestRejected(401, "url".into()));
        assert!(matches!(&e, Error::Auth(m) if m.starts_with("Xbox Live authentication: ")), "got {e:?}");
    }

    #[test]
    fn a_step_the_server_could_not_answer_is_passed_on() {
        let e = refused("Xbox Live authentication", Error::RateLimitExceeded("url".into()));
        assert!(matches!(e, Error::RateLimitExceeded(_)), "got {e:?}");
        let e = refused("Xbox Live authentication", Error::HttpRequestRejected(502, "url".into()));
        assert!(matches!(e, Error::HttpRequestRejected(502, _)), "got {e:?}");
        let e = refused("Xbox Live authentication", Error::HttpRequestRejected(400, "url".into()));
        assert!(matches!(e, Error::HttpRequestRejected(400, _)), "got {e:?}");
    }

    #[tokio::test]
    async fn a_server_that_never_answers_leaves_the_token_unchecked() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let url = format!("http://{}/minecraft/profile", listener.local_addr().expect("addr"));
        // Accept the connection and hold it open without ever replying.
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("accept");
            tokio::time::sleep(Duration::from_secs(30)).await;
            drop(socket);
        });
        let started = Instant::now();
        let lookup = profile_lookup(&reqwest::Client::new(), &url, "token", Duration::from_millis(200)).await;
        assert!(started.elapsed() < Duration::from_secs(5), "took {:?}", started.elapsed());
        assert!(matches!(&lookup, Err(Error::NetworkError(e)) if e.is_timeout()), "got {lookup:?}");
        assert!(matches!(session_from_lookup(lookup), Ok(Session::Unchecked(_))));
        server.abort();
    }
    /// Answer one request on a local port with `reply`, a raw HTTP response.
    async fn reply_once(reply: String) -> reqwest::Response {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let url = format!("http://{}/", listener.local_addr().expect("addr"));
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut buf = [0u8; 1024];
            let read = socket.read(&mut buf).await.expect("read request");
            assert!(read > 0, "empty request");
            socket.write_all(reply.as_bytes()).await.expect("write");
        });
        reqwest::Client::new().get(&url).send().await.expect("send")
    }

    fn http_reply(status: &str, body: &str) -> String {
        format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\n\r\n{body}", body.len())
    }

    #[tokio::test]
    async fn a_service_refusal_is_read() {
        let body = r#"{"error":"invalid_grant","error_description":"expired"}"#;
        let resp = reply_once(http_reply("400 Bad Request", body)).await;
        let err = service_error::<TokenErrorResponse>(resp, "url").await.expect("refusal");
        assert_eq!(err.error, "invalid_grant");
    }

    #[tokio::test]
    async fn a_page_that_is_not_the_services_is_inconclusive() {
        let resp = reply_once(http_reply("403 Forbidden", "<html>blocked</html>")).await;
        let e = service_error::<TokenErrorResponse>(resp, "url").await.err().expect("not a refusal");
        assert!(matches!(e, Error::InvalidResponse(_)), "got {e:?}");
    }

    #[tokio::test]
    async fn a_server_error_is_inconclusive_whatever_its_body() {
        let body = r#"{"error":"temporarily_unavailable"}"#;
        let resp = reply_once(http_reply("503 Service Unavailable", body)).await;
        let e = service_error::<TokenErrorResponse>(resp, "url").await.err().expect("not a refusal");
        assert!(matches!(e, Error::HttpRequestRejected(503, _)), "got {e:?}");
    }
}
