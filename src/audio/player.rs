//! Playback engine on a dedicated OS thread (rodio's device sink is not
//! `Send`). Commands go in over a channel; state comes out via a `watch`
//! (polled by the UI) and discrete events via an mpsc (track ended or
//! taken over by the preloaded one, error).
//!
//! Each track plays on its own rodio player. A preloaded next track is
//! appended to the current player's queue (gapless), or — with a crossfade —
//! started on a second player during the last seconds while the first fades
//! out. Loudness normalization is a per-track gain on the decoded samples.
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

use super::stream::Media;

const IDLE_CLOSE: Duration = Duration::from_secs(30);
/// How long before the end a gapless next track joins the queue.
const GAPLESS_LEAD: Duration = Duration::from_secs(2);

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
    /// The track ended and nothing was queued after it.
    Ended {
        generation: u64,
    },
    /// A preloaded track took over (gapless or after a crossfade).
    Started {
        generation: u64,
    },
    Error {
        generation: u64,
        message: String,
    },
}

/// A track to play.
pub struct Load {
    /// The audio; `None` when only `url` is known (casting).
    pub media: Option<Media>,
    /// Direct stream URL, for players that fetch it themselves (Chromecast).
    pub url: Option<String>,
    /// What it is (for a receiver's "now playing").
    pub track: Option<crate::api::models::Track>,
    /// When the media doesn't say.
    pub duration: Option<Duration>,
    /// Loudness normalization factor (1.0 = as is).
    pub gain: f32,
    pub generation: u64,
}

enum Command {
    Load(Load),
    Preload(Load),
    CancelPreload,
    SetCrossfade(Duration),
    TogglePause,
    SetPaused(bool),
    Stop,
    SeekBy(i64),
    SeekTo(Duration),
    SetVolume(f32),
}

/// What a Session needs from audio output: rodio's [`PlayerHandle`] in the
/// app, a recording fake in tests. Events come back on the channel given to
/// the adapter.
pub trait Playback: Send {
    /// Replaces whatever is playing.
    fn load(&self, track: Load);
    /// Plays `track` right after the current one: gapless, or overlapping by
    /// the crossfade. Replaces an earlier preload.
    fn preload(&self, track: Load);
    fn cancel_preload(&self);
    /// Overlap between tracks (zero: gapless).
    fn set_crossfade(&self, overlap: Duration);
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
    fn load(&self, track: Load) {
        self.send(Command::Load(track));
    }

    fn preload(&self, track: Load) {
        self.send(Command::Preload(track));
    }

    fn cancel_preload(&self) {
        self.send(Command::CancelPreload);
    }

    fn set_crossfade(&self, overlap: Duration) {
        self.send(Command::SetCrossfade(overlap));
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

type Decoded = rodio::source::Amplify<Decoder<Media>>;

/// A track on its own rodio player (one per track while they overlap).
struct Slot {
    player: Player,
    generation: u64,
    duration: Option<Duration>,
}

/// A preloaded track waiting for the current one to end.
struct Next {
    decoded: Option<Decoded>,
    generation: u64,
    duration: Option<Duration>,
    /// Appended to the current player's queue (gapless): waiting for the
    /// queue to move on.
    queued: bool,
    /// Cancelled after it was queued: stop when it would start.
    cancelled: bool,
}

/// A track fading out under the new one.
struct Fading {
    player: Player,
    started: Instant,
}

/// Field order matters: players drop before the device they play on.
struct Engine {
    current: Option<Slot>,
    next: Option<Next>,
    fading: Option<Fading>,
    sink: Option<MixerDeviceSink>,
    crossfade: Duration,
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
            sink: None,
            current: None,
            next: None,
            fading: None,
            crossfade: Duration::ZERO,
            volume,
            idle_since: Instant::now(),
            status_tx,
            events,
        }
    }

    fn run(mut self, rx: mpsc::Receiver<Command>) {
        loop {
            // Poll faster while playing (position, fades), slowly when idle.
            let timeout = match (&self.fading, &self.current) {
                (Some(_), _) => Duration::from_millis(40),
                (None, Some(_)) => Duration::from_millis(100),
                (None, None) => Duration::from_secs(1),
            };
            match rx.recv_timeout(timeout) {
                Ok(cmd) => self.handle(cmd),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
            self.tick();
        }
    }

    fn new_player(&mut self) -> Result<Player, String> {
        if self.sink.is_none() {
            let mut sink = open_sink(self.device.as_deref())?;
            // Default prints to stderr on drop, which would corrupt the TUI.
            sink.log_on_drop(false);
            self.sink = Some(sink);
            tracing::debug!("audio output opened");
        }
        let player = Player::connect_new(self.sink.as_ref().expect("just opened").mixer());
        player.set_volume(self.volume);
        Ok(player)
    }

    fn error(&self, generation: u64, message: String) {
        let _ = self.events.send(PlayerEvent::Error {
            generation,
            message,
        });
    }

    /// Reads the container header: may block briefly on the network.
    fn decode(load: Load) -> Result<(Decoded, Option<Duration>), String> {
        let media = load.media.ok_or("nothing to play (no audio data)")?;
        let len = media.len();
        let decoder = Decoder::builder()
            .with_data(media)
            .with_byte_len(len)
            .with_seekable(true)
            .with_hint("m4a")
            .with_mime_type("audio/mp4")
            .build()
            .map_err(|err| format!("cannot decode audio: {err}"))?;
        let duration = decoder.total_duration().or(load.duration);
        Ok((decoder.amplify(load.gain), duration))
    }

    fn handle(&mut self, cmd: Command) {
        match cmd {
            Command::Load(load) => {
                let generation = load.generation;
                self.stop_all();
                let player = match self.new_player() {
                    Ok(player) => player,
                    Err(message) => return self.error(generation, message),
                };
                match Self::decode(load) {
                    Ok((decoded, duration)) => {
                        player.append(decoded);
                        player.play();
                        self.current = Some(Slot {
                            player,
                            generation,
                            duration,
                        });
                    }
                    Err(message) => self.error(generation, message),
                }
            }
            Command::Preload(load) => {
                self.cancel_next();
                let generation = load.generation;
                match Self::decode(load) {
                    Ok((decoded, duration)) => {
                        self.next = Some(Next {
                            decoded: Some(decoded),
                            generation,
                            duration,
                            queued: false,
                            cancelled: false,
                        });
                    }
                    Err(message) => self.error(generation, message),
                }
            }
            Command::CancelPreload => self.cancel_next(),
            Command::SetCrossfade(overlap) => self.crossfade = overlap,
            Command::TogglePause => {
                if let Some(slot) = &self.current {
                    if slot.player.is_paused() {
                        slot.player.play()
                    } else {
                        slot.player.pause()
                    }
                }
            }
            Command::SetPaused(paused) => {
                if let Some(slot) = &self.current {
                    if paused {
                        slot.player.pause()
                    } else {
                        slot.player.play()
                    }
                }
            }
            Command::Stop => self.stop_all(),
            Command::SeekBy(secs) => {
                if let Some(slot) = &self.current {
                    let pos = slot.player.get_pos();
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
                if let Some(slot) = &self.current {
                    slot.player.set_volume(v);
                }
            }
        }
    }

    fn stop_all(&mut self) {
        self.current = None;
        self.next = None;
        self.fading = None;
    }

    /// Drops a preload. One already in the gapless queue (last ~2 s) can't
    /// be taken out of rodio's queue: it's stopped when it would start.
    fn cancel_next(&mut self) {
        match &mut self.next {
            Some(next) if next.queued => next.cancelled = true,
            _ => self.next = None,
        }
    }

    fn seek(&self, mut target: Duration) {
        let Some(slot) = &self.current else {
            return;
        };
        if let Some(d) = slot.duration {
            target = target.min(d.saturating_sub(Duration::from_secs(1)));
        }
        if let Err(err) = slot.player.try_seek(target) {
            self.error(slot.generation, format!("seek failed: {err}"));
        }
    }

    fn tick(&mut self) {
        self.advance();
        self.fade();

        let playing = self.current.is_some() || self.fading.is_some();
        if playing {
            self.idle_since = Instant::now();
        } else if self.sink.is_some() && self.idle_since.elapsed() >= IDLE_CLOSE {
            self.sink = None;
            tracing::debug!("audio output closed after idle");
        }

        let (state, position) = match &self.current {
            Some(slot) if slot.player.is_paused() => (PlayState::Paused, slot.player.get_pos()),
            Some(slot) => (PlayState::Playing, slot.player.get_pos()),
            None => (PlayState::Idle, Duration::ZERO),
        };
        let status = PlayerStatus {
            state,
            position,
            duration: self.current.as_ref().and_then(|s| s.duration),
            volume: self.volume,
        };
        self.status_tx.send_if_modified(|old| {
            let changed = *old != status;
            *old = status;
            changed
        });
    }

    /// Moves to the preloaded track when the current one ends (gapless) or
    /// enters its last `crossfade` (overlap), else reports the end.
    fn advance(&mut self) {
        let Some(slot) = &self.current else {
            return;
        };
        let remaining = slot
            .duration
            .map(|d| d.saturating_sub(slot.player.get_pos()));
        // Gapless: append the next track to the queue shortly before the end
        // (late, so a changed queue can still cancel it cleanly).
        if self.crossfade.is_zero()
            && remaining.is_some_and(|r| r <= GAPLESS_LEAD)
            && let Some(next) = &mut self.next
            && let Some(decoded) = next.decoded.take()
        {
            slot.player.append(decoded);
            next.queued = true;
        }
        // Gapless: the queue moved on to the appended track.
        if let Some(next) = &self.next
            && next.queued
            && slot.player.len() <= 1
            && !slot.player.empty()
        {
            let next = self.next.take().expect("checked");
            if next.cancelled {
                let generation = slot.generation;
                self.current = None;
                let _ = self.events.send(PlayerEvent::Ended { generation });
                return;
            }
            let slot = self.current.as_mut().expect("checked");
            slot.generation = next.generation;
            slot.duration = next.duration;
            let _ = self.events.send(PlayerEvent::Started {
                generation: next.generation,
            });
            return;
        }
        // Crossfade: start the next track on its own player, fade the old.
        let overlap_now = !self.crossfade.is_zero()
            && !slot.player.is_paused()
            && remaining.is_some_and(|r| r <= self.crossfade)
            && self.next.as_ref().is_some_and(|n| n.decoded.is_some());
        if overlap_now {
            let next = self.next.take().expect("checked");
            let player = match self.new_player() {
                Ok(player) => player,
                Err(message) => return self.error(next.generation, message),
            };
            player.set_volume(0.0);
            player.append(next.decoded.expect("checked"));
            player.play();
            let old = self.current.replace(Slot {
                player,
                generation: next.generation,
                duration: next.duration,
            });
            self.fading = old.map(|s| Fading {
                player: s.player,
                started: Instant::now(),
            });
            let _ = self.events.send(PlayerEvent::Started {
                generation: next.generation,
            });
            return;
        }
        if slot.player.empty() {
            let generation = slot.generation;
            self.current = None;
            self.next = None;
            let _ = self.events.send(PlayerEvent::Ended { generation });
        }
    }

    /// Volume ramps of a crossfade (equal-power-ish: linear in amplitude).
    fn fade(&mut self) {
        let Some(fading) = &self.fading else {
            return;
        };
        let t = if self.crossfade.is_zero() {
            1.0
        } else {
            (fading.started.elapsed().as_secs_f32() / self.crossfade.as_secs_f32()).min(1.0)
        };
        fading.player.set_volume(self.volume * (1.0 - t));
        if let Some(slot) = &self.current {
            slot.player.set_volume(self.volume * t);
        }
        if t >= 1.0 || fading.player.empty() {
            self.fading = None;
            if let Some(slot) = &self.current {
                slot.player.set_volume(self.volume);
            }
        }
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
