//! Google sign-in.
//!
//! - [`login`]: Authorization Code + PKCE for installed apps — open the
//!   consent page in the browser, receive the code on a loopback listener
//!   (`http://127.0.0.1:<random port>`), exchange it for tokens.
//! - [`login_device`]: device flow for machines without a local browser
//!   (SSH): show a code, approve it on any device. Needs a separate
//!   "TVs and Limited Input devices" OAuth client.

use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use oauth2::{
    AuthType, AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken,
    DeviceAuthorizationUrl, EmptyExtraDeviceAuthorizationFields, PkceCodeChallenge, RedirectUrl,
    RefreshToken, RequestTokenError, Scope, StandardDeviceAuthorizationResponse, TokenResponse,
    TokenUrl,
    basic::{BasicClient, BasicErrorResponseType},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
    sync::Mutex,
};

use super::token_store::{StoredToken, Tokens};

const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const DEVICE_URL: &str = "https://oauth2.googleapis.com/device/code";
/// Read + like/add-to-playlist. Tokens from older read-only logins keep
/// working for reading; writes then ask the user to log in again.
const SCOPE: &str = "https://www.googleapis.com/auth/youtube";
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);
/// How to fix an expired or missing sign-in, in error messages.
pub const SIGN_IN_AGAIN: &str = "sign in again (“Sign in” in the window, or `ytm login`)";

#[derive(Debug, Clone)]
pub struct OAuthClient {
    pub client_id: String,
    /// Google issues a secret even for Desktop clients; it isn't confidential.
    pub client_secret: Option<String>,
}

/// Token endpoint client: redirects disabled (oauth2 crate requirement, SSRF guard).
fn token_http() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?)
}

macro_rules! basic_client {
    ($cfg:expr) => {{
        let mut client = BasicClient::new(ClientId::new($cfg.client_id.clone()))
            .set_auth_uri(AuthUrl::new(AUTH_URL.into())?)
            .set_token_uri(TokenUrl::new(TOKEN_URL.into())?);
        if let Some(secret) = &$cfg.client_secret {
            client = client.set_client_secret(ClientSecret::new(secret.clone()));
        }
        client
    }};
}

/// Interactive login: opens the consent page in the browser (`show_url`
/// gets it too, for printing in case no browser opens) and waits for the
/// redirect to a loopback port.
pub async fn login(cfg: &OAuthClient, show_url: impl FnOnce(&str)) -> Result<StoredToken> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let redirect = format!("http://127.0.0.1:{}", listener.local_addr()?.port());
    let client = basic_client!(cfg).set_redirect_uri(RedirectUrl::new(redirect)?);

    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let (url, csrf) = client
        .authorize_url(CsrfToken::new_random)
        .add_scope(Scope::new(SCOPE.into()))
        .set_pkce_challenge(challenge)
        // Ask for a refresh token every time, so re-login always yields one.
        .add_extra_param("access_type", "offline")
        .add_extra_param("prompt", "consent")
        .url();

    show_url(url.as_str());
    let _ = open::that_detached(url.as_str());

    let code = tokio::time::timeout(LOGIN_TIMEOUT, receive_code(&listener, csrf.secret()))
        .await
        .map_err(|_| anyhow!("login timed out after {LOGIN_TIMEOUT:?}"))??;

    let response = client
        .exchange_code(AuthorizationCode::new(code))
        .set_pkce_verifier(verifier)
        .request_async(&token_http()?)
        .await
        .context("exchanging authorization code")?;

    Ok(stored(&response, None, false))
}

/// Device flow: `show_code(url, code)` tells the user where to approve;
/// then polls until approved on any device.
pub async fn login_device(
    cfg: &OAuthClient,
    show_code: impl FnOnce(&str, &str),
) -> Result<StoredToken> {
    let client = basic_client!(cfg)
        .set_device_authorization_url(DeviceAuthorizationUrl::new(DEVICE_URL.into())?)
        // Google wants client credentials in the request body.
        .set_auth_type(AuthType::RequestBody);
    let http = token_http()?;
    let details: StandardDeviceAuthorizationResponse = client
        .exchange_device_code()
        .add_scope(Scope::new(SCOPE.into()))
        .request_async(&http)
        .await
        .context(
            "requesting a device code (is device_client_id a \"TVs and Limited Input\" client?)",
        )?;
    let _: &EmptyExtraDeviceAuthorizationFields = details.extra_fields();

    show_code(
        details.verification_uri().as_str(),
        details.user_code().secret(),
    );
    let response = client
        .exchange_device_access_token(&details)
        .request_async(&http, tokio::time::sleep, Some(LOGIN_TIMEOUT))
        .await
        .context("waiting for device approval")?;
    Ok(stored(&response, None, true))
}

fn stored(
    response: &impl TokenResponse,
    previous_refresh: Option<String>,
    device: bool,
) -> StoredToken {
    StoredToken {
        access_token: response.access_token().secret().clone(),
        // Google omits the refresh token on refresh: keep the old one.
        refresh_token: response
            .refresh_token()
            .map(|t| t.secret().clone())
            .or(previous_refresh),
        expires_at: expires_at(response.expires_in()),
        device,
    }
}

/// Waits for the browser redirect and returns the authorization code.
async fn receive_code(listener: &TcpListener, expected_state: &str) -> Result<String> {
    loop {
        let (mut socket, _) = listener.accept().await?;
        let mut line = String::new();
        BufReader::new(&mut socket).read_line(&mut line).await?;
        let Some(target) = line.split_whitespace().nth(1) else {
            continue;
        };
        let url = url::Url::parse(&format!("http://127.0.0.1{target}"))?;
        let param = |name: &str| {
            url.query_pairs()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.into_owned())
        };

        // Browsers also ask for /favicon.ico etc.
        if param("code").is_none() && param("error").is_none() {
            let _ = socket
                .write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n")
                .await;
            continue;
        }

        let (body, result) = match (param("error"), param("code"), param("state")) {
            (Some(err), _, _) => (
                "Login failed. You can close this tab.",
                Err(anyhow!("Google returned: {err}")),
            ),
            (None, Some(code), Some(state)) if state == expected_state => (
                "Logged in. You can close this tab and return to the terminal.",
                Ok(code),
            ),
            _ => (
                "Login failed (state mismatch).",
                Err(anyhow!("OAuth state mismatch")),
            ),
        };
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/plain; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = socket.write_all(response.as_bytes()).await;
        return result;
    }
}

fn expires_at(expires_in: Option<Duration>) -> i64 {
    let secs = expires_in.map_or(3600, |d| d.as_secs() as i64);
    chrono::Utc::now().timestamp() + secs
}

/// Hands out valid access tokens, refreshing and persisting as needed.
pub struct Auth {
    cfg: OAuthClient,
    /// Refreshes tokens obtained via [`login_device`].
    device_cfg: Option<OAuthClient>,
    tokens: Tokens,
    token: Mutex<Option<StoredToken>>,
}

impl Auth {
    pub fn new(cfg: OAuthClient, device_cfg: Option<OAuthClient>, tokens: Tokens) -> Self {
        Self {
            cfg,
            device_cfg,
            tokens,
            token: Mutex::new(None),
        }
    }

    pub async fn access_token(&self) -> Result<String> {
        let mut guard = self.token.lock().await;
        if guard.is_none() {
            *guard = self.tokens.load().await?;
        }
        let Some(token) = guard.as_mut() else {
            bail!("not signed in — {SIGN_IN_AGAIN}");
        };
        if token.is_expired(60) {
            *token = self.refresh(token).await?;
            self.tokens.save(token).await?;
        }
        Ok(token.access_token.clone())
    }

    async fn refresh(&self, old: &StoredToken) -> Result<StoredToken> {
        let refresh = old
            .refresh_token
            .clone()
            .with_context(|| format!("session expired — {SIGN_IN_AGAIN}"))?;
        let cfg = match (old.device, &self.device_cfg) {
            (true, Some(device)) => device,
            (true, None) => bail!("signed in with --device but device_client_id is not configured"),
            (false, _) => &self.cfg,
        };
        let result = basic_client!(cfg)
            .set_auth_type(AuthType::RequestBody)
            .exchange_refresh_token(&RefreshToken::new(refresh.clone()))
            .request_async(&token_http()?)
            .await;
        match result {
            Ok(response) => Ok(stored(&response, Some(refresh), old.device)),
            // Revoked, or the 7-day limit Google applies to apps in "Testing".
            Err(RequestTokenError::ServerResponse(r))
                if *r.error() == BasicErrorResponseType::InvalidGrant =>
            {
                bail!(
                    "sign-in expired — {SIGN_IN_AGAIN} (apps in Google's \"Testing\" mode expire logins after 7 days)"
                )
            }
            Err(err) => Err(anyhow!(err)).context("refreshing access token"),
        }
    }
}
