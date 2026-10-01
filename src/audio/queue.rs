//! Play queue: a snapshot of the tracks the user started playing from.

use crate::api::models::Track;

#[derive(Debug, Default)]
pub struct Queue {
    tracks: Vec<Track>,
    current: Option<usize>,
}

impl Queue {
    pub fn set(&mut self, tracks: Vec<Track>, start: usize) {
        self.current = (start < tracks.len()).then_some(start);
        self.tracks = tracks;
    }

    pub fn current(&self) -> Option<&Track> {
        self.tracks.get(self.current?)
    }

    pub fn peek_next(&self) -> Option<&Track> {
        self.tracks.get(self.current? + 1)
    }

    pub fn advance(&mut self) -> Option<&Track> {
        let next = self.current? + 1;
        if next >= self.tracks.len() {
            return None;
        }
        self.current = Some(next);
        self.current()
    }

    pub fn back(&mut self) -> Option<&Track> {
        self.current = Some(self.current?.saturating_sub(1));
        self.current()
    }

    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    pub fn position(&self) -> Option<usize> {
        self.current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(id: &str) -> Track {
        Track {
            video_id: id.into(),
            title: id.into(),
            artist: String::new(),
            duration_secs: None,
        }
    }

    #[test]
    fn navigation() {
        let mut q = Queue::default();
        q.set(vec![t("a"), t("b")], 0);
        assert_eq!(q.peek_next().unwrap().video_id, "b");
        assert_eq!(q.advance().unwrap().video_id, "b");
        assert!(q.advance().is_none(), "stops at the end");
        assert_eq!(q.current().unwrap().video_id, "b");
        assert_eq!(q.back().unwrap().video_id, "a");
        assert_eq!(q.back().unwrap().video_id, "a", "clamps at the start");
    }
}
