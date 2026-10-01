//! OAuth 2.0 Authorization Code + PKCE for installed apps: open the consent
//! page in the browser, receive the code on a loopback listener
//! (`http://127.0.0.1:<random port>`), exchange it for tokens.

use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use oauth2::{
    AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, PkceCodeChallenge, RedirectUrl,
    RefreshToken, Scope, TokenResponse, TokenUrl, basic::BasicClient,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
    sync::Mutex,
};

use super::token_store::{self, StoredToken};

const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const SCOPE: &str = "https://www.googleapis.com/auth/youtube.readonly";
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

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

/// Interactive login. Prints the URL and tries to open the browser.
pub async fn login(cfg: &OAuthClient) -> Result<StoredToken> {
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

    println!("Open this URL to sign in (trying to open your browser):\n\n{url}\n");
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

    Ok(StoredToken {
        access_token: response.access_token().secret().clone(),
        refresh_token: response.refresh_token().map(|t| t.secret().clone()),
        expires_at: expires_at(response.expires_in()),
    })
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
    token: Mutex<Option<StoredToken>>,
}

impl Auth {
    pub fn new(cfg: OAuthClient) -> Self {
        Self {
            cfg,
            token: Mutex::new(None),
        }
    }

    pub async fn is_logged_in(&self) -> bool {
        matches!(token_store::load().await, Ok(Some(_)))
    }

    pub async fn access_token(&self) -> Result<String> {
        let mut guard = self.token.lock().await;
        if guard.is_none() {
            *guard = token_store::load().await?;
        }
        let Some(token) = guard.as_mut() else {
            bail!("not logged in — run `ytm login`");
        };
        if token.is_expired(60) {
            *token = self.refresh(token).await?;
            token_store::save(token).await?;
        }
        Ok(token.access_token.clone())
    }

    async fn refresh(&self, old: &StoredToken) -> Result<StoredToken> {
        let refresh = old
            .refresh_token
            .clone()
            .context("session expired — run `ytm login`")?;
        let response = basic_client!(self.cfg)
            .exchange_refresh_token(&RefreshToken::new(refresh.clone()))
            .request_async(&token_http()?)
            .await
            .context("refreshing access token (if this persists, run `ytm login`)")?;
        Ok(StoredToken {
            access_token: response.access_token().secret().clone(),
            // Google omits the refresh token on refresh: keep the old one.
            refresh_token: Some(
                response
                    .refresh_token()
                    .map_or(refresh, |t| t.secret().clone()),
            ),
            expires_at: expires_at(response.expires_in()),
        })
    }
}
