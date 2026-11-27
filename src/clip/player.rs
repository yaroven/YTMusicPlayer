//! A clip playing in the background: its own thread fetches the segments
//! (one ahead), demuxes and decodes them and keeps a few pictures ready;
//! the window takes the one due at the song's position. Memory stays at
//! [`AHEAD`] pictures, one segment or two (~0.2–1 MB each) and the
//! decoder's own frames.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use slint::{Rgb8Pixel, SharedPixelBuffer};
use tokio::{sync::Notify, task::JoinHandle};

use super::{
    VideoDecoder, hls,
    ts::{Unit, VideoDemux},
};
use crate::audio::innertube::InnerTube;

/// Pictures decoded ahead (~120 ms at 25 fps; up to 2.6 MB each at 720p):
/// the decoder keeps up with plenty to spare.
const AHEAD: usize = 3;
/// A picture this much before the seek point still shows (the first one).
const SLACK: Duration = Duration::from_millis(80);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
/// Tries per segment (a fresh playlist's segments may answer 403 at first).
const SEGMENT_TRIES: u32 = 4;

/// A picture and when it's due on the clip's timeline.
pub struct Frame {
    pub at: Duration,
    pub pixels: SharedPixelBuffer<Rgb8Pixel>,
}

#[derive(Default)]
struct State {
    frames: VecDeque<Frame>,
    /// Where (clip time) the pictures should start, and the seek that set it.
    from: Duration,
    generation: u64,
    /// Largest picture wanted, physical pixels.
    size: (u32, u32),
    stop: bool,
    failed: Option<String>,
}

struct Shared {
    state: Mutex<State>,
    /// Room in the queue, a seek, or stop.
    wake: Notify,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The clip's pictures, decoded ahead in a thread of its own; stops when
/// dropped.
pub struct ClipPlayer {
    shared: Arc<Shared>,
}

impl ClipPlayer {
    /// Starts `video_id` at `at` (clip time), pictures at most `size`.
    pub fn start(
        http: reqwest::Client,
        video_id: String,
        at: Duration,
        size: (u32, u32),
    ) -> Result<Self> {
        let decoder = super::decoder().context("no H.264 decoder on this system")?;
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                from: at,
                size,
                ..State::default()
            }),
            wake: Notify::new(),
        });
        let worker = shared.clone();
        std::thread::Builder::new()
            .name("clip".into())
            .spawn(move || run(&worker, http, &video_id, decoder))?;
        Ok(Self { shared })
    }

    /// The newest picture due by `at` (clip time); older ones are dropped.
    pub fn take(&self, at: Duration) -> Option<Frame> {
        let mut state = self.shared.lock();
        let mut due = None;
        while state.frames.front().is_some_and(|f| f.at <= at) {
            due = state.frames.pop_front();
        }
        drop(state);
        if due.is_some() {
            self.shared.wake.notify_one();
        }
        due
    }

    /// Goes on from `at` (clip time): the pictures queued are thrown away.
    pub fn seek(&self, at: Duration) {
        let mut state = self.shared.lock();
        state.from = at;
        state.generation += 1;
        state.frames.clear();
        drop(state);
        self.shared.wake.notify_one();
    }

    pub fn set_size(&self, size: (u32, u32)) {
        self.shared.lock().size = size;
    }

    /// Why the clip stopped, when it failed.
    pub fn failed(&self) -> Option<String> {
        self.shared.lock().failed.clone()
    }
}

impl Drop for ClipPlayer {
    fn drop(&mut self) {
        self.shared.lock().stop = true;
        self.shared.wake.notify_one();
    }
}

fn run(shared: &Shared, http: reqwest::Client, video_id: &str, mut decoder: Box<dyn VideoDecoder>) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            shared.lock().failed = Some(err.to_string());
            return;
        }
    };
    if let Err(err) = runtime.block_on(play(shared, &http, video_id, decoder.as_mut())) {
        tracing::warn!(video_id, "clip: {err:#}");
        shared.lock().failed = Some(format!("{err:#}"));
    }
}

/// What a wait for room in the queue ended with.
enum Woke {
    Room,
    Seek,
    Stop,
}

async fn play(
    shared: &Shared,
    http: &reqwest::Client,
    video_id: &str,
    decoder: &mut dyn VideoDecoder,
) -> Result<()> {
    let (master_url, user_agent) = InnerTube::new(http.clone())
        .hls_manifest(video_id)
        .await
        .context("YouTube gave no clip")?;
    let master = text(http, &master_url, &user_agent).await?;
    let variants = hls::variants(&master, &master_url);
    let mut variant = hls::pick(&variants, shared.lock().size.1)
        .context("no H.264 variant")?
        .clone();
    let mut segments = playlist(http, &variant, &user_agent).await?;
    tracing::info!(
        video_id,
        height = variant.height,
        segments = segments.len(),
        "clip"
    );

    let mut demux = VideoDemux::default();
    let mut pictures = Vec::new();
    'seek: loop {
        let (generation, from) = {
            let state = shared.lock();
            if state.stop {
                return Ok(());
            }
            (state.generation, state.from)
        };
        decoder.reset();
        pictures.clear();
        let mut index = hls::segment_at(&segments, from);
        let mut next = Some(fetch(http, &segments[index].url, &user_agent));
        while let Some(task) = next.take() {
            let body = task.await.context("segment task")??;
            if index + 1 < segments.len() {
                next = Some(fetch(http, &segments[index + 1].url, &user_agent));
            }
            let start = segments[index].start;
            let units: Vec<Unit> = demux.units(&body);
            drop(body);
            let base = units.iter().map(|u| u.pts).min().unwrap_or_default();
            for unit in units {
                let (width, height) = shared.lock().size;
                decoder.set_size(width, height);
                decoder.decode(&unit.data, unit.pts, &mut pictures);
                for picture in pictures.drain(..) {
                    let ticks = (picture.pts - base).max(0) as f64;
                    let at = start + Duration::from_secs_f64(ticks / 90_000.0);
                    if at + SLACK < from {
                        continue;
                    }
                    let frame = Frame {
                        at,
                        pixels: picture.pixels,
                    };
                    match queue(shared, generation, frame).await {
                        Woke::Room => {}
                        Woke::Seek => {
                            if let Some(task) = next.take() {
                                task.abort();
                            }
                            continue 'seek;
                        }
                        Woke::Stop => return Ok(()),
                    }
                }
            }
            index += 1;
            // The window grew or shrank: the next segments from the variant
            // that fits (variants share their segment boundaries).
            let wanted = shared.lock().size.1;
            if let Some(fits) = hls::pick(&variants, wanted)
                && fits.url != variant.url
                && index < segments.len()
            {
                let switched = playlist(http, fits, &user_agent).await?;
                if switched.len() == segments.len() {
                    tracing::info!(from = variant.height, to = fits.height, "clip variant");
                    if let Some(task) = next.take() {
                        task.abort();
                    }
                    (variant, segments) = (fits.clone(), switched);
                    next = Some(fetch(http, &segments[index].url, &user_agent));
                }
            }
        }
        // The clip has ended: wait for a seek back, or the end.
        loop {
            shared.wake.notified().await;
            let state = shared.lock();
            if state.stop {
                return Ok(());
            }
            if state.generation != generation {
                continue 'seek;
            }
        }
    }
}

/// Queues `frame` once there's room (unless a seek or stop came first).
async fn queue(shared: &Shared, generation: u64, frame: Frame) -> Woke {
    let mut frame = Some(frame);
    loop {
        {
            let mut state = shared.lock();
            if state.stop {
                return Woke::Stop;
            }
            if state.generation != generation {
                return Woke::Seek;
            }
            if state.frames.len() < AHEAD {
                state.frames.extend(frame.take());
                return Woke::Room;
            }
        }
        shared.wake.notified().await;
    }
}

/// The segments of `variant`'s media playlist.
async fn playlist(
    http: &reqwest::Client,
    variant: &hls::Variant,
    user_agent: &str,
) -> Result<Vec<hls::Segment>> {
    let segments = hls::segments(&text(http, &variant.url, user_agent).await?, &variant.url);
    if segments.is_empty() {
        bail!("empty clip playlist");
    }
    Ok(segments)
}

async fn text(http: &reqwest::Client, url: &str, user_agent: &str) -> Result<String> {
    Ok(http
        .get(url)
        .header("User-Agent", user_agent)
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?)
}

/// Fetches a segment in the background; 403 / 5xx are tried again.
fn fetch(http: &reqwest::Client, url: &str, user_agent: &str) -> JoinHandle<Result<Vec<u8>>> {
    let (http, url, user_agent) = (http.clone(), url.to_owned(), user_agent.to_owned());
    tokio::spawn(async move {
        let mut wait = Duration::from_millis(500);
        for attempt in 1..=SEGMENT_TRIES {
            let response = http
                .get(&url)
                .header("User-Agent", &user_agent)
                .timeout(REQUEST_TIMEOUT)
                .send()
                .await?;
            let status = response.status();
            if status.is_success() {
                return Ok(response.bytes().await?.to_vec());
            }
            if attempt == SEGMENT_TRIES || !(status.as_u16() == 403 || status.is_server_error()) {
                bail!("clip segment: HTTP {status}");
            }
            tokio::time::sleep(wait).await;
            wait *= 2;
        }
        unreachable!("the last try returns")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole pipeline on a real clip: `cargo test -- --ignored clip`.
    #[test]
    #[ignore = "network"]
    fn live_clip_pictures_arrive_in_order() {
        if !super::super::available() {
            eprintln!("no H.264 decoder here; skipped");
            return;
        }
        let start = Duration::from_secs(30);
        let player = ClipPlayer::start(
            reqwest::Client::new(),
            "fJ9rUzIMcZQ".into(),
            start,
            (640, 640),
        )
        .unwrap();
        let begun = std::time::Instant::now();
        let mut times = Vec::new();
        while times.len() < 25 && begun.elapsed() < Duration::from_secs(20) {
            assert!(player.failed().is_none(), "{:?}", player.failed());
            // A clock running from the start point, as the window's does.
            if let Some(frame) = player.take(start + begun.elapsed()) {
                assert_eq!((frame.pixels.width(), frame.pixels.height()), (640, 360));
                let bytes = frame.pixels.as_bytes();
                let mean = bytes.iter().map(|&b| u64::from(b)).sum::<u64>() / bytes.len() as u64;
                assert!(mean > 5, "a picture, not black");
                times.push(frame.at);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        eprintln!("first picture after {:?}", begun.elapsed());
        assert!(times.len() >= 25, "only {} pictures", times.len());
        assert!(
            times[0] + SLACK >= start && times[0] < start + Duration::from_secs(5),
            "{:?}",
            times[0]
        );
        assert!(times.windows(2).all(|w| w[0] < w[1]), "in display order");
    }
}
