//! Google account setup shared by the CLI (`ytm import-client`, `ytm login`)
//! and the GUI's account dialog: the OAuth client (ID + secret) in
//! `config.toml`, and the YouTube API client once a token is stored.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result};

use crate::{
    api::{
        auth::{Auth, OAuthClient},
        client::YouTubeClient,
        token_store,
    },
    config::settings::Settings,
};

/// Where Google's console lists OAuth clients ("Create client" is there).
pub const CONSOLE_URL: &str = "https://console.cloud.google.com/auth/clients";

pub fn client_config(id: &str, secret: &str) -> OAuthClient {
    let secret = secret.trim();
    OAuthClient {
        client_id: id.trim().to_owned(),
        client_secret: (!secret.is_empty()).then(|| secret.to_owned()),
    }
}

/// The browser-login client from settings, if configured.
pub fn oauth_client(settings: &Settings) -> Option<OAuthClient> {
    settings
        .has_oauth_client()
        .then(|| client_config(&settings.client_id, &settings.client_secret))
}

/// The "TVs and Limited Input devices" client for `ytm login --device`.
pub fn device_client(settings: &Settings) -> Option<OAuthClient> {
    settings
        .has_device_client()
        .then(|| client_config(&settings.device_client_id, &settings.device_client_secret))
}

/// `Some` when an OAuth client is configured and a token is stored.
pub async fn youtube_client(
    settings: &Settings,
    http: &reqwest::Client,
) -> Result<Option<Arc<YouTubeClient>>> {
    let Some(client) = oauth_client(settings) else {
        return Ok(None);
    };
    if token_store::load().await?.is_none() {
        return Ok(None);
    }
    Ok(Some(api_client(client, device_client(settings), http)))
}

pub fn api_client(
    client: OAuthClient,
    device: Option<OAuthClient>,
    http: &reqwest::Client,
) -> Arc<YouTubeClient> {
    Arc::new(YouTubeClient::new(
        http.clone(),
        Arc::new(Auth::new(client, device)),
    ))
}

/// Client ID and secret from Google's "Download JSON" file
/// (`{"installed": {"client_id": …, "client_secret": …}}`).
pub fn read_client_json(path: &Path) -> Result<(String, String)> {
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

    #[test]
    fn parses_google_json() {
        let json = r#"{"installed":{"client_id":"123-abc.apps.googleusercontent.com",
            "client_secret":"GOCSPX-x","redirect_uris":["http://localhost"]}}"#;
        let (id, secret) = parse_client_json(json).unwrap();
        assert_eq!(id, "123-abc.apps.googleusercontent.com");
        assert_eq!(secret, "GOCSPX-x");
        assert!(parse_client_json(r#"{"foo":1}"#).is_err());
    }
}
