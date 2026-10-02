//! Discord "Listening to" status over Discord's local IPC socket, with the
//! user's own application id (`discord_client_id`). The IPC client lives on
//! its own thread: connecting and writing block, and Discord may not run.

use std::{
    sync::mpsc::{self, Receiver, Sender},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use discord_rich_presence::{
    DiscordIpc, DiscordIpcClient,
    activity::{Activity, ActivityType, Assets, Button, Timestamps},
};

use crate::{api::models::Track, session::Listener};

/// Wait this long before trying to reach Discord again.
const RETRY: Duration = Duration::from_secs(30);
/// A position this far from the expected one is a seek.
const JUMP: Duration = Duration::from_secs(3);

enum Update {
    Show {
        track: Track,
        /// Unix ms when the track (re)started from 0, and when it ends.
        start: i64,
        end: Option<i64>,
    },
    Clear,
}

pub struct Presence {
    tx: Sender<Update>,
    /// The track, its length, and the playing / position last reported.
    current: Option<(Track, Option<Duration>)>,
    playing: bool,
    position: Duration,
}

impl Presence {
    pub fn new(client_id: &str) -> Option<Self> {
        let (tx, rx) = mpsc::channel();
        let id = client_id.to_owned();
        std::thread::Builder::new()
            .name("discord".into())
            .stack_size(256 * 1024)
            .spawn(move || run(&id, rx))
            .ok()?;
        Some(Self {
            tx,
            current: None,
            playing: false,
            position: Duration::ZERO,
        })
    }

    fn show(&self, position: Duration) {
        let Some((track, duration)) = &self.current else {
            return;
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let start = now.saturating_sub(position);
        let _ = self.tx.send(Update::Show {
            track: track.clone(),
            start: start.as_millis() as i64,
            end: duration.map(|d| (start + d).as_millis() as i64),
        });
    }
}

impl Listener for Presence {
    fn started(&mut self, track: &Track, duration: Option<Duration>) {
        self.current = Some((track.clone(), duration));
        self.playing = true;
        self.position = Duration::ZERO;
        self.show(Duration::ZERO);
    }

    fn progress(&mut self, _track: &Track, position: Duration, playing: bool) {
        let expected = self.position + Duration::from_millis(500);
        let jumped = position.abs_diff(expected) > JUMP;
        if playing != self.playing {
            if playing {
                self.show(position);
            } else {
                let _ = self.tx.send(Update::Clear);
            }
        } else if playing && jumped {
            self.show(position);
        }
        self.playing = playing;
        self.position = position;
    }

    fn stopped(&mut self) {
        self.current = None;
        self.playing = false;
        let _ = self.tx.send(Update::Clear);
    }
}

fn run(client_id: &str, rx: Receiver<Update>) {
    let mut client: Option<DiscordIpcClient> = None;
    let mut failed_at: Option<Instant> = None;
    for mut update in rx.iter() {
        // Only the latest update matters.
        while let Ok(newer) = rx.try_recv() {
            update = newer;
        }
        if client.is_none() && failed_at.is_none_or(|t| t.elapsed() > RETRY) {
            let mut c = DiscordIpcClient::new(client_id);
            match c.connect() {
                Ok(()) => client = Some(c),
                Err(err) => {
                    tracing::debug!(%err, "discord not reachable");
                    failed_at = Some(Instant::now());
                }
            }
        }
        let Some(c) = client.as_mut() else {
            continue;
        };
        let result = match &update {
            Update::Show { track, start, end } => c.set_activity(activity(track, *start, *end)),
            Update::Clear => c.clear_activity(),
        };
        if let Err(err) = result {
            tracing::debug!(%err, "discord presence");
            let _ = c.close();
            client = None;
            failed_at = Some(Instant::now());
        }
    }
    if let Some(mut c) = client {
        let _ = c.close();
    }
}

fn activity(track: &Track, start: i64, end: Option<i64>) -> Activity<'static> {
    let mut timestamps = Timestamps::new().start(start);
    if let Some(end) = end {
        timestamps = timestamps.end(end);
    }
    let id = &track.video_id;
    Activity::new()
        .activity_type(ActivityType::Listening)
        .details(track.title.to_string())
        .state(track.artist.to_string())
        .timestamps(timestamps)
        .assets(
            Assets::new()
                .large_image(format!("https://i.ytimg.com/vi/{id}/mqdefault.jpg"))
                .large_text(track.title.to_string()),
        )
        .buttons(vec![Button::new(
            "Listen on YouTube Music",
            format!("https://music.youtube.com/watch?v={id}"),
        )])
}
