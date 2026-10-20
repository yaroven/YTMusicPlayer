//! Where the OAuth token is kept: the OS credential store (Keychain / Secret
//! Service / Credential Manager) in the app, memory in tests. Never written
//! to disk by us.

use std::sync::Arc;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub(crate) const SERVICE: &str = "yarek-music";
/// The keyring's name for the app up to 1.0.0 ("ytm-player").
const OLD_SERVICE: &str = "ytm-player";
const ACCOUNT: &str = "google-oauth";

/// A secret still under the app's old name moves to the new one (read once,
/// saved under the new name, the old entry deleted). `None` when there is
/// none.
pub(crate) fn take_old_secret(account: &str) -> Option<String> {
    let old = keyring::Entry::new(OLD_SERVICE, account).ok()?;
    let value = old.get_password().ok()?;
    let saved = keyring::Entry::new(SERVICE, account)
        .and_then(|e| e.set_password(&value))
        .is_ok();
    if saved {
        let _ = old.delete_credential();
    }
    Some(value)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredToken {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Unix seconds.
    pub expires_at: i64,
    /// Obtained via device flow (refresh with the device client).
    #[serde(default)]
    pub device: bool,
}

impl StoredToken {
    pub fn is_expired(&self, margin_secs: i64) -> bool {
        chrono::Utc::now().timestamp() + margin_secs >= self.expires_at
    }
}

/// Where the sign-in lives. Blocking calls (D-Bus on Linux); [`Tokens`]
/// runs them off the async runtime.
pub trait TokenStore: Send + Sync + 'static {
    fn load(&self) -> Result<Option<StoredToken>>;
    fn save(&self, token: &StoredToken) -> Result<()>;
    fn clear(&self) -> Result<()>;
}

/// The OS credential store (Keychain / Secret Service / Credential Manager).
pub struct Keyring;

impl Keyring {
    fn entry() -> Result<keyring::Entry> {
        keyring::Entry::new(SERVICE, ACCOUNT).context("OS credential store unavailable")
    }
}

impl TokenStore for Keyring {
    fn load(&self) -> Result<Option<StoredToken>> {
        match Self::entry()?.get_password() {
            Ok(json) => Ok(Some(serde_json::from_str(&json)?)),
            Err(keyring::Error::NoEntry) => match take_old_secret(ACCOUNT) {
                Some(json) => Ok(Some(serde_json::from_str(&json)?)),
                None => Ok(None),
            },
            Err(err) => Err(err).context("reading token from credential store"),
        }
    }

    fn save(&self, token: &StoredToken) -> Result<()> {
        Self::entry()?
            .set_password(&serde_json::to_string(token)?)
            .context("saving token to credential store")
    }

    fn clear(&self) -> Result<()> {
        match Self::entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(err).context("deleting token from credential store"),
        }
    }
}

/// Keeps the token in memory only (tests).
#[derive(Default)]
pub struct InMemory(std::sync::Mutex<Option<StoredToken>>);

impl TokenStore for InMemory {
    fn load(&self) -> Result<Option<StoredToken>> {
        Ok(self.0.lock().unwrap_or_else(|e| e.into_inner()).clone())
    }

    fn save(&self, token: &StoredToken) -> Result<()> {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(token.clone());
        Ok(())
    }

    fn clear(&self) -> Result<()> {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = None;
        Ok(())
    }
}

/// Shared handle to a [`TokenStore`] with async access.
#[derive(Clone)]
pub struct Tokens(Arc<dyn TokenStore>);

impl Tokens {
    pub fn new(store: impl TokenStore) -> Self {
        Self(Arc::new(store))
    }

    /// The OS credential store.
    pub fn keyring() -> Self {
        Self::new(Keyring)
    }

    pub async fn load(&self) -> Result<Option<StoredToken>> {
        let store = self.0.clone();
        tokio::task::spawn_blocking(move || store.load()).await?
    }

    pub async fn save(&self, token: &StoredToken) -> Result<()> {
        let (store, token) = (self.0.clone(), token.clone());
        tokio::task::spawn_blocking(move || store.save(&token)).await?
    }

    pub async fn clear(&self) -> Result<()> {
        let store = self.0.clone();
        tokio::task::spawn_blocking(move || store.clear()).await?
    }
}
