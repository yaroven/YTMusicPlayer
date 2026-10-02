//! OS media integration: hardware/keyboard media keys and the system
//! "Now Playing" widget (MPRIS on Linux, Now Playing on macOS).
//!
//! Optional (`media_controls` in config). On macOS the handlers need a run
//! loop on the main thread — see `main.rs`. Windows (SMTC) needs a window
//! handle: only the GUI has one ([`set_window_handle`]); without it
//! [`MediaControls::new`] returns `None`.

use std::{sync::OnceLock, time::Duration};

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

/// The GUI window's HWND (Windows), set before the session starts.
static WINDOW_HANDLE: OnceLock<usize> = OnceLock::new();

pub fn set_window_handle(handle: usize) {
    let _ = WINDOW_HANDLE.set(handle);
}

pub struct MediaControls {
    inner: souvlaki::MediaControls,
    last_state: Option<PlayState>,
}

impl MediaControls {
    pub fn new(tx: UnboundedSender<MediaAction>) -> Option<Self> {
        use souvlaki::{MediaControlEvent as E, PlatformConfig, SeekDirection};

        let hwnd = WINDOW_HANDLE.get().map(|&h| h as *mut std::ffi::c_void);
        if cfg!(windows) && hwnd.is_none() {
            return None;
        }
        let config = PlatformConfig {
            display_name: "ytm-player",
            dbus_name: "ytm_player",
            hwnd,
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
