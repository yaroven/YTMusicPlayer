//! `config.toml`: OAuth client, yt-dlp options, playback defaults.
//! A commented template is written on first run.

use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

const TEMPLATE: &str = r#"# ytm-player configuration

# Google OAuth client ("Desktop app" type) from https://console.cloud.google.com/
# with "YouTube Data API v3" enabled. Can also be set via YTM_CLIENT_ID /
# YTM_CLIENT_SECRET. See README for the step-by-step setup.
client_id = ""
client_secret = ""

# Optional: a second client of type "TVs and Limited Input devices", used by
# `ytm login --device` (sign in by code, e.g. over SSH).
device_client_id = ""
device_client_secret = ""

# Media keys and the system "Now Playing" widget (macOS, Linux).
media_controls = true

# Retry with a JavaScript runtime (system deno/node, or a ~2 MB QuickJS-NG
# download) when yt-dlp fails without one. false = never run JS.
js_fallback = true

# Extra yt-dlp flags, e.g. ["--cookies-from-browser", "firefox"].
ytdlp_extra_args = []

# Liked list: keep only videos in the YouTube "Music" category.
liked_music_only = true

# Audio output device; empty = automatic (on Linux: PipeWire, then
# PulseAudio, then ALSA default). `ytm devices` lists the names.
audio_device = ""

# Startup volume, 0.0 - 1.0.
volume = 0.8
"#;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub client_id: String,
    pub client_secret: String,
    pub device_client_id: String,
    pub device_client_secret: String,
    pub media_controls: bool,
    pub audio_device: String,
    pub js_fallback: bool,
    pub ytdlp_extra_args: Vec<String>,
    pub liked_music_only: bool,
    pub volume: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            client_id: String::new(),
            client_secret: String::new(),
            device_client_id: String::new(),
            device_client_secret: String::new(),
            media_controls: true,
            audio_device: String::new(),
            js_fallback: true,
            ytdlp_extra_args: Vec::new(),
            liked_music_only: true,
            volume: 0.8,
        }
    }
}

impl Settings {
    /// Loads `path`, creating the commented template if it doesn't exist.
    /// `YTM_CLIENT_ID` / `YTM_CLIENT_SECRET` override the file.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(path, TEMPLATE)
                .with_context(|| format!("writing {}", path.display()))?;
        }
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut settings: Settings =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;

        if let Ok(id) = std::env::var("YTM_CLIENT_ID") {
            settings.client_id = id;
        }
        if let Ok(secret) = std::env::var("YTM_CLIENT_SECRET") {
            settings.client_secret = secret;
        }
        for (name, id) in [
            ("client_id", &mut settings.client_id),
            ("device_client_id", &mut settings.device_client_id),
        ] {
            *id = normalize_client_id(id);
            if !id.is_empty() && !is_valid_client_id(id) {
                bail!(
                    "{name} in {} doesn't look like a Google OAuth client ID \
                     (expected `<digits>-<letters>.apps.googleusercontent.com`)",
                    path.display()
                );
            }
        }
        settings.client_secret = settings.client_secret.trim().to_owned();
        settings.device_client_secret = settings.device_client_secret.trim().to_owned();
        settings.volume = settings.volume.clamp(0.0, 1.0);
        Ok(settings)
    }

    pub fn has_oauth_client(&self) -> bool {
        !self.client_id.trim().is_empty()
    }

    /// `audio_device`, or `None` for automatic selection.
    pub fn audio_device(&self) -> Option<String> {
        let name = self.audio_device.trim();
        (!name.is_empty()).then(|| name.to_owned())
    }

    pub fn has_device_client(&self) -> bool {
        !self.device_client_id.trim().is_empty()
    }

    /// Writes OAuth client credentials into `path`, keeping comments and
    /// other settings. `device` selects the `device_client_*` keys.
    pub fn store_client(path: &Path, id: &str, secret: &str, device: bool) -> Result<()> {
        let id = normalize_client_id(id);
        if !is_valid_client_id(&id) {
            bail!("not a Google OAuth client ID: {id}");
        }
        if secret.contains(['"', '\\', '\n']) {
            bail!("unexpected characters in client secret");
        }
        let prefix = if device { "device_client" } else { "client" };
        let text = std::fs::read_to_string(path).unwrap_or_else(|_| TEMPLATE.to_owned());
        let (mut seen_id, mut seen_secret) = (false, false);
        let mut out: Vec<String> = text
            .lines()
            .map(|line| {
                let key = line.split('=').next().unwrap_or("").trim();
                if key == format!("{prefix}_id") {
                    seen_id = true;
                    format!("{prefix}_id = \"{id}\"")
                } else if key == format!("{prefix}_secret") {
                    seen_secret = true;
                    format!("{prefix}_secret = \"{secret}\"")
                } else {
                    line.to_owned()
                }
            })
            .collect();
        // Older config files may lack the keys: prepend (before any table).
        if !seen_secret {
            out.insert(0, format!("{prefix}_secret = \"{secret}\""));
        }
        if !seen_id {
            out.insert(0, format!("{prefix}_id = \"{id}\""));
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, out.join("\n") + "\n")?;
        restrict_permissions(path);
        Ok(())
    }
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) {}

/// Undoes copy-paste damage: terminals and editors sometimes turn the ID
/// into a link (`http://…googleusercontent.com/`).
fn normalize_client_id(raw: &str) -> String {
    let id = raw.trim();
    let id = id
        .strip_prefix("https://")
        .or_else(|| id.strip_prefix("http://"))
        .unwrap_or(id);
    id.trim_end_matches('/').to_owned()
}

fn is_valid_client_id(id: &str) -> bool {
    id.strip_suffix(".apps.googleusercontent.com")
        .and_then(|head| head.split_once('-'))
        .is_some_and(|(num, rest)| {
            !num.is_empty()
                && num.bytes().all(|b| b.is_ascii_digit())
                && !rest.is_empty()
                && rest.bytes().all(|b| b.is_ascii_alphanumeric())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_id_link_damage_is_repaired() {
        let id = "859927131765-abc123.apps.googleusercontent.com";
        assert_eq!(normalize_client_id(&format!("http://{id}/")), id);
        assert_eq!(normalize_client_id(&format!(" https://{id} ")), id);
        assert_eq!(normalize_client_id(id), id);
        assert!(is_valid_client_id(id));
        assert!(!is_valid_client_id("abc.apps.googleusercontent.com"));
        assert!(!is_valid_client_id("859927131765-abc123"));
    }

    #[test]
    fn store_client_rewrites_keys_in_place() {
        let dir = std::env::temp_dir().join(format!("ytm-cfg-test-{}", std::process::id()));
        let path = dir.join("config.toml");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, TEMPLATE).unwrap();
        let id = "123-abc.apps.googleusercontent.com";
        Settings::store_client(&path, &format!("http://{id}/"), "GOCSPX-x", false).unwrap();
        let s = Settings::load(&path).unwrap();
        assert_eq!(s.client_id, id);
        assert_eq!(s.client_secret, "GOCSPX-x");
        assert!(!s.has_device_client());
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("# Startup volume")
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn template_parses_to_defaults() {
        let s: Settings = toml::from_str(TEMPLATE).unwrap();
        assert!(s.js_fallback && s.liked_music_only);
        assert_eq!(s.volume, 0.8);
        assert!(!s.has_oauth_client());
    }
}
