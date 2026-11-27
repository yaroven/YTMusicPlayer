//! The clip over the big cover, played by us (`crate::clip`): a timer shows
//! the picture due at the song's position plus the clip's intro offset, so
//! the studio song keeps playing and the window's own controls drive both.
//! Pause, seek and the next song come from the snapshots, like the lyrics'
//! clock.

use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

use anyhow::Result;
use slint::{ComponentHandle, Image};

use super::MainWindow;
use crate::{clip::ClipPlayer, session::Clip};

/// How often the timer looks for the next picture (it redraws only when one
/// is due: 25–30 times a second).
const TICK: Duration = Duration::from_millis(15);
/// A snapshot this far from where the clock had got to is a seek.
const JUMP: Duration = Duration::from_millis(1500);

struct Clock {
    /// Song position at the last snapshot, and when it came (`None`: paused).
    at: Duration,
    since: Option<Instant>,
    /// How far the clip runs ahead of the song (its intro).
    offset: Duration,
}

impl Clock {
    fn song(&self) -> Duration {
        self.since.map_or(self.at, |t| self.at + t.elapsed())
    }

    fn clip(&self) -> Duration {
        self.song() + self.offset
    }
}

pub struct ClipView {
    player: Rc<ClipPlayer>,
    clock: Rc<RefCell<Clock>>,
    _timer: slint::Timer,
}

impl ClipView {
    /// Starts the song's clip (its id and intro offset) where the song is.
    /// `failed` runs (once) when the clip can't play: the caller turns the
    /// video off and falls back.
    pub fn open(
        ui: &MainWindow,
        http: reqwest::Client,
        (clip, offset): &Clip,
        position: Duration,
        playing: bool,
        rect: (f32, f32, f32, f32),
        failed: impl Fn(&MainWindow) + 'static,
    ) -> Result<Self> {
        let clock = Rc::new(RefCell::new(Clock {
            at: position,
            since: playing.then(Instant::now),
            offset: Duration::from_secs_f64(offset.max(0.0)),
        }));
        let size = physical(rect, ui.window().scale_factor());
        let player = Rc::new(ClipPlayer::start(
            http,
            clip.to_string(),
            clock.borrow().clip(),
            size,
        )?);
        let timer = slint::Timer::default();
        let (weak, shown, ticking) = (ui.as_weak(), player.clone(), clock.clone());
        timer.start(slint::TimerMode::Repeated, TICK, move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            if let Some(frame) = shown.take(ticking.borrow().clip()) {
                ui.set_video_frame(Image::from_rgb8(frame.pixels));
            } else if shown.failed().is_some() && ui.get_video_on() {
                ui.set_video_frame(Image::default());
                failed(&ui);
            }
        });
        Ok(Self {
            player,
            clock,
            _timer: timer,
        })
    }

    /// The song's position at a snapshot; a jump is a seek.
    pub fn follow(&self, position: Duration, playing: bool) {
        let mut clock = self.clock.borrow_mut();
        let jumped = clock.song().abs_diff(position) > JUMP;
        clock.at = position;
        clock.since = playing.then(Instant::now);
        if jumped {
            self.player.seek(clock.clip());
        }
    }

    /// The video area moved or changed size.
    pub fn set_rect(&self, rect: (f32, f32, f32, f32), scale: f32) {
        self.player.set_size(physical(rect, scale));
    }
}

/// The area's size in physical pixels (what the pictures are scaled to).
fn physical(rect: (f32, f32, f32, f32), scale: f32) -> (u32, u32) {
    let px = |v: f32| (v * scale).round().max(2.0) as u32;
    (px(rect.2), px(rect.3))
}
