//! OAuth tokens in the OS credential store (Keychain / Secret Service /
//! Credential Manager). Never written to disk by us.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const SERVICE: &str = "ytm-player";
const ACCOUNT: &str = "google-oauth";

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

fn entry() -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, ACCOUNT).context("OS credential store unavailable")
}

/// Keyring calls block (D-Bus on Linux), so run them off the async runtime.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f).await?
}

pub async fn load() -> Result<Option<StoredToken>> {
    blocking(|| match entry()?.get_password() {
        Ok(json) => Ok(Some(serde_json::from_str(&json)?)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(err) => Err(err).context("reading token from credential store"),
    })
    .await
}

pub async fn save(token: &StoredToken) -> Result<()> {
    let json = serde_json::to_string(token)?;
    blocking(move || {
        entry()?
            .set_password(&json)
            .context("saving token to credential store")
    })
    .await
}

pub async fn clear() -> Result<()> {
    blocking(|| match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(err).context("deleting token from credential store"),
    })
    .await
}
