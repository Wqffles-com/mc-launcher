//! Microsoft account authentication: the device code flow and the Xbox Live
//! to XSTS to Minecraft token exchange chain.
//!
//! Flow overview:
//!
//! 1. `request_device_code` - ask Microsoft for a device code the user types
//!    at `microsoft.com/link`.
//! 2. `wait_for_device_approval` - poll the token endpoint until the user
//!    approves (or the code expires); yields a Microsoft access + refresh
//!    token.
//! 3. `xbl_authenticate` / `xsts_authorize` - exchange the Microsoft token
//!    for Xbox Live and XSTS tokens.
//! 4. `login_with_xbox` - trade the XSTS token for a Minecraft access token.
//! 5. `fetch_profile` - resolve the Minecraft UUID and name for launches.
//!
//! The Microsoft refresh token (step 2) can later be traded for a fresh
//! access token via `refresh_minecraft_token`, which starts the chain over at
//! step 3.
//!
//! Every function has an `_at` variant taking explicit endpoint URLs so the
//! chain can be exercised against a mock server in tests.

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Microsoft's consumer OAuth 2.0 device code endpoint.
pub const DEVICE_CODE_URL: &str =
    "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode";
/// Microsoft's consumer OAuth 2.0 token endpoint (polling + refresh).
pub const TOKEN_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
/// Xbox Live user authentication endpoint.
pub const XBL_URL: &str = "https://user.auth.xboxlive.com/user/authenticate";
/// Xbox Security Token Service authorization endpoint.
pub const XSTS_URL: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
/// Mojang's Xbox-token to Minecraft-token exchange endpoint.
pub const MC_LOGIN_URL: &str = "https://api.minecraftservices.com/authentication/login_with_xbox";
/// Mojang's authenticated player profile endpoint.
pub const PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";

/// OAuth scope: Xbox Live sign-in plus offline access (refresh token).
pub const SCOPE: &str = "XboxLive.signin offline_access";

/// Default Azure client id. This is the well-known public client used by
/// community launchers for the Minecraft device code flow. Override with the
/// `MC_LAUNCHER_CLIENT_ID` environment variable to use your own Azure app
/// registration (recommended for production builds).
pub const DEFAULT_CLIENT_ID: &str = "00000000402B5328";

/// Environment variable that overrides [`DEFAULT_CLIENT_ID`].
pub const CLIENT_ID_ENV: &str = "MC_LAUNCHER_CLIENT_ID";

/// The Azure client id to use: `MC_LAUNCHER_CLIENT_ID` if set, else
/// [`DEFAULT_CLIENT_ID`].
#[must_use]
pub fn client_id() -> String {
    std::env::var(CLIENT_ID_ENV).unwrap_or_else(|_| DEFAULT_CLIENT_ID.to_owned())
}

/// The device code handed to the user.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    #[serde(rename = "verification_uri")]
    pub verification_uri: String,
    /// Optional: a link with the code pre-filled.
    #[serde(rename = "verification_uri_complete", default)]
    pub verification_uri_complete: Option<String>,
    /// Seconds until the code expires.
    #[serde(rename = "expires_in")]
    pub expires_in: u64,
    /// Suggested polling interval in seconds.
    #[serde(default)]
    pub interval: u64,
    /// Microsoft's human-readable instruction message.
    #[serde(default)]
    pub message: Option<String>,
}

/// Request a device code from Microsoft. The user must visit
/// `verification_uri` and enter `user_code`, after which
/// [`wait_for_device_approval`] completes the flow.
///
/// # Errors
///
/// Fails on network errors or an invalid response body.
pub async fn request_device_code(client: &reqwest::Client, client_id: &str) -> Result<DeviceCode> {
    request_device_code_at(client, client_id, DEVICE_CODE_URL).await
}

/// [`request_device_code`] against an explicit endpoint URL (tests).
///
/// # Errors
///
/// Same as [`request_device_code`].
#[doc(hidden)]
pub async fn request_device_code_at(
    client: &reqwest::Client,
    client_id: &str,
    url: &str,
) -> Result<DeviceCode> {
    let body = form_encode(&[("client_id", client_id), ("scope", SCOPE)]);
    let response = client
        .post(url)
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(body)
        .send()
        .await?
        .error_for_status()?;
    let bytes = response.bytes().await?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Result of one poll attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DevicePoll {
    /// The user has not approved yet; poll again later.
    Pending,
    /// Microsoft asked us to stretch the polling interval.
    SlowDown,
    /// The user approved; the Microsoft token pair is ready.
    Authorized {
        access_token: String,
        refresh_token: String,
    },
    /// The user declined the authorization request.
    Declined,
    /// The device code expired before approval.
    Expired,
}

/// Poll the token endpoint once.
///
/// # Errors
///
/// Fails on network errors, HTTP errors, or an unexpected response body.
pub async fn poll_device_code(
    client: &reqwest::Client,
    client_id: &str,
    device_code: &str,
) -> Result<DevicePoll> {
    poll_device_code_at(client, client_id, device_code, TOKEN_URL).await
}

/// [`poll_device_code`] against an explicit endpoint URL (tests).
///
/// # Errors
///
/// Same as [`poll_device_code`].
#[doc(hidden)]
pub async fn poll_device_code_at(
    client: &reqwest::Client,
    client_id: &str,
    device_code: &str,
    url: &str,
) -> Result<DevicePoll> {
    let body = form_encode(&[
        ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ("client_id", client_id),
        ("device_code", device_code),
    ]);
    let response = client
        .post(url)
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(body)
        .send()
        .await?;
    let status = response.status();
    let bytes = response.bytes().await?;
    if status.is_success() {
        let tokens: TokenResponse = serde_json::from_slice(&bytes)?;
        return Ok(DevicePoll::Authorized {
            access_token: tokens.access_token,
            refresh_token: tokens.refresh_token.ok_or_else(|| {
                Error::Auth("token response is missing a refresh token".to_owned())
            })?,
        });
    }
    let error: TokenError = serde_json::from_slice(&bytes).map_err(|_| {
        Error::Auth(format!(
            "token endpoint returned HTTP {status} with an unparseable body"
        ))
    })?;
    match error.error.as_str() {
        "authorization_pending" => Ok(DevicePoll::Pending),
        "slow_down" => Ok(DevicePoll::SlowDown),
        "authorization_declined" => Ok(DevicePoll::Declined),
        "expired_token" => Ok(DevicePoll::Expired),
        other => Err(Error::Auth(format!(
            "token endpoint error '{other}': {}",
            error
                .error_description
                .as_deref()
                .unwrap_or("no description")
        ))),
    }
}

/// Poll until the user approves or the code expires, honoring the server's
/// `interval` and `expires_in` hints. The code's instructions are printed
/// through `on_code` (the CLI uses it to tell the user what to do).
///
/// # Errors
///
/// Fails on network errors or an unexpected server response; returns
/// [`Error::AuthDeclined`] / [`Error::AuthExpired`] for terminal states.
pub async fn wait_for_device_approval<F>(
    client: &reqwest::Client,
    client_id: &str,
    code: &DeviceCode,
    on_code: F,
) -> Result<DevicePoll>
where
    F: FnMut(&DeviceCode),
{
    wait_for_device_approval_at(client, client_id, code, on_code, TOKEN_URL).await
}

/// [`wait_for_device_approval`] against an explicit token endpoint URL
/// (tests).
///
/// # Errors
///
/// Same as [`wait_for_device_approval`].
#[doc(hidden)]
pub async fn wait_for_device_approval_at<F>(
    client: &reqwest::Client,
    client_id: &str,
    code: &DeviceCode,
    mut on_code: F,
    url: &str,
) -> Result<DevicePoll>
where
    F: FnMut(&DeviceCode),
{
    on_code(code);
    let mut interval = code.interval.max(1);
    let mut remaining = code.expires_in.max(1);
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
        match poll_device_code_at(client, client_id, &code.device_code, url).await? {
            DevicePoll::Pending => {}
            DevicePoll::SlowDown => interval += 5,
            done @ DevicePoll::Authorized { .. } => return Ok(done),
            DevicePoll::Declined => return Err(Error::AuthDeclined),
            DevicePoll::Expired => return Err(Error::AuthExpired),
        }
        remaining = remaining.saturating_sub(interval);
        if remaining == 0 {
            return Err(Error::AuthExpired);
        }
    }
}

/// A Microsoft access + refresh token pair.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
}

/// Exchange a Microsoft refresh token for a fresh token pair. Microsoft
/// rotates refresh tokens, so the returned pair replaces the old one.
///
/// # Errors
///
/// Fails on network errors, invalid responses, or a rejected refresh token.
pub async fn refresh_ms_token(
    client: &reqwest::Client,
    client_id: &str,
    refresh_token: &str,
) -> Result<TokenPair> {
    refresh_ms_token_at(client, client_id, refresh_token, TOKEN_URL).await
}

/// [`refresh_ms_token`] against an explicit endpoint URL (tests).
///
/// # Errors
///
/// Same as [`refresh_ms_token`].
#[doc(hidden)]
pub async fn refresh_ms_token_at(
    client: &reqwest::Client,
    client_id: &str,
    refresh_token: &str,
    url: &str,
) -> Result<TokenPair> {
    let body = form_encode(&[
        ("grant_type", "refresh_token"),
        ("client_id", client_id),
        ("scope", SCOPE),
        ("refresh_token", refresh_token),
    ]);
    let response = client
        .post(url)
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(body)
        .send()
        .await?;
    let status = response.status();
    let bytes = response.bytes().await?;
    if status.is_success() {
        let tokens: TokenResponse = serde_json::from_slice(&bytes)?;
        return Ok(TokenPair {
            access_token: tokens.access_token,
            refresh_token: tokens.refresh_token.ok_or_else(|| {
                Error::Auth("refresh response is missing a refresh token".to_owned())
            })?,
        });
    }
    let error: TokenError = serde_json::from_slice(&bytes)
        .map_err(|_| Error::Auth(format!("token endpoint returned HTTP {status} on refresh")))?;
    Err(Error::Auth(format!(
        "refresh failed ('{}'): {}",
        error.error,
        error
            .error_description
            .as_deref()
            .unwrap_or("no description")
    )))
}

/// An Xbox Live token plus the user hash (`uhs`) needed downstream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XblToken {
    /// The full `XBL3.0 x=...` token string.
    pub token: String,
    /// User hash, embedded in the identity token sent to Mojang.
    pub uhs: String,
}

/// Authenticate with Xbox Live using a Microsoft access token.
///
/// # Errors
///
/// Fails on network errors, invalid responses, or a rejected token.
pub async fn xbl_authenticate(client: &reqwest::Client, ms_access_token: &str) -> Result<XblToken> {
    xbl_authenticate_at(client, ms_access_token, XBL_URL).await
}

/// [`xbl_authenticate`] against an explicit endpoint URL (tests).
///
/// # Errors
///
/// Same as [`xbl_authenticate`].
#[doc(hidden)]
pub async fn xbl_authenticate_at(
    client: &reqwest::Client,
    ms_access_token: &str,
    url: &str,
) -> Result<XblToken> {
    let body = serde_json::json!({
        "RelyingParty": "http://auth.xboxlive.com",
        "TokenType": "JWT",
        "Properties": {
            "AuthMethod": "RPS",
            "SiteName": "user.auth.xboxlive.com",
            "RpsTicket": format!("d={ms_access_token}"),
        },
    });
    let xsts = authenticate_at(client, url, &body.to_string()).await?;
    Ok(XblToken {
        uhs: xsts.uhs,
        token: xsts.token,
    })
}

/// The XSTS token chain result: the token and the user hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XstsToken {
    pub token: String,
    pub uhs: String,
}

/// Authorize an Xbox Live token with the XSTS service, scoped to Mojang.
///
/// # Errors
///
/// Fails on network errors, invalid responses, or accounts that cannot use
/// Minecraft (e.g. underage accounts, no Xbox profile).
pub async fn xsts_authorize(client: &reqwest::Client, xbl: &XblToken) -> Result<XstsToken> {
    xsts_authorize_at(client, xbl, XSTS_URL).await
}

/// [`xsts_authorize`] against an explicit endpoint URL (tests).
///
/// # Errors
///
/// Same as [`xsts_authorize`].
#[doc(hidden)]
pub async fn xsts_authorize_at(
    client: &reqwest::Client,
    xbl: &XblToken,
    url: &str,
) -> Result<XstsToken> {
    let body = serde_json::json!({
        "RelyingParty": "rp://api.minecraftservices.com/",
        "TokenType": "JWT",
        "Properties": {
            "SandboxId": "RETAIL",
            "UserTokens": [xbl.token],
        },
    });
    let xsts = authenticate_at(client, url, &body.to_string()).await?;
    Ok(XstsToken {
        token: xsts.token,
        uhs: xsts.uhs,
    })
}

/// Shared plumbing for the XBL and XSTS endpoints, including the Xbox error
/// codes that need human-readable messages.
async fn authenticate_at(client: &reqwest::Client, url: &str, body: &str) -> Result<XstsToken> {
    let response = client
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header("x-xbl-contract-version", "1")
        .body(body.to_owned())
        .send()
        .await?;
    let status = response.status();
    let bytes = response.bytes().await?;
    if !status.is_success() {
        if let Ok(xbox_error) = serde_json::from_slice::<XboxError>(&bytes) {
            return Err(Error::Auth(xbox_error.message()));
        }
        return Err(Error::Auth(format!(
            "Xbox authentication returned HTTP {status}"
        )));
    }
    let parsed: AuthenticateResponse = serde_json::from_slice(&bytes)?;
    let xui = parsed
        .display_claims
        .xui
        .first()
        .ok_or_else(|| Error::Auth("Xbox response is missing the user hash".to_owned()))?;
    Ok(XstsToken {
        token: parsed.token,
        uhs: xui.uhs.clone(),
    })
}

#[derive(Debug, Clone, Deserialize)]
struct AuthenticateResponse {
    #[serde(rename = "Token")]
    token: String,
    #[serde(rename = "DisplayClaims")]
    display_claims: DisplayClaims,
}

#[derive(Debug, Clone, Deserialize)]
struct DisplayClaims {
    xui: Vec<XuiClaim>,
}

#[derive(Debug, Clone, Deserialize)]
struct XuiClaim {
    uhs: String,
}

#[derive(Debug, Clone, Deserialize)]
struct XboxError {
    #[serde(default, rename = "XErr")]
    xerr: Option<i64>,
    #[serde(default, rename = "Message")]
    message: Option<String>,
}

impl XboxError {
    fn message(&self) -> String {
        if let Some(code) = self.xerr {
            let reason = match code {
                2_148_916_233 => {
                    "the account has no Xbox Live profile (created after October 2022 and never played on Xbox)"
                }
                2_148_916_235 => "the account has no Xbox Live profile",
                2_148_916_236 => "the account is underage; its family has no adult",
                2_148_916_237 => "the account is underage; ask a parent to add it to a family",
                2_148_916_238 => "the account is underage and its parent has not set up a family",
                _ => "unexpected Xbox error",
            };
            format!("Xbox error {code}: {reason}")
        } else {
            self.message
                .clone()
                .unwrap_or_else(|| "Xbox authentication failed".to_owned())
        }
    }
}

/// A Minecraft access token (short-lived, ~24 hours).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MinecraftToken {
    /// The token to send as `Authorization: Bearer`.
    pub access_token: String,
    /// The player UUID (without dashes) as reported by Mojang.
    pub username: String,
    /// Seconds until the token expires.
    pub expires_in: u64,
}

/// Trade the XSTS token for a Minecraft access token.
///
/// # Errors
///
/// Fails on network errors, invalid responses, or a rejected token.
pub async fn login_with_xbox(client: &reqwest::Client, xsts: &XstsToken) -> Result<MinecraftToken> {
    login_with_xbox_at(client, xsts, MC_LOGIN_URL).await
}

/// [`login_with_xbox`] against an explicit endpoint URL (tests).
///
/// # Errors
///
/// Same as [`login_with_xbox`].
#[doc(hidden)]
pub async fn login_with_xbox_at(
    client: &reqwest::Client,
    xsts: &XstsToken,
    url: &str,
) -> Result<MinecraftToken> {
    let body = serde_json::json!({
        "identityToken": format!("XBL3.0 x={};{}", xsts.uhs, xsts.token),
    });
    let response = client
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::ACCEPT, "application/json")
        .body(body.to_string())
        .send()
        .await?;
    let status = response.status();
    let bytes = response.bytes().await?;
    if !status.is_success() {
        return Err(Error::Auth(format!(
            "Minecraft login returned HTTP {status}: {}",
            String::from_utf8_lossy(&bytes)
        )));
    }
    let parsed: MinecraftLoginResponse = serde_json::from_slice(&bytes)?;
    Ok(MinecraftToken {
        access_token: parsed.access_token,
        username: parsed.username,
        expires_in: parsed.expires_in,
    })
}

#[derive(Debug, Clone, Deserialize)]
struct MinecraftLoginResponse {
    username: String,
    access_token: String,
    expires_in: u64,
}

/// A Minecraft player profile.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Profile {
    /// UUID with dashes.
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub skins: Vec<Skin>,
    #[serde(default)]
    pub capes: Vec<Cape>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Skin {
    pub id: String,
    pub state: String,
    pub url: String,
    #[serde(default)]
    pub variant: Option<String>,
    #[serde(default)]
    pub alias: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Cape {
    pub id: String,
    pub state: String,
    pub url: String,
    #[serde(default)]
    pub alias: Option<String>,
}

/// Fetch the authenticated player's profile (UUID, name, skins, capes).
///
/// # Errors
///
/// Fails on network errors, non-2xx responses (e.g. no profile), or an
/// unparseable body.
pub async fn fetch_profile(client: &reqwest::Client, mc_access_token: &str) -> Result<Profile> {
    fetch_profile_at(client, mc_access_token, PROFILE_URL).await
}

/// [`fetch_profile`] against an explicit endpoint URL (tests).
///
/// # Errors
///
/// Same as [`fetch_profile`].
#[doc(hidden)]
pub async fn fetch_profile_at(
    client: &reqwest::Client,
    mc_access_token: &str,
    url: &str,
) -> Result<Profile> {
    let response = client
        .get(url)
        .header(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {mc_access_token}"),
        )
        .send()
        .await?;
    let status = response.status();
    let bytes = response.bytes().await?;
    if !status.is_success() {
        return Err(Error::Auth(format!(
            "profile request returned HTTP {status}"
        )));
    }
    Ok(serde_json::from_slice(&bytes)?)
}

/// Run the full sign-in chain after the user approves the device code:
/// Microsoft token to XBL to XSTS to Minecraft token to profile.
///
/// # Errors
///
/// Fails on network errors or any rejected token in the chain.
pub async fn complete_sign_in(
    client: &reqwest::Client,
    ms_access_token: &str,
) -> Result<(MinecraftToken, Profile)> {
    complete_sign_in_at(
        client,
        ms_access_token,
        XBL_URL,
        XSTS_URL,
        MC_LOGIN_URL,
        PROFILE_URL,
    )
    .await
}

/// [`complete_sign_in`] against explicit endpoint URLs (tests).
///
/// # Errors
///
/// Same as [`complete_sign_in`].
#[doc(hidden)]
pub async fn complete_sign_in_at(
    client: &reqwest::Client,
    ms_access_token: &str,
    xbl_url: &str,
    xsts_url: &str,
    mc_login_url: &str,
    profile_url: &str,
) -> Result<(MinecraftToken, Profile)> {
    let xbl = xbl_authenticate_at(client, ms_access_token, xbl_url).await?;
    let xsts = xsts_authorize_at(client, &xbl, xsts_url).await?;
    let mc = login_with_xbox_at(client, &xsts, mc_login_url).await?;
    let profile = fetch_profile_at(client, &mc.access_token, profile_url).await?;
    Ok((mc, profile))
}

/// Refresh an expired Minecraft token: trade the Microsoft refresh token for
/// a fresh access token, then re-run the XBL to XSTS to Minecraft chain.
///
/// # Errors
///
/// Fails on network errors or a rejected refresh token.
pub async fn refresh_minecraft_token(
    client: &reqwest::Client,
    client_id: &str,
    refresh_token: &str,
) -> Result<(TokenPair, MinecraftToken)> {
    refresh_minecraft_token_at(
        client,
        client_id,
        refresh_token,
        TOKEN_URL,
        XBL_URL,
        XSTS_URL,
        MC_LOGIN_URL,
    )
    .await
}

/// [`refresh_minecraft_token`] against explicit endpoint URLs (tests).
///
/// # Errors
///
/// Same as [`refresh_minecraft_token`].
#[doc(hidden)]
pub async fn refresh_minecraft_token_at(
    client: &reqwest::Client,
    client_id: &str,
    refresh_token: &str,
    token_url: &str,
    xbl_url: &str,
    xsts_url: &str,
    mc_login_url: &str,
) -> Result<(TokenPair, MinecraftToken)> {
    let pair = refresh_ms_token_at(client, client_id, refresh_token, token_url).await?;
    let xbl = xbl_authenticate_at(client, &pair.access_token, xbl_url).await?;
    let xsts = xsts_authorize_at(client, &xbl, xsts_url).await?;
    let mc = login_with_xbox_at(client, &xsts, mc_login_url).await?;
    Ok((pair, mc))
}

/// The token response shape from the Microsoft token endpoint.
#[derive(Debug, Clone, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
}

/// The error shape from the Microsoft token endpoint.
#[derive(Debug, Clone, Deserialize)]
struct TokenError {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

/// Percent-encode a value for `application/x-www-form-urlencoded` bodies.
fn urlencode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(byte));
            }
            b' ' => out.push('+'),
            _ => {
                out.push('%');
                out.push(char::from_digit(u32::from(byte) >> 4, 16).unwrap_or('0'));
                out.push(char::from_digit(u32::from(byte) & 0x0F, 16).unwrap_or('0'));
            }
        }
    }
    out
}

fn form_encode<'a>(fields: &[(&'a str, &'a str)]) -> String {
    fields
        .iter()
        .map(|(key, value)| format!("{}={}", urlencode(key), urlencode(value)))
        .collect::<Vec<_>>()
        .join("&")
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    use super::*;

    /// A response script: status line, content type, body.
    type Script = (&'static str, &'static str, &'static str);
    /// Per-path script queues: `(route, scripts)` — scripts are consumed in
    /// order and the last one repeats.
    type Routes = Vec<(&'static str, Vec<Script>)>;

    /// A scripted mock server. Routes by request path; per-path scripts are
    /// consumed in order (the last one repeats), and requests are recorded
    /// for assertions.
    struct MockAuth {
        addr: String,
        requests: Arc<Mutex<Vec<String>>>,
    }

    impl MockAuth {
        fn new(routes: Routes) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            let addr = listener.local_addr().expect("local addr");
            let requests: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
            let recorded = Arc::clone(&requests);
            let counters: Arc<Mutex<std::collections::HashMap<&'static str, usize>>> =
                Arc::new(Mutex::new(std::collections::HashMap::new()));
            let counters_for_server = Arc::clone(&counters);
            thread::spawn(move || {
                for stream in listener.incoming() {
                    let routes = routes.clone();
                    let recorded = Arc::clone(&recorded);
                    let counters = Arc::clone(&counters_for_server);
                    thread::spawn(move || {
                        let mut stream = stream.expect("accept");
                        let mut buf = [0u8; 16384];
                        let _ = stream.read(&mut buf);
                        let head = String::from_utf8_lossy(&buf);
                        if let Ok(mut recorded) = recorded.lock() {
                            recorded.push(head.to_string());
                        }
                        let path = head
                            .lines()
                            .next()
                            .and_then(|l| l.split_whitespace().nth(1))
                            .unwrap_or("/")
                            .to_owned();
                        let (route, scripts) = routes
                            .iter()
                            .find(|(route, _)| format!("/{route}") == path)
                            .or_else(|| routes.iter().find(|(route, _)| path.contains(route)))
                            .map_or(("__missing__", Vec::new()), |(route, scripts)| {
                                (*route, scripts.clone())
                            });
                        let index = {
                            let mut counters = counters.lock().expect("lock");
                            let entry = counters.entry(route).or_insert(0);
                            let index = *entry;
                            *entry += 1;
                            index
                        };
                        let script = scripts
                            .get(index.min(scripts.len().saturating_sub(1)))
                            .copied()
                            .unwrap_or(("404 Not Found", "text/plain", ""));
                        let response = format!(
                            "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            script.0,
                            script.1,
                            script.2.len(),
                            script.2
                        );
                        let _ = stream.write_all(response.as_bytes());
                        let _ = stream.flush();
                    });
                }
            });
            Self {
                addr: format!("http://{addr}"),
                requests,
            }
        }

        fn url(&self, path: &str) -> String {
            format!("{}{}", self.addr, path)
        }

        fn request_bodies(&self) -> Vec<String> {
            self.requests
                .lock()
                .expect("lock")
                .iter()
                .map(|head| head.split("\r\n\r\n").nth(1).unwrap_or("").to_owned())
                .collect()
        }
    }

    fn respond(status: &'static str, body: &'static str) -> Script {
        (status, "application/json", body)
    }

    fn route(path: &'static str, scripts: Vec<Script>) -> (&'static str, Vec<Script>) {
        (path, scripts)
    }

    const DEVICE_CODE_JSON: &str = r#"{
        "device_code": "dc-123",
        "user_code": "ABCDEF",
        "verification_uri": "https://microsoft.com/link",
        "verification_uri_complete": "https://microsoft.com/link?otc=ABCDEF",
        "expires_in": 900,
        "interval": 5,
        "message": "Enter the code ABCDEF"
    }"#;

    const TOKEN_OK_JSON: &str = r#"{
        "token_type": "Bearer",
        "scope": "XboxLive.signin offline_access",
        "expires_in": 3600,
        "access_token": "ms-access",
        "refresh_token": "ms-refresh"
    }"#;

    const XBL_JSON: &str = r#"{
        "IssueInstant": "2026-01-01T00:00:00Z",
        "Token": "XBL_TOKEN",
        "DisplayClaims": { "xui": [{ "uhs": "user-hash", "gtg": "Steve", "xid": "123" }] }
    }"#;

    const XSTS_JSON: &str = r#"{
        "IssueInstant": "2026-01-01T00:00:00Z",
        "Token": "XSTS_TOKEN",
        "DisplayClaims": { "xui": [{ "uhs": "user-hash" }] }
    }"#;

    const MC_LOGIN_JSON: &str = r#"{
        "username": "853c80ef3c3749fdaa49938b674adae6",
        "roles": [],
        "access_token": "mc-access-token",
        "token_type": "Bearer",
        "expires_in": 86400
    }"#;

    const PROFILE_JSON: &str = r#"{
        "id": "853c80ef-3c37-49fd-aa49-938b674adae6",
        "name": "Steve",
        "skins": [{ "id": "s1", "state": "ACTIVE", "url": "https://textures/skin.png", "variant": "CLASSIC" }],
        "capes": []
    }"#;

    #[test]
    fn urlencode_escapes_specials_and_spaces() {
        assert_eq!(
            urlencode("XboxLive.signin offline_access"),
            "XboxLive.signin+offline_access"
        );
        assert_eq!(urlencode("a/b+c=~"), "a%2fb%2bc%3d~");
        assert_eq!(urlencode("plain"), "plain");
    }

    #[tokio::test]
    async fn request_device_code_parses_and_sends_scope() {
        let mock = MockAuth::new(vec![route(
            "devicecode",
            vec![respond("200 OK", DEVICE_CODE_JSON)],
        )]);
        let client = reqwest::Client::new();
        let code = request_device_code_at(&client, "my-client", &mock.url("/devicecode"))
            .await
            .expect("device code");
        assert_eq!(code.device_code, "dc-123");
        assert_eq!(code.user_code, "ABCDEF");
        assert_eq!(code.verification_uri, "https://microsoft.com/link");
        assert_eq!(code.expires_in, 900);
        assert_eq!(code.interval, 5);
        assert_eq!(
            code.verification_uri_complete.as_deref(),
            Some("https://microsoft.com/link?otc=ABCDEF")
        );
        let body = mock.request_bodies().pop().expect("one request");
        assert!(body.contains("client_id=my-client"));
        assert!(body.contains("scope=XboxLive.signin+offline_access"));
    }

    #[tokio::test]
    async fn poll_returns_authorized_tokens() {
        let mock = MockAuth::new(vec![route("token", vec![respond("200 OK", TOKEN_OK_JSON)])]);
        let client = reqwest::Client::new();
        let result = poll_device_code_at(&client, "cid", "dc-123", &mock.url("/token"))
            .await
            .expect("poll");
        assert_eq!(
            result,
            DevicePoll::Authorized {
                access_token: "ms-access".to_owned(),
                refresh_token: "ms-refresh".to_owned(),
            }
        );
        let body = mock.request_bodies().pop().expect("one request");
        assert!(body.contains("grant_type=urn%3aietf%3aparams%3aoauth%3agrant-type%3adevice_code"));
        assert!(body.contains("device_code=dc-123"));
    }

    #[tokio::test]
    async fn poll_maps_terminal_and_pending_errors() {
        let cases: &[(&str, DevicePoll)] = &[
            (
                r#"{"error":"authorization_pending","error_description":"pending"}"#,
                DevicePoll::Pending,
            ),
            (
                r#"{"error":"slow_down","error_description":"slow"}"#,
                DevicePoll::SlowDown,
            ),
            (
                r#"{"error":"authorization_declined","error_description":"no"}"#,
                DevicePoll::Declined,
            ),
            (
                r#"{"error":"expired_token","error_description":"gone"}"#,
                DevicePoll::Expired,
            ),
        ];
        for (body, expected) in cases {
            let mock = MockAuth::new(vec![route("token", vec![respond("400 Bad Request", body)])]);
            let client = reqwest::Client::new();
            let result = poll_device_code_at(&client, "cid", "dc", &mock.url("/token"))
                .await
                .expect("poll");
            assert_eq!(&result, expected, "body: {body}");
        }
    }

    #[tokio::test]
    async fn poll_surfaces_unknown_errors() {
        let mock = MockAuth::new(vec![route(
            "token",
            vec![respond(
                "400 Bad Request",
                r#"{"error":"invalid_grant","error_description":"bad"}"#,
            )],
        )]);
        let client = reqwest::Client::new();
        let err = poll_device_code_at(&client, "cid", "dc", &mock.url("/token"))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Auth(_)));
        assert!(err.to_string().contains("invalid_grant"));
    }

    #[tokio::test]
    async fn wait_loop_returns_authorized_after_pending() {
        let mock = MockAuth::new(vec![route(
            "token",
            vec![
                respond("400 Bad Request", r#"{"error":"authorization_pending"}"#),
                respond("200 OK", TOKEN_OK_JSON),
            ],
        )]);
        let client = reqwest::Client::new();
        let code = DeviceCode {
            device_code: "dc".to_owned(),
            user_code: "ABCDEF".to_owned(),
            verification_uri: "https://microsoft.com/link".to_owned(),
            verification_uri_complete: None,
            expires_in: 30,
            interval: 1,
            message: None,
        };
        let mut shown = Vec::new();
        let result = wait_for_device_approval_at(
            &client,
            "cid",
            &code,
            |c| {
                shown.push(c.user_code.clone());
            },
            &mock.url("/token"),
        )
        .await
        .expect("approval");
        assert!(matches!(result, DevicePoll::Authorized { .. }));
        assert_eq!(shown, vec!["ABCDEF".to_owned()]);
    }

    #[tokio::test]
    async fn wait_loop_returns_declined() {
        let mock = MockAuth::new(vec![route(
            "token",
            vec![respond(
                "400 Bad Request",
                r#"{"error":"authorization_declined"}"#,
            )],
        )]);
        let client = reqwest::Client::new();
        let code = DeviceCode {
            device_code: "dc".to_owned(),
            user_code: "ABCDEF".to_owned(),
            verification_uri: "https://microsoft.com/link".to_owned(),
            verification_uri_complete: None,
            expires_in: 30,
            interval: 1,
            message: None,
        };
        let err = wait_for_device_approval_at(&client, "cid", &code, |_| {}, &mock.url("/token"))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::AuthDeclined));
    }

    #[tokio::test]
    async fn complete_sign_in_runs_the_whole_chain() {
        let mock = MockAuth::new(vec![
            route("xbl", vec![respond("200 OK", XBL_JSON)]),
            route("xsts", vec![respond("200 OK", XSTS_JSON)]),
            route("mclogin", vec![respond("200 OK", MC_LOGIN_JSON)]),
            route("profile", vec![respond("200 OK", PROFILE_JSON)]),
        ]);
        let client = reqwest::Client::new();
        let (mc, profile) = complete_sign_in_at(
            &client,
            "ms-access",
            &mock.url("/xbl"),
            &mock.url("/xsts"),
            &mock.url("/mclogin"),
            &mock.url("/profile"),
        )
        .await
        .expect("sign in");
        assert_eq!(mc.access_token, "mc-access-token");
        assert_eq!(mc.expires_in, 86_400);
        assert_eq!(profile.id, "853c80ef-3c37-49fd-aa49-938b674adae6");
        assert_eq!(profile.name, "Steve");
        assert_eq!(profile.skins.len(), 1);
        let bodies = mock.request_bodies();
        assert_eq!(bodies.len(), 4);
        // The Minecraft login body carries the XSTS identity token.
        assert!(bodies[2].contains(r#""identityToken":"XBL3.0 x=user-hash;XSTS_TOKEN""#));
    }

    #[tokio::test]
    async fn refresh_chain_rotates_ms_and_minecraft_tokens() {
        let mock = MockAuth::new(vec![
            route("token", vec![respond("200 OK", TOKEN_OK_JSON)]),
            route("xbl", vec![respond("200 OK", XBL_JSON)]),
            route("xsts", vec![respond("200 OK", XSTS_JSON)]),
            route("mclogin", vec![respond("200 OK", MC_LOGIN_JSON)]),
        ]);
        let client = reqwest::Client::new();
        let (pair, mc) = refresh_minecraft_token_at(
            &client,
            "cid",
            "old-refresh",
            &mock.url("/token"),
            &mock.url("/xbl"),
            &mock.url("/xsts"),
            &mock.url("/mclogin"),
        )
        .await
        .expect("refresh");
        assert_eq!(pair.access_token, "ms-access");
        assert_eq!(pair.refresh_token, "ms-refresh");
        assert_eq!(mc.access_token, "mc-access-token");
        let bodies = mock.request_bodies();
        assert!(bodies[0].contains("grant_type=refresh_token"));
        assert!(bodies[0].contains("refresh_token=old-refresh"));
    }

    #[tokio::test]
    async fn xbox_errors_get_human_readable_messages() {
        let mock = MockAuth::new(vec![route(
            "xbl",
            vec![respond(
                "400 Bad Request",
                r#"{"XErr":2148916233,"Message":"Bad","Redirect":""}"#,
            )],
        )]);
        let client = reqwest::Client::new();
        let err = xbl_authenticate_at(&client, "ms-access", &mock.url("/xbl"))
            .await
            .unwrap_err();
        let message = err.to_string();
        assert!(message.contains("2148916233"), "{message}");
        assert!(message.contains("no Xbox Live profile"), "{message}");
    }

    #[tokio::test]
    async fn profile_requests_send_bearer_token() {
        let mock = MockAuth::new(vec![route(
            "profile",
            vec![respond("200 OK", PROFILE_JSON)],
        )]);
        let client = reqwest::Client::new();
        let profile = fetch_profile_at(&client, "mc-access-token", &mock.url("/profile"))
            .await
            .expect("profile");
        assert_eq!(profile.name, "Steve");
        let heads = mock.requests.lock().expect("lock");
        let lower = heads[0].to_lowercase();
        assert!(
            lower.contains("authorization: bearer mc-access-token"),
            "expected bearer header in {heads:?}"
        );
    }

    #[tokio::test]
    async fn profile_request_fails_on_non_2xx() {
        let mock = MockAuth::new(vec![route("profile", vec![respond("404 Not Found", "{}")])]);
        let client = reqwest::Client::new();
        let err = fetch_profile_at(&client, "token", &mock.url("/profile"))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Auth(_)));
        assert!(err.to_string().contains("404"));
    }
}
