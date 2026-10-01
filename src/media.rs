//! OS media integration: hardware/keyboard media keys and the system
//! "Now Playing" widget (MPRIS on Linux, Now Playing on macOS).
//!
//! Optional (`media_controls` in config). On macOS the handlers need a run
//! loop on the main thread — see `main.rs`. Windows needs a window handle and
//! isn't supported yet; [`MediaControls::new`] returns `None` there.

use std::time::Duration;

use tokio::sync::mpsc::UnboundedSender;

use crate::{api::models::Track, audio::player::PlayState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaAction {
    Toggle,
    Play,
    Pause,
    Next,
    Prev,
    SeekBy(i64),
    SeekTo(Duration),
}

#[cfg(not(windows))]
pub struct MediaControls {
    inner: souvlaki::MediaControls,
    last_state: Option<PlayState>,
}

#[cfg(not(windows))]
impl MediaControls {
    pub fn new(tx: UnboundedSender<MediaAction>) -> Option<Self> {
        use souvlaki::{MediaControlEvent as E, PlatformConfig, SeekDirection};

        let config = PlatformConfig {
            display_name: "ytm-player",
            dbus_name: "ytm_player",
            hwnd: None,
        };
        let mut inner = souvlaki::MediaControls::new(config)
            .map_err(|e| tracing::warn!("media controls unavailable: {e:?}"))
            .ok()?;
        let attached = inner.attach(move |event| {
            let action = match event {
                E::Toggle => MediaAction::Toggle,
                E::Play => MediaAction::Play,
                E::Pause | E::Stop => MediaAction::Pause,
                E::Next => MediaAction::Next,
                E::Previous => MediaAction::Prev,
                E::Seek(SeekDirection::Forward) => MediaAction::SeekBy(10),
                E::Seek(SeekDirection::Backward) => MediaAction::SeekBy(-10),
                E::SeekBy(dir, d) => {
                    let secs = d.as_secs() as i64;
                    MediaAction::SeekBy(if matches!(dir, SeekDirection::Forward) {
                        secs
                    } else {
                        -secs
                    })
                }
                E::SetPosition(p) => MediaAction::SeekTo(p.0),
                _ => return,
            };
            let _ = tx.send(action);
        });
        if let Err(e) = attached {
            tracing::warn!("media controls: attach failed: {e:?}");
            return None;
        }
        Some(Self {
            inner,
            last_state: None,
        })
    }

    pub fn set_track(&mut self, track: &Track, duration: Option<Duration>) {
        let _ = self.inner.set_metadata(souvlaki::MediaMetadata {
            title: Some(&track.title),
            artist: Some(&track.artist),
            duration,
            ..Default::default()
        });
    }

    /// Cheap to call often: only talks to the OS when the state changes or
    /// on `force` (e.g. after a seek, so the widget's clock re-syncs).
    pub fn set_state(&mut self, state: PlayState, position: Duration, force: bool) {
        use souvlaki::{MediaPlayback, MediaPosition};

        if !force && self.last_state == Some(state) {
            return;
        }
        self.last_state = Some(state);
        let progress = Some(MediaPosition(position));
        let playback = match state {
            PlayState::Playing => MediaPlayback::Playing { progress },
            PlayState::Paused => MediaPlayback::Paused { progress },
            PlayState::Idle => MediaPlayback::Stopped,
        };
        let _ = self.inner.set_playback(playback);
    }
}

#[cfg(windows)]
pub struct MediaControls;

#[cfg(windows)]
impl MediaControls {
    pub fn new(_tx: UnboundedSender<MediaAction>) -> Option<Self> {
        None
    }
    pub fn set_track(&mut self, _track: &Track, _duration: Option<Duration>) {}
    pub fn set_state(&mut self, _state: PlayState, _position: Duration, _force: bool) {}
}
