//! Play queue over a shared track list.
//!
//! The source playlist is an `Arc<[Track]>` shared with the library view;
//! the queue only stores a `u32` play order (identity or shuffled), plus a
//! small "play next" list for tracks the user queued explicitly.

use std::{collections::VecDeque, sync::Arc};

use crate::api::models::Track;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Repeat {
    #[default]
    Off,
    All,
    One,
}

impl Repeat {
    pub fn cycle(self) -> Self {
        match self {
            Self::Off => Self::All,
            Self::All => Self::One,
            Self::One => Self::Off,
        }
    }
}

#[derive(Debug, Default)]
pub struct Queue {
    source: Arc<[Track]>,
    order: Vec<u32>,
    pos: Option<usize>,
    next_up: VecDeque<Track>,
    /// Set while playing an item taken from `next_up`.
    detour: Option<Track>,
    pub shuffle: bool,
    pub repeat: Repeat,
    rng: u64,
}

impl Queue {
    /// Starts playing `source[start]`, keeping shuffle/repeat settings.
    pub fn set(&mut self, source: Arc<[Track]>, start: usize) {
        self.source = source;
        self.detour = None;
        if start >= self.source.len() {
            self.order.clear();
            self.pos = None;
            return;
        }
        self.rebuild_order(start as u32);
    }

    /// Order with `first` at position 0 when shuffling, else identity.
    fn rebuild_order(&mut self, first: u32) {
        let n = self.source.len() as u32;
        self.order.clear();
        self.order.reserve_exact(n as usize);
        self.order.extend(0..n);
        if self.shuffle {
            self.order.swap(0, first as usize);
            // Fisher–Yates over the rest.
            for i in (2..n as usize).rev() {
                let j = 1 + (self.next_random() % i as u64) as usize;
                self.order.swap(i, j);
            }
            self.pos = Some(0);
        } else {
            self.pos = Some(first as usize);
        }
        self.order.shrink_to_fit();
    }

    fn next_random(&mut self) -> u64 {
        if self.rng == 0 {
            self.rng = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0x9E37_79B9_7F4A_7C15, |d| d.as_nanos() as u64)
                | 1;
        }
        // xorshift64*
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn toggle_shuffle(&mut self) {
        self.shuffle = !self.shuffle;
        if let Some(cur) = self.pos.map(|p| self.order[p]) {
            self.rebuild_order(cur);
        }
    }

    pub fn current(&self) -> Option<&Track> {
        if let Some(t) = &self.detour {
            return Some(t);
        }
        self.source.get(self.order[self.pos?] as usize)
    }

    fn index_after(&self, pos: usize) -> Option<usize> {
        match pos + 1 {
            next if next < self.order.len() => Some(next),
            _ if self.repeat == Repeat::All && !self.order.is_empty() => Some(0),
            _ => None,
        }
    }

    pub fn peek_next(&self) -> Option<&Track> {
        if let Some(t) = self.next_up.front() {
            return Some(t);
        }
        let next = self.index_after(self.pos?)?;
        self.source.get(self.order[next] as usize)
    }

    /// Moves to the next track (explicit "next" or natural end). Repeat-one
    /// is handled by the caller replaying on natural end.
    pub fn advance(&mut self) -> Option<&Track> {
        if let Some(t) = self.next_up.pop_front() {
            self.detour = Some(t);
            return self.detour.as_ref();
        }
        self.detour = None;
        let next = self.index_after(self.pos?)?;
        if next == 0 && self.shuffle {
            let first = self.order[0];
            self.rebuild_order(first); // fresh shuffle for the next round
        }
        self.pos = Some(next);
        self.current()
    }

    pub fn back(&mut self) -> Option<&Track> {
        if self.detour.take().is_none() {
            self.pos = Some(self.pos?.saturating_sub(1));
        }
        self.current()
    }

    /// Queues `track` to play right after the current one (FIFO).
    pub fn play_next(&mut self, track: Track) {
        self.next_up.push_back(track);
    }

    /// Upcoming tracks, lazily: "play next" items first, then the order.
    pub fn upcoming(&self) -> impl Iterator<Item = &Track> {
        let rest = self
            .pos
            .map_or(0..0, |p| (p + 1).min(self.order.len())..self.order.len());
        self.next_up
            .iter()
            .chain(rest.map(|i| &self.source[self.order[i] as usize]))
    }

    pub fn len(&self) -> usize {
        self.order.len() + self.next_up.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Index of the current track in the play order.
    pub fn position(&self) -> Option<usize> {
        self.pos
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracks(ids: &[&str]) -> Arc<[Track]> {
        ids.iter()
            .map(|id| Track {
                video_id: (*id).into(),
                title: (*id).into(),
                artist: "".into(),
                duration_secs: None,
            })
            .collect()
    }

    fn id(t: Option<&Track>) -> &str {
        t.map_or("-", |t| &t.video_id)
    }

    #[test]
    fn navigation_and_repeat() {
        let mut q = Queue::default();
        q.set(tracks(&["a", "b"]), 0);
        assert_eq!(id(q.peek_next()), "b");
        assert_eq!(id(q.advance()), "b");
        assert_eq!(id(q.advance()), "-", "stops at the end");
        q.repeat = Repeat::All;
        assert_eq!(id(q.advance()), "a", "wraps with repeat all");
        assert_eq!(id(q.back()), "a", "clamps at the start");
    }

    #[test]
    fn play_next_detour() {
        let mut q = Queue::default();
        q.set(tracks(&["a", "b", "c"]), 0);
        q.play_next(tracks(&["x"])[0].clone());
        assert_eq!(
            q.upcoming().map(|t| &*t.video_id).collect::<Vec<_>>(),
            ["x", "b", "c"]
        );
        assert_eq!(id(q.advance()), "x");
        assert_eq!(
            id(q.advance()),
            "b",
            "returns to the order after the detour"
        );
    }

    #[test]
    fn shuffle_keeps_current_and_covers_all() {
        let mut q = Queue {
            shuffle: true,
            ..Default::default()
        };
        q.set(tracks(&["a", "b", "c", "d", "e"]), 2);
        assert_eq!(id(q.current()), "c");
        let mut seen: Vec<_> = std::iter::once("c".to_owned())
            .chain(q.upcoming().map(|t| t.video_id.to_string()))
            .collect();
        seen.sort();
        assert_eq!(seen, ["a", "b", "c", "d", "e"]);
        q.toggle_shuffle();
        assert_eq!(id(q.current()), "c", "unshuffle keeps the current track");
        assert_eq!(id(q.peek_next()), "d");
    }
}
