//! Last.fm scrobbling with the user's own API account
//! (<https://www.last.fm/api/account/create>): "now playing" when a track
//! starts, a scrobble once it has played half its length or four minutes
//! (Last.fm's rules; tracks under 30 s are never scrobbled).
//!
//! `ytm lastfm-login` stores a session key in `<data_dir>/lastfm.session`.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::{api::models::Track, session::Listener};

const API: &str = "https://ws.audioscrobbler.com/2.0/";
/// Shortest track Last.fm accepts.
const MIN_LENGTH: Duration = Duration::from_secs(30);
/// Played this long, any track counts.
const ENOUGH: Duration = Duration::from_secs(240);
/// How long `login` waits for the user to allow access.
const LOGIN_WAIT: Duration = Duration::from_secs(180);

/// API key and secret of the user's Last.fm application.
#[derive(Clone)]
pub struct Credentials {
    pub key: String,
    pub secret: String,
}

impl Credentials {
    /// `api_sig`: md5 of the sorted `name value` pairs plus the secret.
    fn sign(&self, params: &mut BTreeMap<&'static str, String>) {
        params.insert("api_key", self.key.clone());
        let mut text = String::new();
        for (k, v) in params.iter() {
            text.push_str(k);
            text.push_str(v);
        }
        text.push_str(&self.secret);
        params.insert("api_sig", format!("{:x}", md5::compute(text)));
    }

    async fn call(
        &self,
        http: &reqwest::Client,
        method: &'static str,
        mut params: BTreeMap<&'static str, String>,
    ) -> Result<Value> {
        params.insert("method", method.to_owned());
        self.sign(&mut params);
        params.insert("format", "json".to_owned());
        let response: Value = http
            .post(API)
            .form(&params)
            .send()
            .await
            .with_context(|| format!("Last.fm {method}"))?
            .json()
            .await
            .with_context(|| format!("Last.fm {method}: bad response"))?;
        if let Some(code) = response.get("error") {
            let message = response["message"].as_str().unwrap_or("");
            bail!("Last.fm {method}: error {code}: {message}");
        }
        Ok(response)
    }
}

pub fn session_file(data_dir: &Path) -> PathBuf {
    data_dir.join("lastfm.session")
}

/// Asks the user to allow access in the browser (`show_url` prints or
/// opens it), waits for that and stores the session key.
pub async fn login(
    http: &reqwest::Client,
    credentials: &Credentials,
    data_dir: &Path,
    show_url: impl FnOnce(&str),
) -> Result<String> {
    let token = credentials
        .call(http, "auth.getToken", BTreeMap::new())
        .await?["token"]
        .as_str()
        .context("Last.fm returned no token")?
        .to_owned();
    show_url(&format!(
        "https://www.last.fm/api/auth/?api_key={}&token={token}",
        credentials.key
    ));
    let deadline = tokio::time::Instant::now() + LOGIN_WAIT;
    loop {
        tokio::time::sleep(Duration::from_secs(3)).await;
        let params = BTreeMap::from([("token", token.clone())]);
        match credentials.call(http, "auth.getSession", params).await {
            Ok(response) => {
                let session = &response["session"];
                let key = session["key"].as_str().context("no session key")?;
                let name = session["name"].as_str().unwrap_or("?").to_owned();
                let path = session_file(data_dir);
                std::fs::create_dir_all(data_dir)?;
                std::fs::write(&path, key)?;
                restrict(&path);
                return Ok(name);
            }
            // 14: not authorized yet.
            Err(err) if err.to_string().contains("error 14") => {}
            Err(err) => return Err(err),
        }
        if tokio::time::Instant::now() > deadline {
            bail!("timed out waiting for Last.fm authorization");
        }
    }
}

#[cfg(unix)]
fn restrict(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict(_path: &Path) {}

/// The track being listened to.
struct Playing {
    track: Track,
    duration: Option<Duration>,
    started: SystemTime,
    /// Time actually played (seeks don't count).
    played: Duration,
    last_position: Duration,
    scrobbled: bool,
}

/// A [`Listener`] that scrobbles; requests run in spawned tasks.
pub struct Scrobbler {
    http: reqwest::Client,
    credentials: Credentials,
    session: String,
    playing: Option<Playing>,
}

impl Scrobbler {
    /// `None` without a stored session (`ytm lastfm-login` not run).
    pub fn new(http: reqwest::Client, credentials: Credentials, data_dir: &Path) -> Option<Self> {
        let session = std::fs::read_to_string(session_file(data_dir)).ok()?;
        let session = session.trim().to_owned();
        (!session.is_empty()).then_some(Self {
            http,
            credentials,
            session,
            playing: None,
        })
    }

    fn send(&self, method: &'static str, mut params: BTreeMap<&'static str, String>) {
        params.insert("sk", self.session.clone());
        let (http, credentials) = (self.http.clone(), self.credentials.clone());
        tokio::spawn(async move {
            if let Err(err) = credentials.call(&http, method, params).await {
                tracing::warn!("{err:#}");
            }
        });
    }

    fn params(track: &Track, duration: Option<Duration>) -> BTreeMap<&'static str, String> {
        let mut params = BTreeMap::from([
            ("artist", artist(track).to_owned()),
            ("track", track.title.to_string()),
        ]);
        if let Some(d) = duration {
            params.insert("duration", d.as_secs().to_string());
        }
        params
    }
}

/// YouTube's "Artist - Topic" channels name the artist; uploads by
/// others keep the channel name.
fn artist(track: &Track) -> &str {
    track
        .artist
        .strip_suffix(" - Topic")
        .unwrap_or(&track.artist)
}

/// Whether `played` of a track `duration` long counts as a listen.
fn counts(played: Duration, duration: Option<Duration>) -> bool {
    match duration {
        Some(d) if d < MIN_LENGTH => false,
        Some(d) => played >= (d / 2).min(ENOUGH),
        None => played >= ENOUGH,
    }
}

impl Listener for Scrobbler {
    fn started(&mut self, track: &Track, duration: Option<Duration>) {
        let duration = duration.or(track.duration_secs.map(|s| Duration::from_secs(s.into())));
        self.send("track.updateNowPlaying", Self::params(track, duration));
        self.playing = Some(Playing {
            track: track.clone(),
            duration,
            started: SystemTime::now(),
            played: Duration::ZERO,
            last_position: Duration::ZERO,
            scrobbled: false,
        });
    }

    fn progress(&mut self, track: &Track, position: Duration, playing: bool) {
        let Some(p) = self.playing.as_mut() else {
            return;
        };
        if p.track.video_id != track.video_id {
            return;
        }
        // Count normal playback only: small forward steps.
        let step = position.saturating_sub(p.last_position);
        if playing && position >= p.last_position && step <= Duration::from_secs(3) {
            p.played += step;
        }
        p.last_position = position;
        if !p.scrobbled && counts(p.played, p.duration) {
            p.scrobbled = true;
            let mut params = Self::params(&p.track, p.duration);
            let at = p.started.duration_since(UNIX_EPOCH).unwrap_or_default();
            params.insert("timestamp", at.as_secs().to_string());
            self.send("track.scrobble", params);
        }
    }

    fn stopped(&mut self) {
        self.playing = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_follows_last_fm_rules() {
        let c = Credentials {
            key: "k".into(),
            secret: "s".into(),
        };
        let mut params = BTreeMap::from([("method", "auth.getToken".to_owned())]);
        c.sign(&mut params);
        let expected = format!("{:x}", md5::compute("api_keykmethodauth.getTokens"));
        assert_eq!(params["api_sig"], expected);
    }

    #[test]
    fn half_or_four_minutes_counts() {
        let secs = Duration::from_secs;
        assert!(!counts(secs(20), Some(secs(25))), "too short a track");
        assert!(!counts(secs(89), Some(secs(180))));
        assert!(counts(secs(90), Some(secs(180))));
        assert!(counts(secs(240), Some(secs(3600))));
        assert!(!counts(secs(100), None));
    }

    #[test]
    fn topic_channels_name_the_artist() {
        let t = Track {
            video_id: "x".into(),
            title: "t".into(),
            artist: "ABBA - Topic".into(),
            duration_secs: None,
        };
        assert_eq!(artist(&t), "ABBA");
    }
}
