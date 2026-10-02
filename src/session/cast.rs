//! Playing on a Chromecast: the session swaps its [`Playback`] for a
//! [`CastPlayer`] and back, continuing the track where it was.

use std::{sync::atomic::Ordering, time::Duration};

use tokio::sync::mpsc;

use super::{Background, Changes, Session};
use crate::{
    audio::player::{PlayState, Playback, PlayerEvent},
    cast::{self, CastPlayer, Device},
};

/// How long a device scan listens for answers.
const SCAN: Duration = Duration::from_secs(2);

impl Session {
    pub(super) fn cast_event(&mut self, event: Background, changes: &mut Changes) {
        match event {
            Background::CastDevices(result) => {
                self.cast_scanning = false;
                match result {
                    Ok(devices) => {
                        if devices.is_empty() {
                            self.set_info("No Cast devices found on this network");
                        }
                        self.cast_devices = devices.into();
                    }
                    Err(err) => self.set_error(format!("Looking for Cast devices: {err:#}")),
                }
                changes.cast = true;
            }
            Background::CastConnected { name, result } => {
                changes.cast = true;
                match result {
                    Ok((player, events)) => {
                        self.cast_alive = Some(player.alive());
                        self.switch_output(Box::new(player), events, Some(name.clone()));
                        self.set_info(format!("Playing on {name}"));
                    }
                    Err(err) => self.set_error(format!("Can't cast to {name}: {err:#}")),
                }
            }
            _ => unreachable!("not a cast event"),
        }
    }

    /// Looks for Cast devices on the local network.
    pub fn find_cast_devices(&mut self) {
        if self.cast_scanning {
            return;
        }
        self.cast_scanning = true;
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(Background::CastDevices(cast::discover(SCAN).await));
        });
    }

    /// Plays on `device`, or back on this computer (`None`).
    pub fn cast_to(&mut self, device: Option<Device>) {
        let Some(device) = device else {
            if let Some((player, events)) = self.local.take() {
                self.cast_alive = None;
                self.switch_output(player, events, None);
                self.set_info("Playing on this computer");
            }
            return;
        };
        self.set_info(format!("Connecting to {}…", device.name));
        let (tx, volume) = (self.tx.clone(), self.player_status.volume);
        tokio::spawn(async move {
            let name = device.name.clone();
            let (events, rx) = mpsc::unbounded_channel();
            let result = CastPlayer::connect(device, volume, events)
                .await
                .map(|player| (player, rx));
            let _ = tx.send(Background::CastConnected { name, result });
        });
    }

    /// The device went away: continue here.
    pub(super) fn check_cast(&mut self) {
        if self
            .cast_alive
            .as_ref()
            .is_some_and(|alive| !alive.load(Ordering::Relaxed))
        {
            let name = self.casting.clone().unwrap_or_default();
            self.cast_to(None);
            self.set_error(format!("Lost the connection to {name}"));
        }
    }

    fn switch_output(
        &mut self,
        player: Box<dyn Playback>,
        events: mpsc::UnboundedReceiver<PlayerEvent>,
        casting: Option<String>,
    ) {
        let status = self.player.status();
        let resume = self.queue.current().is_some() && status.state != PlayState::Idle;
        self.player.stop();
        let old = std::mem::replace(&mut self.player, player);
        let old_events = std::mem::replace(&mut self.player_rx, events);
        // Keep the local engine for later; a previous cast player just goes
        // (dropping it closes the receiver app).
        if self.casting.is_none() {
            self.local = Some((old, old_events));
        }
        self.casting = casting;
        self.player.set_volume(status.volume);
        self.player.set_crossfade(self.deps.crossfade);
        if resume {
            self.resume_at = Some(status.position);
            self.play_current();
        }
    }
}
