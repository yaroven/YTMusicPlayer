//! Playback engine on a dedicated OS thread (rodio's device sink is not
//! `Send`). Commands go in over a channel; state comes out via a `watch`
//! (polled by the UI) and discrete events via an mpsc (track ended, error).

use std::{sync::mpsc, thread, time::Duration};

use anyhow::{Context, Result, anyhow};
use rodio::{Decoder, DeviceSinkBuilder, Player, Source};
use tokio::sync::{mpsc::UnboundedSender, watch};

use super::stream::HttpStream;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayState {
    Idle,
    Playing,
    Paused,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlayerStatus {
    pub state: PlayState,
    pub position: Duration,
    pub duration: Option<Duration>,
    pub volume: f32,
}

/// `generation` identifies the load a event belongs to, so stale events
/// from a track the user already skipped are ignored.
#[derive(Debug, Clone)]
pub enum PlayerEvent {
    Ended { generation: u64 },
    Error { generation: u64, message: String },
}

enum Command {
    Load {
        stream: HttpStream,
        duration: Option<Duration>,
        generation: u64,
    },
    TogglePause,
    Stop,
    SeekBy(i64),
    SetVolume(f32),
}

#[derive(Clone)]
pub struct PlayerHandle {
    tx: mpsc::Sender<Command>,
    status: watch::Receiver<PlayerStatus>,
}

impl PlayerHandle {
    /// Opens the default output device; fails if there is none.
    pub fn spawn(volume: f32, events: UnboundedSender<PlayerEvent>) -> Result<Self> {
        let (tx, rx) = mpsc::channel();
        let (status_tx, status) = watch::channel(PlayerStatus {
            state: PlayState::Idle,
            position: Duration::ZERO,
            duration: None,
            volume,
        });
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("audio".into())
            .spawn(move || run(rx, status_tx, events, volume, ready_tx))?;
        ready_rx
            .recv()
            .context("audio thread exited")?
            .map_err(|e| anyhow!("cannot open audio output: {e}"))?;
        Ok(Self { tx, status })
    }

    /// Replaces whatever is playing. Decoding starts on the audio thread.
    pub fn load(&self, stream: HttpStream, duration: Option<Duration>, generation: u64) {
        self.send(Command::Load {
            stream,
            duration,
            generation,
        });
    }

    pub fn toggle_pause(&self) {
        self.send(Command::TogglePause);
    }

    pub fn stop(&self) {
        self.send(Command::Stop);
    }

    pub fn seek_by(&self, secs: i64) {
        self.send(Command::SeekBy(secs));
    }

    pub fn change_volume(&self, delta: f32) {
        let volume = (self.status().volume + delta).clamp(0.0, 1.0);
        self.send(Command::SetVolume(volume));
    }

    pub fn status(&self) -> PlayerStatus {
        self.status.borrow().clone()
    }

    fn send(&self, cmd: Command) {
        // Only fails if the audio thread died; the UI shows it via status.
        let _ = self.tx.send(cmd);
    }
}

fn run(
    rx: mpsc::Receiver<Command>,
    status_tx: watch::Sender<PlayerStatus>,
    events: UnboundedSender<PlayerEvent>,
    volume: f32,
    ready: mpsc::SyncSender<Result<(), String>>,
) {
    let mut sink = match DeviceSinkBuilder::open_default_sink() {
        Ok(sink) => sink,
        Err(err) => {
            let _ = ready.send(Err(err.to_string()));
            return;
        }
    };
    // Default prints to stderr on drop, which would corrupt the TUI.
    sink.log_on_drop(false);
    let player = Player::connect_new(sink.mixer());
    player.set_volume(volume);
    let _ = ready.send(Ok(()));

    // (generation, duration) of the loaded track.
    let mut current: Option<(u64, Option<Duration>)> = None;

    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Command::Load {
                stream,
                duration,
                generation,
            }) => {
                player.clear();
                current = None;
                let len = stream.len();
                // Reads the container header: may block briefly on the network.
                let built = Decoder::builder()
                    .with_data(stream)
                    .with_byte_len(len)
                    .with_seekable(true)
                    .with_hint("m4a")
                    .with_mime_type("audio/mp4")
                    .build();
                match built {
                    Ok(decoder) => {
                        let duration = decoder.total_duration().or(duration);
                        player.append(decoder);
                        player.play();
                        current = Some((generation, duration));
                    }
                    Err(err) => {
                        let _ = events.send(PlayerEvent::Error {
                            generation,
                            message: format!("cannot decode audio: {err}"),
                        });
                    }
                }
            }
            Ok(Command::TogglePause) if current.is_some() => {
                if player.is_paused() {
                    player.play()
                } else {
                    player.pause()
                }
            }
            Ok(Command::TogglePause) => {}
            Ok(Command::Stop) => {
                player.clear();
                current = None;
            }
            Ok(Command::SeekBy(secs)) => {
                if let Some((generation, duration)) = current {
                    let pos = player.get_pos();
                    let delta = Duration::from_secs(secs.unsigned_abs());
                    let mut target = if secs < 0 {
                        pos.saturating_sub(delta)
                    } else {
                        pos + delta
                    };
                    if let Some(d) = duration {
                        target = target.min(d.saturating_sub(Duration::from_secs(1)));
                    }
                    if let Err(err) = player.try_seek(target) {
                        let _ = events.send(PlayerEvent::Error {
                            generation,
                            message: format!("seek failed: {err}"),
                        });
                    }
                }
            }
            Ok(Command::SetVolume(v)) => player.set_volume(v),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }

        if let Some((generation, _)) = current
            && player.empty()
        {
            current = None;
            let _ = events.send(PlayerEvent::Ended { generation });
        }

        let status = PlayerStatus {
            state: match (&current, player.is_paused()) {
                (None, _) => PlayState::Idle,
                (Some(_), true) => PlayState::Paused,
                (Some(_), false) => PlayState::Playing,
            },
            position: if current.is_some() {
                player.get_pos()
            } else {
                Duration::ZERO
            },
            duration: current.and_then(|(_, d)| d),
            volume: player.volume(),
        };
        status_tx.send_if_modified(|old| {
            let changed = *old != status;
            *old = status;
            changed
        });
    }
}
