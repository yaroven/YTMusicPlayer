//! The Google account: the OAuth client (ID + secret, saved in
//! `config.toml`), sign-in (browser or device code), the stored token and
//! the YouTube API client built from them. One module for the CLI
//! (`ytm import-client`, `ytm login`, `ytm logout`) and both UIs.
//!
//! The token lives behind the [`Tokens`] seam: the OS keyring in the app,
//! memory in tests.

use std::{
    future::Future,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result};

use crate::{
    api::{
        auth::{self, Auth, OAuthClient},
        client::YouTubeClient,
        token_store::Tokens,
    },
    config::settings::Settings,
};

/// Where Google's console lists OAuth clients ("Create client" is there).
pub const CONSOLE_URL: &str = "https://console.cloud.google.com/auth/clients";

/// Which OAuth client / sign-in flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    /// "Desktop app" client: consent page in the browser, loopback redirect.
    Browser,
    /// "TVs and Limited Input devices" client: approve a code on any device.
    Device,
}

/// What the user must do to finish signing in.
pub enum Prompt<'a> {
    /// The consent page (the browser is opened too).
    OpenUrl(&'a str),
    /// Open `url` on any device and enter `code`.
    EnterCode { url: &'a str, code: &'a str },
}

pub struct Account {
    config_file: PathBuf,
    http: reqwest::Client,
    tokens: Tokens,
    browser: Option<OAuthClient>,
    device: Option<OAuthClient>,
    /// Signed in (a token is stored) with a configured client.
    youtube: Option<Arc<YouTubeClient>>,
    /// Flow of the stored token.
    signed_in_with: Option<Flow>,
}

impl Account {
    /// Reads the clients from `settings` and checks for a stored token.
    /// Never fails on keyring trouble: that just means not signed in.
    pub async fn load(
        settings: &Settings,
        config_file: PathBuf,
        http: reqwest::Client,
        tokens: Tokens,
    ) -> Result<Self> {
        let browser = client(&settings.client_id, &settings.client_secret);
        let device = client(&settings.device_client_id, &settings.device_client_secret);
        // No client, no point asking the keyring (which may prompt or be
        // missing). A broken keyring means offline, not a failed start.
        let token = if browser.is_some() || device.is_some() {
            tokens.load().await.unwrap_or_else(|err| {
                tracing::warn!(%err, "can't read the stored sign-in");
                None
            })
        } else {
            None
        };
        let mut account = Self {
            config_file,
            http,
            tokens,
            browser,
            device,
            youtube: None,
            signed_in_with: None,
        };
        if let Some(token) = token {
            account.signed_in_with = Some(if token.device {
                Flow::Device
            } else {
                Flow::Browser
            });
            account.connect();
        }
        Ok(account)
    }

    /// The API client, when signed in.
    pub fn youtube(&self) -> Option<Arc<YouTubeClient>> {
        self.youtube.clone()
    }

    /// `config.toml`, where preferences are saved too.
    pub fn config_file(&self) -> &Path {
        &self.config_file
    }

    pub fn signed_in(&self) -> bool {
        self.youtube.is_some()
    }

    /// How the current sign-in was made.
    pub fn signed_in_with(&self) -> Option<Flow> {
        self.signed_in_with.filter(|_| self.signed_in())
    }

    /// The configured client ID for `flow`.
    pub fn client_id(&self, flow: Flow) -> Option<&str> {
        self.client(flow).map(|c| c.client_id.as_str())
    }

    /// Saves an OAuth client into `config.toml` (keeping the rest of it).
    pub fn set_client(&mut self, flow: Flow, id: &str, secret: &str) -> Result<()> {
        Settings::store_client(&self.config_file, id, secret, flow == Flow::Device)?;
        let settings = Settings::load(&self.config_file)?;
        self.browser = client(&settings.client_id, &settings.client_secret);
        self.device = client(&settings.device_client_id, &settings.device_client_secret);
        if self.signed_in() {
            self.connect();
        }
        Ok(())
    }

    /// Reads Google's "Download JSON" file and saves the client in it;
    /// returns its ID.
    pub fn import_client(&mut self, flow: Flow, path: &Path) -> Result<String> {
        let (id, secret) = read_client_json(path)?;
        self.set_client(flow, &id, &secret)?;
        Ok(self.client_id(flow).unwrap_or(&id).to_owned())
    }

    /// Starts signing in. `prompt` tells the user what to do (the browser
    /// flow also opens the page). Spawn the returned future; when it
    /// succeeds, call [`connect`](Self::connect).
    pub fn sign_in(
        &self,
        flow: Flow,
        prompt: impl FnOnce(Prompt) + Send + 'static,
    ) -> Result<impl Future<Output = Result<()>> + Send + 'static> {
        let client = self.client(flow).cloned().with_context(|| match flow {
            Flow::Browser => "no Google OAuth client yet: add one under “Sign in” in the window \
                              (`ytm gui`) or run `ytm import-client <downloaded.json>`"
                .to_owned(),
            Flow::Device => format!(
                "signing in with a code needs a \"TVs and Limited Input devices\" OAuth \
                 client: run `ytm import-client <json> --device` or set \
                 device_client_id/device_client_secret in {}",
                self.config_file.display()
            ),
        })?;
        let tokens = self.tokens.clone();
        Ok(async move {
            let token = match flow {
                Flow::Browser => auth::login(&client, |url| prompt(Prompt::OpenUrl(url))).await?,
                Flow::Device => {
                    auth::login_device(&client, |url, code| prompt(Prompt::EnterCode { url, code }))
                        .await?
                }
            };
            if token.refresh_token.is_none() {
                tracing::warn!("Google returned no refresh token; sign-in may expire soon");
            }
            tokens.save(&token).await
        })
    }

    /// Builds the API client from the stored token (after [`sign_in`]).
    ///
    /// [`sign_in`]: Self::sign_in
    pub fn connect(&mut self) {
        // A token with no matching client can't be refreshed; the browser
        // client also covers device tokens' API calls until they expire.
        let Some(client) = self.browser.clone().or_else(|| self.device.clone()) else {
            self.youtube = None;
            return;
        };
        let auth = Auth::new(client, self.device.clone(), self.tokens.clone());
        self.youtube = Some(Arc::new(YouTubeClient::new(
            self.http.clone(),
            Arc::new(auth),
        )));
        if self.signed_in_with.is_none() {
            self.signed_in_with = Some(Flow::Browser);
        }
    }

    /// Forgets the sign-in now; the returned future deletes the stored token.
    pub fn sign_out(&mut self) -> impl Future<Output = Result<()>> + Send + 'static {
        self.youtube = None;
        self.signed_in_with = None;
        let tokens = self.tokens.clone();
        async move { tokens.clear().await }
    }

    fn client(&self, flow: Flow) -> Option<&OAuthClient> {
        match flow {
            Flow::Browser => self.browser.as_ref(),
            Flow::Device => self.device.as_ref(),
        }
    }
}

fn client(id: &str, secret: &str) -> Option<OAuthClient> {
    let (id, secret) = (id.trim(), secret.trim());
    (!id.is_empty()).then(|| OAuthClient {
        client_id: id.to_owned(),
        client_secret: (!secret.is_empty()).then(|| secret.to_owned()),
    })
}

/// Client ID and secret from Google's "Download JSON" file
/// (`{"installed": {"client_id": …, "client_secret": …}}`).
fn read_client_json(path: &Path) -> Result<(String, String)> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    parse_client_json(&text)
}

fn parse_client_json(text: &str) -> Result<(String, String)> {
    let json: serde_json::Value = serde_json::from_str(text).context("not a JSON file")?;
    let client = ["installed", "web"]
        .iter()
        .find_map(|k| json.get(*k))
        .unwrap_or(&json);
    let id = client["client_id"]
        .as_str()
        .context("no client_id in the file — is this Google's OAuth client JSON?")?;
    let secret = client["client_secret"].as_str().unwrap_or("");
    Ok((id.to_owned(), secret.to_owned()))
}

/// The newest `client_secret_*.json` in the user's Downloads folder (where
/// Google's "Download JSON" button saves it).
pub fn downloaded_client_json() -> Option<PathBuf> {
    let downloads = directories::UserDirs::new()?.download_dir()?.to_path_buf();
    std::fs::read_dir(downloads)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            name.starts_with("client_secret_") && name.ends_with(".json")
        })
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, path)| path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::token_store::{InMemory, StoredToken};

    const ID: &str = "123456789-abc.apps.googleusercontent.com";

    fn temp_config(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ytm-account-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("config.toml")
    }

    async fn account(config: &Path, tokens: &Tokens) -> Account {
        let settings = Settings::load(config).unwrap();
        Account::load(
            &settings,
            config.to_owned(),
            reqwest::Client::new(),
            tokens.clone(),
        )
        .await
        .unwrap()
    }

    fn token(device: bool) -> StoredToken {
        StoredToken {
            access_token: "a".into(),
            refresh_token: Some("r".into()),
            expires_at: i64::MAX,
            device,
        }
    }

    #[test]
    fn parses_google_json() {
        let json = r#"{"installed":{"client_id":"123-abc.apps.googleusercontent.com",
            "client_secret":"GOCSPX-x","redirect_uris":["http://localhost"]}}"#;
        let (id, secret) = parse_client_json(json).unwrap();
        assert_eq!(id, "123-abc.apps.googleusercontent.com");
        assert_eq!(secret, "GOCSPX-x");
        assert!(parse_client_json(r#"{"foo":1}"#).is_err());
    }

    #[tokio::test]
    async fn client_setup_survives_a_restart() {
        let config = temp_config("setup");
        let tokens = Tokens::new(InMemory::default());
        let mut a = account(&config, &tokens).await;
        assert_eq!(a.client_id(Flow::Browser), None);
        assert!(
            a.sign_in(Flow::Browser, |_| {}).is_err(),
            "needs a client first"
        );

        let json = config.with_file_name("client_secret_1.json");
        std::fs::write(
            &json,
            format!(r#"{{"installed":{{"client_id":"{ID}","client_secret":"s"}}}}"#),
        )
        .unwrap();
        assert_eq!(a.import_client(Flow::Browser, &json).unwrap(), ID);
        assert!(!a.signed_in());

        let again = account(&config, &tokens).await;
        assert_eq!(again.client_id(Flow::Browser), Some(ID));
        assert_eq!(again.client_id(Flow::Device), None);
    }

    #[tokio::test]
    async fn stored_token_means_signed_in_until_sign_out() {
        let config = temp_config("token");
        let tokens = Tokens::new(InMemory::default());
        let mut a = account(&config, &tokens).await;
        a.set_client(Flow::Browser, ID, "s").unwrap();
        tokens.save(&token(true)).await.unwrap();

        let mut a = account(&config, &tokens).await;
        assert!(a.signed_in());
        assert_eq!(a.signed_in_with(), Some(Flow::Device));
        assert!(a.youtube().is_some());

        a.sign_out().await.unwrap();
        assert!(!a.signed_in());
        assert!(tokens.load().await.unwrap().is_none());
        assert!(!account(&config, &tokens).await.signed_in());
    }

    struct Broken;
    impl crate::api::token_store::TokenStore for Broken {
        fn load(&self) -> Result<Option<StoredToken>> {
            anyhow::bail!("no keychain")
        }
        fn save(&self, _: &StoredToken) -> Result<()> {
            anyhow::bail!("no keychain")
        }
        fn clear(&self) -> Result<()> {
            anyhow::bail!("no keychain")
        }
    }

    #[tokio::test]
    async fn broken_keyring_means_offline_not_failure() {
        let config = temp_config("broken");
        let tokens = Tokens::new(InMemory::default());
        account(&config, &tokens)
            .await
            .set_client(Flow::Browser, ID, "s")
            .unwrap();
        let broken = Tokens::new(Broken);
        assert!(!account(&config, &broken).await.signed_in());
    }

    #[tokio::test]
    async fn token_without_client_is_not_signed_in() {
        let config = temp_config("noclient");
        let tokens = Tokens::new(InMemory::default());
        tokens.save(&token(false)).await.unwrap();
        assert!(!account(&config, &tokens).await.signed_in());
    }
}
