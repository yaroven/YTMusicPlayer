//! Library view: what the track list shows — one of the user's playlists
//! or the last search results — the filter typed over it, and the selected
//! row. Both frontends hold one and only render it and forward input, so
//! browsing rules live here once:
//!
//! - the playlist selection survives a library reload (by id) and restarts;
//! - an edit to the shown playlist reloads it, keeping filter and selection;
//! - new search results replace the list; the playlist is one click away;
//! - "the track an action applies to" is the selected row, else the
//!   playing track;
//! - "save to playlist" offers every playlist except Liked music.
//!
//! Rows are indices into a shared `Arc<[Track]>`: no track is copied.

use std::sync::Arc;

use anyhow::Result;

use crate::{
    api::models::{LIKED_PLAYLIST_ID, Playlist, Track},
    session::{Changes, SearchResults},
    storage::Library,
};

/// Meta key of the playlist shown last (restored on start).
const LAST_PLAYLIST: &str = "playlist";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shown {
    /// Index into `playlists`.
    Playlist(usize),
    Results,
}

pub struct LibraryView {
    library: Arc<Library>,
    playlists: Vec<Playlist>,
    shown: Option<Shown>,
    /// The last search (query, results), also while a playlist is shown.
    results: Option<(String, Arc<[Track]>)>,
    tracks: Arc<[Track]>,
    filter: String,
    /// Indices into `tracks` matching `filter`.
    visible: Vec<u32>,
    /// Selected row (index into `visible`).
    selected: Option<usize>,
}

impl LibraryView {
    /// Loads the playlists and shows the one shown last (else the first).
    pub fn new(library: Arc<Library>) -> Result<Self> {
        let mut view = Self {
            library,
            playlists: Vec::new(),
            shown: None,
            results: None,
            tracks: Arc::from([]),
            filter: String::new(),
            visible: Vec::new(),
            selected: None,
        };
        let last = view.library.get_meta(LAST_PLAYLIST).ok().flatten();
        view.load_playlists(last.as_deref())?;
        Ok(view)
    }

    // --- what to show -------------------------------------------------------------

    pub fn playlists(&self) -> &[Playlist] {
        &self.playlists
    }

    /// Index of the shown playlist (`None` while search results are shown).
    pub fn selected_playlist(&self) -> Option<usize> {
        match self.shown {
            Some(Shown::Playlist(i)) => Some(i),
            _ => None,
        }
    }

    pub fn showing_results(&self) -> bool {
        self.shown == Some(Shown::Results)
    }

    /// Query of the last search, if any (shown or not).
    pub fn search_query(&self) -> Option<&str> {
        self.results.as_ref().map(|(q, _)| q.as_str())
    }

    /// Heading of the list: the playlist's title or the quoted query.
    pub fn title(&self) -> String {
        match self.shown {
            Some(Shown::Playlist(i)) => self.playlists[i].title.clone(),
            Some(Shown::Results) => format!("“{}”", self.search_query().unwrap_or_default()),
            None => String::new(),
        }
    }

    /// Where playback started from, for "Playing from".
    pub fn source_name(&self) -> String {
        match self.shown {
            Some(Shown::Results) => format!("Search: {}", self.search_query().unwrap_or_default()),
            _ => self.title(),
        }
    }

    /// Tracks in the shown list (before filtering).
    pub fn total(&self) -> usize {
        self.tracks.len()
    }

    /// Rows after filtering.
    pub fn len(&self) -> usize {
        self.visible.len()
    }

    pub fn is_empty(&self) -> bool {
        self.visible.is_empty()
    }

    /// Track at `row`, with its index in the unfiltered list.
    pub fn row(&self, row: usize) -> Option<(usize, &Track)> {
        let index = *self.visible.get(row)? as usize;
        Some((index, self.tracks.get(index)?))
    }

    /// Rows showing `video_id` (to redraw after art or a like arrived).
    pub fn rows_of(&self, video_id: &str) -> Vec<usize> {
        self.visible
            .iter()
            .enumerate()
            .filter(|(_, i)| {
                self.tracks
                    .get(**i as usize)
                    .is_some_and(|t| &*t.video_id == video_id)
            })
            .map(|(row, _)| row)
            .collect()
    }

    pub fn filter(&self) -> &str {
        &self.filter
    }

    pub fn selected_row(&self) -> Option<usize> {
        self.selected
    }

    pub fn selected_track(&self) -> Option<&Track> {
        self.row(self.selected?).map(|(_, t)| t)
    }

    /// The track an action (like, play next, save to playlist) applies to:
    /// the selected row, else the playing track.
    pub fn target(&self, playing: Option<&Track>) -> Option<Track> {
        self.selected_track().or(playing).cloned()
    }

    /// The queue for playing from `row` (`None`: the first row): the whole
    /// shown list and the index to start at.
    pub fn play_from(&self, row: Option<usize>) -> Option<(Arc<[Track]>, usize)> {
        let (index, _) = self.row(row.unwrap_or(0))?;
        Some((self.tracks.clone(), index))
    }

    /// The whole shown list (for shuffle-play), unless empty.
    pub fn all_tracks(&self) -> Option<Arc<[Track]>> {
        (!self.tracks.is_empty()).then(|| self.tracks.clone())
    }

    /// Playlists a track can be saved to: every one except Liked music.
    pub fn add_choices(&self) -> impl Iterator<Item = &Playlist> {
        self.playlists.iter().filter(|p| p.id != LIKED_PLAYLIST_ID)
    }

    pub fn add_choice(&self, i: usize) -> Option<&Playlist> {
        self.add_choices().nth(i)
    }

    /// Whether `video_id` is in Liked music (an indexed lookup).
    pub fn is_liked(&self, video_id: &str) -> bool {
        self.library
            .contains(LIKED_PLAYLIST_ID, video_id)
            .unwrap_or(false)
    }

    // --- input ---------------------------------------------------------------------

    /// Shows playlist `index` (unfiltered, nothing selected).
    pub fn select_playlist(&mut self, index: Option<usize>) -> Result<()> {
        let index = index.filter(|&i| i < self.playlists.len());
        self.shown = index.map(Shown::Playlist);
        self.filter.clear();
        self.selected = None;
        self.tracks = match index {
            Some(i) => self.library.tracks(&self.playlists[i].id)?,
            None => Arc::from([]),
        };
        self.refilter();
        Ok(())
    }

    /// Shows the last search results again; false if there are none.
    pub fn show_results(&mut self) -> bool {
        let Some((_, tracks)) = &self.results else {
            return false;
        };
        self.tracks = tracks.clone();
        self.shown = Some(Shown::Results);
        self.filter.clear();
        self.selected = None;
        self.refilter();
        true
    }

    pub fn set_filter(&mut self, filter: &str) {
        if self.filter != filter {
            self.filter = filter.to_owned();
            self.selected = None;
            self.refilter();
        }
    }

    pub fn select_row(&mut self, row: Option<usize>) {
        self.selected = row.filter(|&r| r < self.visible.len());
    }

    /// Selects the first row when nothing is selected (keyboard UIs).
    pub fn ensure_selection(&mut self) {
        if self.selected.is_none() && !self.visible.is_empty() {
            self.selected = Some(0);
        }
    }

    /// Moves the selection by `delta` rows, clamped to the list.
    pub fn move_selection(&mut self, delta: i64) {
        if self.visible.is_empty() {
            return;
        }
        let last = self.visible.len() as i64 - 1;
        let from = self.selected.map_or(0, |r| r as i64);
        self.selected = Some(from.saturating_add(delta).clamp(0, last) as usize);
    }

    /// Reacts to what a Session changed; true when the list's rows were
    /// replaced (so a UI must rebuild them, not just redraw some).
    pub fn apply(&mut self, changes: &Changes, search: Option<&SearchResults>) -> Result<bool> {
        let mut replaced = false;
        if changes.library {
            self.load_playlists(None)?;
            replaced = true;
        } else if let Some(id) = &changes.playlist {
            self.playlists = self.library.playlists()?;
            if self.shown_playlist_id() == Some(id.as_str()) {
                self.reload_keeping_place()?;
                replaced = true;
            } else if let Some(Shown::Playlist(i)) = self.shown {
                // Counts changed; keep pointing at the same playlist.
                if i >= self.playlists.len() {
                    self.select_playlist(Some(0))?;
                    replaced = true;
                }
            }
        }
        if changes.search
            && let Some(results) = search
        {
            self.results = Some((results.query.clone(), results.tracks.clone()));
            self.show_results();
            replaced = true;
        }
        Ok(replaced)
    }

    /// Remembers the shown playlist for the next start.
    pub fn save(&self) {
        if let Some(id) = self.shown_playlist_id()
            && let Err(err) = self.library.set_meta(LAST_PLAYLIST, id)
        {
            tracing::warn!(%err, "saving the shown playlist");
        }
    }

    // --- internals -----------------------------------------------------------------

    fn shown_playlist_id(&self) -> Option<&str> {
        self.selected_playlist()
            .and_then(|i| self.playlists.get(i))
            .map(|p| p.id.as_str())
    }

    /// Re-reads the playlists, showing `prefer_id`, else the one shown,
    /// else the first.
    fn load_playlists(&mut self, prefer_id: Option<&str>) -> Result<()> {
        let keep = prefer_id
            .map(str::to_owned)
            .or_else(|| self.shown_playlist_id().map(str::to_owned));
        self.playlists = self.library.playlists()?;
        let index = keep
            .and_then(|id| self.playlists.iter().position(|p| p.id == id))
            .or((!self.playlists.is_empty()).then_some(0));
        self.select_playlist(index)
    }

    /// The shown playlist's tracks changed: reload, keep filter and row.
    fn reload_keeping_place(&mut self) -> Result<()> {
        let Some(id) = self.shown_playlist_id().map(str::to_owned) else {
            return Ok(());
        };
        self.tracks = self.library.tracks(&id)?;
        let selected = self.selected;
        self.refilter();
        self.select_row(selected);
        Ok(())
    }

    fn refilter(&mut self) {
        filter_indices(&self.tracks, &self.filter, &mut self.visible);
    }
}

/// Fills `out` with indices of `tracks` whose title or artist contains
/// `filter`, case-insensitively (all of them when `filter` is empty).
fn filter_indices(tracks: &[Track], filter: &str, out: &mut Vec<u32>) {
    let needle: Vec<char> = filter.chars().flat_map(char::to_lowercase).collect();
    out.clear();
    out.extend(
        tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                needle.is_empty()
                    || contains_ci(&t.title, &needle)
                    || contains_ci(&t.artist, &needle)
            })
            .map(|(i, _)| i as u32),
    );
    out.shrink_to_fit();
}

/// Case-insensitive substring test without allocating (`needle` lowercased).
fn contains_ci(haystack: &str, needle: &[char]) -> bool {
    needle.is_empty()
        || haystack.char_indices().any(|(start, _)| {
            let mut chars = haystack[start..].chars().flat_map(char::to_lowercase);
            needle.iter().all(|n| chars.next() == Some(*n))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(id: &str, title: &str) -> Track {
        Track {
            video_id: id.into(),
            title: title.into(),
            artist: "Artist".into(),
            duration_secs: None,
        }
    }

    fn playlist(id: &str, title: &str, n: u32) -> Playlist {
        Playlist {
            id: id.into(),
            title: title.into(),
            item_count: n,
            etag: None,
        }
    }

    /// Liked music (2 tracks) and "Road trip" (3 tracks).
    fn library() -> Arc<Library> {
        let lib = Library::open_in_memory().unwrap();
        let liked: Arc<[Track]> =
            vec![track("aaaaaaaaaaa", "Alpha"), track("bbbbbbbbbbb", "Beta")].into();
        let trip: Arc<[Track]> = vec![
            track("ccccccccccc", "Gamma"),
            track("ddddddddddd", "Delta"),
            track("eeeeeeeeeee", "Echo"),
        ]
        .into();
        lib.replace_library(&[
            (playlist(LIKED_PLAYLIST_ID, "Liked music", 2), liked),
            (playlist("PLtrip", "Road trip", 3), trip),
        ])
        .unwrap();
        Arc::new(lib)
    }

    fn titles(v: &LibraryView) -> Vec<String> {
        (0..v.len())
            .map(|r| v.row(r).unwrap().1.title.to_string())
            .collect()
    }

    #[test]
    fn case_insensitive_contains() {
        let needle: Vec<char> = "QUEEN".chars().flat_map(char::to_lowercase).collect();
        assert!(contains_ci("We are Queen fans", &needle));
        assert!(!contains_ci("Que", &needle));
        let cyr: Vec<char> = "ОКЕАН".chars().flat_map(char::to_lowercase).collect();
        assert!(contains_ci("Океан Ельзи", &cyr));
    }

    #[test]
    fn restores_the_last_shown_playlist() {
        let lib = library();
        let mut v = LibraryView::new(lib.clone()).unwrap();
        assert_eq!(v.title(), "Liked music", "first start: the first playlist");
        v.select_playlist(Some(1)).unwrap();
        v.save();
        let v = LibraryView::new(lib).unwrap();
        assert_eq!(v.title(), "Road trip");
        assert_eq!(titles(&v), ["Gamma", "Delta", "Echo"]);
    }

    #[test]
    fn filter_and_play_from_a_filtered_row() {
        let mut v = LibraryView::new(library()).unwrap();
        v.select_playlist(Some(1)).unwrap();
        v.set_filter("ec");
        assert_eq!(titles(&v), ["Echo"]);
        let (queue, start) = v.play_from(Some(0)).unwrap();
        assert_eq!(queue.len(), 3, "the queue is the whole playlist");
        assert_eq!(&*queue[start].title, "Echo");
    }

    #[test]
    fn target_is_the_selected_row_else_the_playing_track() {
        let mut v = LibraryView::new(library()).unwrap();
        let playing = track("zzzzzzzzzzz", "Playing");
        assert_eq!(&*v.target(Some(&playing)).unwrap().title, "Playing");
        v.select_row(Some(1));
        assert_eq!(&*v.target(Some(&playing)).unwrap().title, "Beta");
        v.select_row(Some(9));
        assert_eq!(v.selected_row(), None, "out of range selects nothing");
    }

    #[test]
    fn edit_of_the_shown_playlist_keeps_filter_and_row() {
        let lib = library();
        let mut v = LibraryView::new(lib.clone()).unwrap();
        v.select_playlist(Some(1)).unwrap();
        v.set_filter("e");
        v.select_row(Some(1));
        lib.add_track("PLtrip", &track("fffffffffff", "Epsilon"), false)
            .unwrap();
        let changes = Changes {
            playlist: Some("PLtrip".into()),
            ..Default::default()
        };
        assert!(v.apply(&changes, None).unwrap());
        assert_eq!(v.filter(), "e");
        assert_eq!(titles(&v), ["Delta", "Echo", "Epsilon"]);
        assert_eq!(v.selected_row(), Some(1));
        assert_eq!(v.playlists()[1].item_count, 4);
    }

    #[test]
    fn edit_of_another_playlist_only_updates_counts() {
        let lib = library();
        let mut v = LibraryView::new(lib.clone()).unwrap();
        v.select_playlist(Some(1)).unwrap();
        v.select_row(Some(2));
        lib.add_track(LIKED_PLAYLIST_ID, &track("fffffffffff", "Kappa"), true)
            .unwrap();
        let changes = Changes {
            playlist: Some(LIKED_PLAYLIST_ID.into()),
            ..Default::default()
        };
        assert!(!v.apply(&changes, None).unwrap());
        assert_eq!(v.selected_row(), Some(2));
        assert_eq!(v.playlists()[0].item_count, 3);
    }

    #[test]
    fn search_results_replace_the_list_until_a_playlist_is_picked() {
        let mut v = LibraryView::new(library()).unwrap();
        let results = SearchResults {
            query: "daft punk".into(),
            tracks: vec![track("ggggggggggg", "Get Lucky")].into(),
        };
        let changes = Changes {
            search: true,
            ..Default::default()
        };
        assert!(v.apply(&changes, Some(&results)).unwrap());
        assert!(v.showing_results());
        assert_eq!(v.selected_playlist(), None);
        assert_eq!(v.title(), "“daft punk”");
        assert_eq!(v.source_name(), "Search: daft punk");
        assert_eq!(titles(&v), ["Get Lucky"]);

        v.select_playlist(Some(0)).unwrap();
        assert!(!v.showing_results());
        assert_eq!(
            v.search_query(),
            Some("daft punk"),
            "results stay reachable"
        );
        assert!(v.show_results());
        assert_eq!(titles(&v), ["Get Lucky"]);
    }

    #[test]
    fn sync_keeps_the_shown_playlist_by_id() {
        let lib = library();
        let mut v = LibraryView::new(lib.clone()).unwrap();
        v.select_playlist(Some(1)).unwrap();
        let changes = Changes {
            library: true,
            ..Default::default()
        };
        v.apply(&changes, None).unwrap();
        assert_eq!(v.title(), "Road trip");
    }

    #[test]
    fn liked_music_is_never_an_add_choice() {
        let v = LibraryView::new(library()).unwrap();
        let choices: Vec<_> = v.add_choices().map(|p| p.title.as_str()).collect();
        assert_eq!(choices, ["Road trip"]);
        assert_eq!(v.add_choice(0).unwrap().id, "PLtrip");
        assert!(v.is_liked("aaaaaaaaaaa"));
        assert!(!v.is_liked("ccccccccccc"));
    }

    #[test]
    fn keyboard_selection_starts_at_the_top_and_clamps() {
        let mut v = LibraryView::new(library()).unwrap();
        assert_eq!(v.selected_row(), None);
        v.ensure_selection();
        assert_eq!(v.selected_row(), Some(0));
        v.move_selection(10);
        assert_eq!(v.selected_row(), Some(1));
        v.move_selection(-10);
        assert_eq!(v.selected_row(), Some(0));
    }
}
