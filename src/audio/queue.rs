//! Play queue over a shared track list.
//!
//! The source playlist is an `Arc<[Track]>` shared with the library view;
//! the queue stores a `u32` play order over it plus a small list of tracks
//! added beyond it ("play next", radio, autoplay), so the order can be
//! edited (remove, move, clear) without copying the playlist.

use std::sync::Arc;

use crate::api::models::Track;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Repeat {
    #[default]
    Off,
    All,
    One,
}

impl Repeat {
    /// Name used in settings and messages: "off", "all", "one".
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::All => "all",
            Self::One => "one",
        }
    }

    /// Inverse of [`as_str`](Self::as_str); anything else is `Off`.
    pub fn parse(s: &str) -> Self {
        match s {
            "all" => Self::All,
            "one" => Self::One,
            _ => Self::Off,
        }
    }

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
    /// Tracks queued beyond the source list ("play next", radio, autoplay);
    /// index `source.len() + i` in `order`.
    extra: Vec<Track>,
    /// Play order: indices into `source` then `extra`.
    order: Vec<u32>,
    pos: Option<usize>,
    /// "Play next" items waiting right after `pos` (kept in FIFO order).
    queued_next: usize,
    pub shuffle: bool,
    pub repeat: Repeat,
    rng: u64,
    /// Bumped whenever the current track or what comes next changes.
    revision: u64,
}

impl Queue {
    /// Starts playing `source[start]`, keeping shuffle/repeat settings.
    pub fn set(&mut self, source: Arc<[Track]>, start: usize) {
        self.revision += 1;
        self.source = source;
        self.extra.clear();
        self.queued_next = 0;
        if start >= self.source.len() {
            self.order.clear();
            self.pos = None;
            return;
        }
        self.rebuild_order(start as u32);
    }

    fn total(&self) -> u32 {
        (self.source.len() + self.extra.len()) as u32
    }

    fn track(&self, index: u32) -> Option<&Track> {
        let i = index as usize;
        self.source
            .get(i)
            .or_else(|| self.extra.get(i.checked_sub(self.source.len())?))
    }

    /// Order with `first` at the current position when shuffling (the rest
    /// shuffled after it), else identity starting at `first`.
    fn rebuild_order(&mut self, first: u32) {
        let n = self.total();
        self.order.clear();
        self.order.reserve_exact(n as usize);
        self.order.extend(0..n);
        self.queued_next = 0;
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
        self.revision += 1;
        self.shuffle = !self.shuffle;
        if let Some(cur) = self.pos.map(|p| self.order[p]) {
            self.rebuild_order(cur);
        }
    }

    pub fn current(&self) -> Option<&Track> {
        self.track(*self.order.get(self.pos?)?)
    }

    fn index_after(&self, pos: Option<usize>) -> Option<usize> {
        let next = pos.map_or(0, |p| p + 1);
        match next {
            next if next < self.order.len() => Some(next),
            _ if self.repeat == Repeat::All && !self.order.is_empty() => Some(0),
            _ => None,
        }
    }

    pub fn peek_next(&self) -> Option<&Track> {
        let next = self.index_after(self.pos)?;
        self.track(self.order[next])
    }

    /// Moves to the next track (explicit "next" or natural end). Repeat-one
    /// is handled by the caller replaying on natural end.
    pub fn advance(&mut self) -> Option<&Track> {
        self.revision += 1;
        let next = self.index_after(self.pos)?;
        if next == 0 && self.shuffle && self.pos.is_some() {
            let first = self.order[0];
            self.rebuild_order(first); // fresh shuffle for the next round
        }
        self.queued_next = self.queued_next.saturating_sub(1);
        self.pos = Some(next);
        self.current()
    }

    pub fn back(&mut self) -> Option<&Track> {
        self.revision += 1;
        self.pos = Some(self.pos?.saturating_sub(1));
        self.current()
    }

    /// Queues `track` to play right after the current one (FIFO with other
    /// "play next" tracks).
    pub fn play_next(&mut self, track: Track) {
        self.revision += 1;
        let index = self.total();
        self.extra.push(track);
        let at = self.pos.map_or(0, |p| p + 1) + self.queued_next;
        self.order.insert(at.min(self.order.len()), index);
        self.queued_next += 1;
    }

    /// Appends tracks at the end (radio, autoplay, "add to queue").
    pub fn append(&mut self, tracks: impl IntoIterator<Item = Track>) {
        self.revision += 1;
        for track in tracks {
            let index = self.total();
            self.extra.push(track);
            self.order.push(index);
        }
    }

    /// Upcoming tracks, lazily, in play order.
    pub fn upcoming(&self) -> impl Iterator<Item = &Track> {
        let start = self.pos.map_or(0, |p| p + 1).min(self.order.len());
        self.order[start..].iter().filter_map(|&i| self.track(i))
    }

    /// Removes the `n`-th upcoming track (0 = next).
    pub fn remove_upcoming(&mut self, n: usize) {
        let at = self.pos.map_or(0, |p| p + 1) + n;
        if at < self.order.len() {
            self.revision += 1;
            self.order.remove(at);
            if n < self.queued_next {
                self.queued_next -= 1;
            }
        }
    }

    /// Moves the `from`-th upcoming track to position `to` among them.
    pub fn move_upcoming(&mut self, from: usize, to: usize) {
        let base = self.pos.map_or(0, |p| p + 1);
        let (from_at, to_at) = (base + from, base + to);
        if from_at < self.order.len() && to_at < self.order.len() && from != to {
            self.revision += 1;
            let index = self.order.remove(from_at);
            self.order.insert(to_at, index);
            // A moved "play next" item no longer keeps its FIFO slot.
            self.queued_next = self.queued_next.min(from.min(to));
        }
    }

    /// Drops everything after the current track.
    pub fn clear_upcoming(&mut self) {
        let keep = self.pos.map_or(0, |p| p + 1);
        if keep < self.order.len() {
            self.revision += 1;
            self.order.truncate(keep);
        }
        self.queued_next = 0;
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// Changes whenever [`current`](Self::current) or
    /// [`upcoming`](Self::upcoming) may have changed.
    pub fn revision(&self) -> u64 {
        self.revision
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
    fn edit_upcoming() {
        let mut q = Queue::default();
        q.set(tracks(&["a", "b", "c", "d"]), 0);
        let upcoming = |q: &Queue| {
            q.upcoming()
                .map(|t| t.video_id.to_string())
                .collect::<Vec<_>>()
        };
        q.remove_upcoming(1);
        assert_eq!(upcoming(&q), ["b", "d"]);
        q.move_upcoming(1, 0);
        assert_eq!(upcoming(&q), ["d", "b"]);
        q.append(tracks(&["r1", "r2"]).iter().cloned());
        assert_eq!(upcoming(&q), ["d", "b", "r1", "r2"]);
        q.play_next(tracks(&["x"])[0].clone());
        q.play_next(tracks(&["y"])[0].clone());
        assert_eq!(
            upcoming(&q),
            ["x", "y", "d", "b", "r1", "r2"],
            "play next keeps FIFO"
        );
        q.clear_upcoming();
        assert!(q.upcoming().next().is_none());
        assert_eq!(id(q.current()), "a");
    }

    #[test]
    fn play_next_on_an_empty_queue() {
        let mut q = Queue::default();
        q.play_next(tracks(&["x"])[0].clone());
        assert_eq!(id(q.current()), "-");
        assert_eq!(id(q.advance()), "x");
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
