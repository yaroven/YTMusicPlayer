//! Playback engine on a dedicated OS thread (rodio's device sink is not
//! `Send`). Commands go in over a channel; state comes out via a `watch`
//! (polled by the UI) and discrete events via an mpsc (track ended, error).
//!
//! The audio device is opened on the first track and closed again after
//! [`IDLE_CLOSE`] without playback, so an idle player holds no audio buffers.

use std::{
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use anyhow::Result;
use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player, Source};
use tokio::sync::{mpsc::UnboundedSender, watch};

use super::stream::HttpStream;

const IDLE_CLOSE: Duration = Duration::from_secs(30);

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

/// `generation` identifies the load an event belongs to, so stale events
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
    SetPaused(bool),
    Stop,
    SeekBy(i64),
    SeekTo(Duration),
    SetVolume(f32),
}

/// What a Session needs from audio output: rodio's [`PlayerHandle`] in the
/// app, a scripted fake in tests. Events come back on the channel given to
/// the adapter.
pub trait Playback: Send {
    /// Replaces whatever is playing; `generation` tags its events.
    fn load(&self, stream: HttpStream, duration: Option<Duration>, generation: u64);
    fn toggle_pause(&self);
    fn set_paused(&self, paused: bool);
    fn stop(&self);
    fn seek_by(&self, secs: i64);
    fn seek_to(&self, position: Duration);
    /// Clamped to 0..=1.
    fn set_volume(&self, volume: f32);
    fn status(&self) -> PlayerStatus;

    fn change_volume(&self, delta: f32) {
        self.set_volume(self.status().volume + delta);
    }
}

#[derive(Clone)]
pub struct PlayerHandle {
    tx: mpsc::Sender<Command>,
    status: watch::Receiver<PlayerStatus>,
}

impl PlayerHandle {
    /// Starts the audio thread. The device itself is opened lazily; failures
    /// to open it are reported as [`PlayerEvent::Error`] on the first load.
    /// `device`: output device name (see [`output_device_names`]); `None`
    /// picks automatically.
    pub fn spawn(
        volume: f32,
        device: Option<String>,
        events: UnboundedSender<PlayerEvent>,
    ) -> Result<Self> {
        let (tx, rx) = mpsc::channel();
        let (status_tx, status) = watch::channel(PlayerStatus {
            state: PlayState::Idle,
            position: Duration::ZERO,
            duration: None,
            volume,
        });
        thread::Builder::new()
            .name("audio".into())
            // Decoding needs little stack; the default 2 MiB is reserved per thread.
            .stack_size(256 * 1024)
            .spawn(move || Engine::new(status_tx, events, volume, device).run(rx))?;
        Ok(Self { tx, status })
    }

    fn send(&self, cmd: Command) {
        // Only fails if the audio thread died.
        let _ = self.tx.send(cmd);
    }
}

/// Decoding starts on the audio thread; calls never block.
impl Playback for PlayerHandle {
    fn load(&self, stream: HttpStream, duration: Option<Duration>, generation: u64) {
        self.send(Command::Load {
            stream,
            duration,
            generation,
        });
    }

    fn toggle_pause(&self) {
        self.send(Command::TogglePause);
    }

    fn set_paused(&self, paused: bool) {
        self.send(Command::SetPaused(paused));
    }

    fn stop(&self) {
        self.send(Command::Stop);
    }

    fn seek_by(&self, secs: i64) {
        self.send(Command::SeekBy(secs));
    }

    fn seek_to(&self, position: Duration) {
        self.send(Command::SeekTo(position));
    }

    fn set_volume(&self, volume: f32) {
        self.send(Command::SetVolume(volume.clamp(0.0, 1.0)));
    }

    fn status(&self) -> PlayerStatus {
        self.status.borrow().clone()
    }
}

struct Output {
    // Field order matters: the player must drop before its device.
    player: Player,
    _sink: MixerDeviceSink,
}

struct Engine {
    output: Option<Output>,
    /// (generation, duration) of the loaded track.
    current: Option<(u64, Option<Duration>)>,
    volume: f32,
    idle_since: Instant,
    status_tx: watch::Sender<PlayerStatus>,
    events: UnboundedSender<PlayerEvent>,
    device: Option<String>,
}

impl Engine {
    fn new(
        status_tx: watch::Sender<PlayerStatus>,
        events: UnboundedSender<PlayerEvent>,
        volume: f32,
        device: Option<String>,
    ) -> Self {
        Self {
            device,
            output: None,
            current: None,
            volume,
            idle_since: Instant::now(),
            status_tx,
            events,
        }
    }

    fn run(mut self, rx: mpsc::Receiver<Command>) {
        loop {
            // Poll faster while playing (position updates), slowly when idle.
            let timeout = if self.current.is_some() {
                Duration::from_millis(100)
            } else {
                Duration::from_secs(1)
            };
            match rx.recv_timeout(timeout) {
                Ok(cmd) => self.handle(cmd),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
            self.tick();
        }
    }

    fn open_output(&mut self) -> Result<&Player, String> {
        if self.output.is_none() {
            let mut sink = open_sink(self.device.as_deref())?;
            // Default prints to stderr on drop, which would corrupt the TUI.
            sink.log_on_drop(false);
            let player = Player::connect_new(sink.mixer());
            player.set_volume(self.volume);
            self.output = Some(Output {
                player,
                _sink: sink,
            });
            tracing::debug!("audio output opened");
        }
        Ok(&self.output.as_ref().expect("just opened").player)
    }

    fn error(&self, generation: u64, message: String) {
        let _ = self.events.send(PlayerEvent::Error {
            generation,
            message,
        });
    }

    fn handle(&mut self, cmd: Command) {
        match cmd {
            Command::Load {
                stream,
                duration,
                generation,
            } => {
                self.current = None;
                let player = match self.open_output() {
                    Ok(player) => player,
                    Err(message) => return self.error(generation, message),
                };
                player.clear();
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
                        self.current = Some((generation, duration));
                    }
                    Err(err) => self.error(generation, format!("cannot decode audio: {err}")),
                }
            }
            Command::TogglePause => {
                if let (Some(out), Some(_)) = (&self.output, self.current) {
                    if out.player.is_paused() {
                        out.player.play()
                    } else {
                        out.player.pause()
                    }
                }
            }
            Command::SetPaused(paused) => {
                if let (Some(out), Some(_)) = (&self.output, self.current) {
                    if paused {
                        out.player.pause()
                    } else {
                        out.player.play()
                    }
                }
            }
            Command::Stop => {
                if let Some(out) = &self.output {
                    out.player.clear();
                }
                self.current = None;
            }
            Command::SeekBy(secs) => {
                if let Some(out) = &self.output {
                    let pos = out.player.get_pos();
                    let delta = Duration::from_secs(secs.unsigned_abs());
                    let target = if secs < 0 {
                        pos.saturating_sub(delta)
                    } else {
                        pos + delta
                    };
                    self.seek(target);
                }
            }
            Command::SeekTo(target) => self.seek(target),
            Command::SetVolume(v) => {
                self.volume = v;
                if let Some(out) = &self.output {
                    out.player.set_volume(v);
                }
            }
        }
    }

    fn seek(&self, mut target: Duration) {
        let (Some(out), Some((generation, duration))) = (&self.output, self.current) else {
            return;
        };
        if let Some(d) = duration {
            target = target.min(d.saturating_sub(Duration::from_secs(1)));
        }
        if let Err(err) = out.player.try_seek(target) {
            self.error(generation, format!("seek failed: {err}"));
        }
    }

    fn tick(&mut self) {
        if let (Some((generation, _)), Some(out)) = (self.current, &self.output)
            && out.player.empty()
        {
            self.current = None;
            let _ = self.events.send(PlayerEvent::Ended { generation });
        }

        let playing = self.current.is_some();
        if playing {
            self.idle_since = Instant::now();
        } else if self.output.is_some() && self.idle_since.elapsed() >= IDLE_CLOSE {
            self.output = None;
            tracing::debug!("audio output closed after idle");
        }

        let (state, position) = match (&self.output, self.current) {
            (Some(out), Some(_)) if out.player.is_paused() => {
                (PlayState::Paused, out.player.get_pos())
            }
            (Some(out), Some(_)) => (PlayState::Playing, out.player.get_pos()),
            _ => (PlayState::Idle, Duration::ZERO),
        };
        let status = PlayerStatus {
            state,
            position,
            duration: self.current.and_then(|(_, d)| d),
            volume: self.volume,
        };
        self.status_tx.send_if_modified(|old| {
            let changed = *old != status;
            *old = status;
            changed
        });
    }
}

/// Names of the available output devices, for the `audio_device` setting.
pub fn output_device_names() -> Vec<String> {
    use rodio::cpal::traits::HostTrait;
    rodio::cpal::default_host()
        .output_devices()
        .map(|devices| devices.filter_map(|d| device_name(&d)).collect())
        .unwrap_or_default()
}

fn device_name(device: &rodio::cpal::Device) -> Option<String> {
    use rodio::cpal::traits::DeviceTrait;
    // On ALSA this is the PCM id ("pipewire", "default:CARD=PCH"), which is
    // what users see in `aplay -L`; `description()` would be less specific.
    #[allow(deprecated)]
    device.name().ok()
}

fn open_named(name: &str) -> Result<MixerDeviceSink, String> {
    use rodio::cpal::traits::HostTrait;
    let device = rodio::cpal::default_host()
        .output_devices()
        .map_err(|e| format!("cannot list audio devices: {e}"))?
        .find(|d| device_name(d).as_deref() == Some(name))
        .ok_or_else(|| format!("audio device {name:?} not found (see `ytm devices`)"))?;
    DeviceSinkBuilder::from_device(device)
        .and_then(|b| b.open_stream())
        .map_err(|e| format!("cannot open audio device {name:?}: {e}"))
}

/// Opens the configured device, or picks one. On Linux, ALSA's `default`
/// may point straight at a sound card that PipeWire/PulseAudio already owns
/// (no `pipewire-alsa` installed) — silence. Their ALSA plugins are tried
/// first so sound reaches the sound server either way.
fn open_sink(configured: Option<&str>) -> Result<MixerDeviceSink, String> {
    if let Some(name) = configured {
        return open_named(name);
    }
    #[cfg(target_os = "linux")]
    {
        let available = output_device_names();
        for server in ["pipewire", "pulse"] {
            if available.iter().any(|n| n == server) {
                match open_named(server) {
                    Ok(sink) => {
                        tracing::info!(device = server, "audio output");
                        return Ok(sink);
                    }
                    Err(err) => tracing::warn!(%err, "falling back"),
                }
            }
        }
    }
    DeviceSinkBuilder::open_default_sink().map_err(|e| format!("cannot open audio output: {e}"))
}
