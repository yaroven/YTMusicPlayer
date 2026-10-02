//! Library view: what the main list shows — one of the user's playlists,
//! search results, an album / artist / playlist page, the home feed,
//! history, downloads, saved albums or followed artists — the filter typed
//! over its tracks, the selected row, and the way back. Both frontends hold
//! one and only render it and forward input, so browsing rules live here
//! once:
//!
//! - the playlist selection survives a library reload (by id) and restarts;
//! - an edit to the shown list reloads it, keeping filter and selection;
//! - opening something remembers what was shown (Back returns to it);
//! - "the track an action applies to" is the selected row, else the
//!   playing track;
//! - "save to playlist" offers every playlist except Liked music.
//!
//! Rows are indices into a shared `Arc<[Track]>`: no track is copied.

use std::sync::Arc;

use anyhow::Result;

use crate::{
    api::models::{LIKED_PLAYLIST_ID, Playlist, Track},
    catalog::{Item, ItemKind, Page, SearchKind, Shelf},
    session::{Changes, SearchResults},
    storage::Library,
};

/// Meta key of the playlist shown last (restored on start).
const LAST_PLAYLIST: &str = "playlist";
/// How many screens Back remembers.
const BACK_DEPTH: usize = 12;

/// Where the shown list comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// One of the user's playlists (by id).
    Playlist(String),
    Search,
    /// An album, artist or playlist page (by browse id).
    Page(String),
    Home,
    History,
    Downloads,
    /// Saved albums or followed artists.
    Saved(ItemKind),
    /// Nothing (no playlists yet).
    Empty,
}

/// One screen of the view.
#[derive(Clone)]
struct Shown {
    source: Source,
    title: String,
    subtitle: String,
    thumbnail: Option<String>,
    /// Album / artist / playlist pages: what it is.
    kind: Option<ItemKind>,
    /// Artist pages: the channel to subscribe to.
    channel_id: Option<String>,
    tracks: Arc<[Track]>,
    items: Arc<[Item]>,
    shelves: Arc<[Shelf]>,
}

impl Shown {
    fn empty() -> Self {
        Self {
            source: Source::Empty,
            title: String::new(),
            subtitle: String::new(),
            thumbnail: None,
            kind: None,
            channel_id: None,
            tracks: Arc::from([]),
            items: Arc::from([]),
            shelves: Arc::from([]),
        }
    }
}

/// What a Session holds that the view may show.
#[derive(Default, Clone, Copy)]
pub struct SessionData<'a> {
    pub search: Option<&'a SearchResults>,
    pub page: Option<&'a Page>,
    pub home: Option<&'a [Shelf]>,
}

pub struct LibraryView {
    library: Arc<Library>,
    playlists: Vec<Playlist>,
    shown: Shown,
    back: Vec<Shown>,
    /// The last search, also while something else is shown.
    results: Option<SearchResults>,
    filter: String,
    /// Indices into the shown tracks matching `filter`.
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
            shown: Shown::empty(),
            back: Vec::new(),
            results: None,
            filter: String::new(),
            visible: Vec::new(),
            selected: None,
        };
        let last = view.library.get_meta(LAST_PLAYLIST).ok().flatten();
        view.load_playlists(last.as_deref())?;
        Ok(view)
    }

    // --- what is shown ---------------------------------------------------------------

    pub fn playlists(&self) -> &[Playlist] {
        &self.playlists
    }

    pub fn source(&self) -> &Source {
        &self.shown.source
    }

    /// Index of the shown playlist (`None` when something else is shown).
    pub fn selected_playlist(&self) -> Option<usize> {
        match &self.shown.source {
            Source::Playlist(id) => self.playlists.iter().position(|p| &p.id == id),
            _ => None,
        }
    }

    /// The shown user playlist.
    pub fn shown_playlist(&self) -> Option<&Playlist> {
        self.selected_playlist().map(|i| &self.playlists[i])
    }

    pub fn showing_results(&self) -> bool {
        self.shown.source == Source::Search
    }

    /// Query of the last search, if any (shown or not).
    pub fn search_query(&self) -> Option<&str> {
        self.results.as_ref().map(|r| r.query.as_str())
    }

    pub fn search_kind(&self) -> Option<SearchKind> {
        self.results.as_ref().map(|r| r.kind)
    }

    /// Heading of the list.
    pub fn title(&self) -> String {
        match &self.shown.source {
            Source::Search => format!("“{}”", self.search_query().unwrap_or_default()),
            _ => self.shown.title.clone(),
        }
    }

    pub fn subtitle(&self) -> &str {
        &self.shown.subtitle
    }

    pub fn thumbnail(&self) -> Option<&str> {
        self.shown.thumbnail.as_deref()
    }

    /// For pages: album, artist or playlist.
    pub fn page_kind(&self) -> Option<ItemKind> {
        self.shown.kind
    }

    /// The shown page as a card (to save the album / follow the artist),
    /// with the artist's channel id.
    pub fn page_item(&self) -> Option<(Item, Option<String>)> {
        let Source::Page(id) = &self.shown.source else {
            return None;
        };
        let item = Item {
            kind: self.shown.kind?,
            id: id.as_str().into(),
            title: self.shown.title.as_str().into(),
            subtitle: self.shown.subtitle.as_str().into(),
            thumbnail: self.shown.thumbnail.as_deref().map(Into::into),
            track: None,
        };
        Some((item, self.shown.channel_id.clone()))
    }

    /// Where playback started from, for "Playing from".
    pub fn source_name(&self) -> String {
        match &self.shown.source {
            Source::Search => format!("Search: {}", self.search_query().unwrap_or_default()),
            _ => self.shown.title.clone(),
        }
    }

    /// Tracks in the shown list (before filtering).
    pub fn total(&self) -> usize {
        self.shown.tracks.len()
    }

    /// Track rows after filtering.
    pub fn len(&self) -> usize {
        self.visible.len()
    }

    pub fn is_empty(&self) -> bool {
        self.visible.is_empty()
    }

    /// Track at `row`, with its index in the unfiltered list.
    pub fn row(&self, row: usize) -> Option<(usize, &Track)> {
        let index = *self.visible.get(row)? as usize;
        Some((index, self.shown.tracks.get(index)?))
    }

    /// Rows showing `video_id` (to redraw after art or a like arrived).
    pub fn rows_of(&self, video_id: &str) -> Vec<usize> {
        self.visible
            .iter()
            .enumerate()
            .filter(|(_, i)| {
                self.shown
                    .tracks
                    .get(**i as usize)
                    .is_some_and(|t| &*t.video_id == video_id)
            })
            .map(|(row, _)| row)
            .collect()
    }

    /// Cards (albums, artists, playlists) the shown list consists of.
    pub fn items(&self) -> &Arc<[Item]> {
        &self.shown.items
    }

    /// Titled card rows under the tracks (artist pages, home).
    pub fn shelves(&self) -> &Arc<[Shelf]> {
        &self.shown.shelves
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
        Some((self.shown.tracks.clone(), index))
    }

    /// The whole shown list (for shuffle-play), unless empty.
    pub fn all_tracks(&self) -> Option<Arc<[Track]>> {
        (!self.shown.tracks.is_empty()).then(|| self.shown.tracks.clone())
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

    pub fn is_downloaded(&self, video_id: &str) -> bool {
        self.library
            .download_path(video_id)
            .ok()
            .flatten()
            .is_some()
    }

    pub fn is_saved(&self, id: &str) -> bool {
        self.library.is_saved(id).unwrap_or(false)
    }

    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    // --- navigation ----------------------------------------------------------------

    /// Shows playlist `index` (unfiltered, nothing selected).
    pub fn select_playlist(&mut self, index: Option<usize>) -> Result<()> {
        let Some(p) = index.and_then(|i| self.playlists.get(i)).cloned() else {
            self.replace(Shown::empty());
            return Ok(());
        };
        let tracks = self.library.tracks(&p.id)?;
        self.navigate(Shown {
            source: Source::Playlist(p.id.clone()),
            subtitle: format!("{} songs", tracks.len()),
            title: p.title,
            tracks,
            ..Shown::empty()
        });
        Ok(())
    }

    /// Shows the last search results again; false if there are none.
    pub fn show_results(&mut self) -> bool {
        let Some(r) = &self.results else {
            return false;
        };
        let shown = Shown {
            source: Source::Search,
            title: r.query.clone(),
            subtitle: format!(
                "{} {}",
                r.tracks.len() + r.items.len(),
                r.kind.label().to_lowercase()
            ),
            tracks: r.tracks.clone(),
            items: r.items.clone(),
            ..Shown::empty()
        };
        if self.shown.source == Source::Search {
            self.replace(shown); // another category: not a step back
        } else {
            self.navigate(shown);
        }
        true
    }

    /// Shows a loaded album / artist / playlist page.
    pub fn show_page(&mut self, page: &Page) {
        let shown = Shown {
            source: Source::Page(page.id.clone()),
            title: page.title.clone(),
            subtitle: page.subtitle.clone(),
            thumbnail: page.thumbnail.clone(),
            kind: Some(page.kind),
            channel_id: page.channel_id.clone(),
            tracks: page.tracks.clone().into(),
            shelves: page.shelves.clone().into(),
            ..Shown::empty()
        };
        self.navigate(shown);
    }

    /// Home: recently played, saved albums, then YouTube Music's shelves.
    pub fn show_home(&mut self, home: Option<&[Shelf]>) -> Result<()> {
        let recent: Arc<[Track]> = self.library.history()?.iter().take(20).cloned().collect();
        let mut shelves: Vec<Shelf> = Vec::new();
        let saved = self.library.saved(ItemKind::Album)?;
        if !saved.is_empty() {
            shelves.push(Shelf {
                title: "Your albums".into(),
                items: saved,
            });
        }
        shelves.extend(home.unwrap_or_default().iter().cloned());
        let shown = Shown {
            source: Source::Home,
            title: "Home".into(),
            subtitle: if recent.is_empty() {
                String::new()
            } else {
                "Listen again".into()
            },
            tracks: recent,
            shelves: shelves.into(),
            ..Shown::empty()
        };
        if self.shown.source == Source::Home {
            self.replace(shown);
        } else {
            self.navigate(shown);
        }
        Ok(())
    }

    pub fn show_history(&mut self) -> Result<()> {
        let tracks = self.library.history()?;
        self.navigate(Shown {
            source: Source::History,
            title: "History".into(),
            subtitle: format!("{} songs", tracks.len()),
            tracks,
            ..Shown::empty()
        });
        Ok(())
    }

    pub fn show_downloads(&mut self) -> Result<()> {
        let downloads = self.library.downloads()?;
        let bytes: u64 = downloads.iter().map(|d| d.bytes).sum();
        let tracks: Arc<[Track]> = downloads.into_iter().map(|d| d.track).collect();
        self.navigate(Shown {
            source: Source::Downloads,
            title: "Downloads".into(),
            subtitle: format!("{} songs · {:.0} MB", tracks.len(), bytes as f64 / 1e6),
            tracks,
            ..Shown::empty()
        });
        Ok(())
    }

    /// Saved albums (`Album`) or followed artists (`Artist`).
    pub fn show_saved(&mut self, kind: ItemKind) -> Result<()> {
        let items: Arc<[Item]> = self.library.saved(kind)?.into();
        let title = match kind {
            ItemKind::Artist => "Artists",
            _ => "Albums",
        };
        self.navigate(Shown {
            source: Source::Saved(kind),
            title: title.into(),
            subtitle: format!("{} {}", items.len(), title.to_lowercase()),
            items,
            ..Shown::empty()
        });
        Ok(())
    }

    /// Returns to what was shown before; false if there's nothing.
    pub fn back(&mut self) -> bool {
        let Some(previous) = self.back.pop() else {
            return false;
        };
        self.shown = previous;
        self.reset_rows();
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

    /// Reacts to what a Session changed; true when the shown rows were
    /// replaced (so a UI must rebuild them, not just redraw some).
    pub fn apply(&mut self, changes: &Changes, data: SessionData<'_>) -> Result<bool> {
        let mut replaced = false;
        if changes.library {
            self.reload_playlists()?;
            replaced = true;
        } else if let Some(id) = &changes.playlist {
            self.playlists = self.library.playlists()?;
            if self.shown.source == Source::Playlist(id.clone()) {
                self.reload_keeping_place()?;
                replaced = true;
            }
        }
        if changes.search
            && let Some(results) = data.search
        {
            self.results = Some(results.clone());
            self.show_results();
            replaced = true;
        }
        if changes.page
            && let Some(page) = data.page
        {
            self.show_page(page);
            replaced = true;
        }
        if changes.home && self.shown.source == Source::Home {
            self.show_home(data.home)?;
            replaced = true;
        }
        let refresh = match &self.shown.source {
            Source::Saved(_) => changes.saved,
            Source::Downloads => changes.downloads,
            _ => false,
        };
        if refresh {
            self.reload_keeping_place()?;
            replaced = true;
        }
        Ok(replaced)
    }

    /// Remembers the shown playlist for the next start.
    pub fn save(&self) {
        if let Source::Playlist(id) = &self.shown.source
            && let Err(err) = self.library.set_meta(LAST_PLAYLIST, id)
        {
            tracing::warn!(%err, "saving the shown playlist");
        }
    }

    // --- internals -----------------------------------------------------------------

    /// Shows `shown`, remembering the current screen for Back.
    fn navigate(&mut self, shown: Shown) {
        if self.shown.source != Source::Empty && self.shown.source != shown.source {
            let previous = std::mem::replace(&mut self.shown, shown);
            self.back.push(previous);
            if self.back.len() > BACK_DEPTH {
                self.back.remove(0);
            }
        } else {
            self.shown = shown;
        }
        self.reset_rows();
    }

    fn replace(&mut self, shown: Shown) {
        self.shown = shown;
        self.reset_rows();
    }

    fn reset_rows(&mut self) {
        self.filter.clear();
        self.selected = None;
        self.refilter();
    }

    /// Re-reads the playlists, showing `prefer_id` (start), else the first.
    fn load_playlists(&mut self, prefer_id: Option<&str>) -> Result<()> {
        self.playlists = self.library.playlists()?;
        let index = prefer_id
            .and_then(|id| self.playlists.iter().position(|p| p.id == id))
            .or((!self.playlists.is_empty()).then_some(0));
        self.select_playlist(index)
    }

    /// After a sync: keep showing what was shown (a playlist by id; if it
    /// was deleted, the first one).
    fn reload_playlists(&mut self) -> Result<()> {
        self.playlists = self.library.playlists()?;
        match self.shown.source.clone() {
            Source::Playlist(id) => match self.playlists.iter().position(|p| p.id == id) {
                Some(_) => self.reload_keeping_place(),
                None => {
                    let first = (!self.playlists.is_empty()).then_some(0);
                    self.back.clear();
                    self.select_playlist(first)
                }
            },
            Source::Empty => {
                let first = (!self.playlists.is_empty()).then_some(0);
                self.select_playlist(first)
            }
            _ => Ok(()),
        }
    }

    /// The shown list's contents changed: reload, keep filter and row.
    fn reload_keeping_place(&mut self) -> Result<()> {
        match self.shown.source.clone() {
            Source::Playlist(id) => {
                self.shown.tracks = self.library.tracks(&id)?;
                self.shown.subtitle = format!("{} songs", self.shown.tracks.len());
                if let Some(p) = self.playlists.iter().find(|p| p.id == id) {
                    self.shown.title = p.title.clone();
                }
            }
            Source::Downloads => {
                self.shown.tracks = self
                    .library
                    .downloads()?
                    .into_iter()
                    .map(|d| d.track)
                    .collect();
            }
            Source::Saved(kind) => self.shown.items = self.library.saved(kind)?.into(),
            _ => {}
        }
        let selected = self.selected;
        self.refilter();
        self.select_row(selected);
        Ok(())
    }

    fn refilter(&mut self) {
        filter_indices(&self.shown.tracks, &self.filter, &mut self.visible);
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

    fn page(id: &str, kind: ItemKind) -> Page {
        Page {
            kind,
            id: id.into(),
            title: "ABBA Gold".into(),
            subtitle: "Album • 1992 • ABBA".into(),
            thumbnail: None,
            tracks: vec![track("fffffffffff", "Mamma Mia")],
            shelves: Vec::new(),
            channel_id: None,
        }
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
        assert!(v.apply(&changes, SessionData::default()).unwrap());
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
        assert!(!v.apply(&changes, SessionData::default()).unwrap());
        assert_eq!(v.selected_row(), Some(2));
        assert_eq!(v.playlists()[0].item_count, 3);
    }

    #[test]
    fn search_results_replace_the_list_and_back_returns() {
        let mut v = LibraryView::new(library()).unwrap();
        let results = SearchResults {
            query: "daft punk".into(),
            kind: SearchKind::Songs,
            tracks: vec![track("ggggggggggg", "Get Lucky")].into(),
            items: Arc::from([]),
        };
        let changes = Changes {
            search: true,
            ..Default::default()
        };
        let data = SessionData {
            search: Some(&results),
            ..Default::default()
        };
        assert!(v.apply(&changes, data).unwrap());
        assert!(v.showing_results());
        assert_eq!(v.selected_playlist(), None);
        assert_eq!(v.title(), "“daft punk”");
        assert_eq!(v.source_name(), "Search: daft punk");
        assert_eq!(titles(&v), ["Get Lucky"]);

        assert!(v.back());
        assert_eq!(v.title(), "Liked music");
        assert_eq!(
            v.search_query(),
            Some("daft punk"),
            "results stay reachable"
        );
        assert!(v.show_results());
        assert_eq!(titles(&v), ["Get Lucky"]);
    }

    #[test]
    fn pages_history_and_back() {
        let lib = library();
        lib.add_history(&track("aaaaaaaaaaa", "Alpha")).unwrap();
        let mut v = LibraryView::new(lib.clone()).unwrap();
        let album = page("MPREb_gold", ItemKind::Album);
        let changes = Changes {
            page: true,
            ..Default::default()
        };
        let data = SessionData {
            page: Some(&album),
            ..Default::default()
        };
        v.apply(&changes, data).unwrap();
        assert_eq!(v.title(), "ABBA Gold");
        assert_eq!(v.page_kind(), Some(ItemKind::Album));
        let (item, _) = v.page_item().unwrap();
        assert_eq!((item.kind, &*item.id), (ItemKind::Album, "MPREb_gold"));
        assert_eq!(titles(&v), ["Mamma Mia"]);

        v.show_history().unwrap();
        assert_eq!(titles(&v), ["Alpha"]);
        assert!(v.back());
        assert_eq!(v.title(), "ABBA Gold");
        assert!(v.back());
        assert_eq!(v.title(), "Liked music");
        assert!(!v.back());
    }

    #[test]
    fn saved_albums_refresh_when_changed() {
        let lib = library();
        let mut v = LibraryView::new(lib.clone()).unwrap();
        v.show_saved(ItemKind::Album).unwrap();
        assert!(v.items().is_empty());
        let mut other = LibraryView::new(lib.clone()).unwrap();
        other.show_page(&page("MPREb_gold", ItemKind::Album));
        let (item, _) = other.page_item().unwrap();
        lib.save(&item).unwrap();
        let changes = Changes {
            saved: true,
            ..Default::default()
        };
        assert!(v.apply(&changes, SessionData::default()).unwrap());
        assert_eq!(v.items().len(), 1);
        assert!(v.is_saved("MPREb_gold"));
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
        v.apply(&changes, SessionData::default()).unwrap();
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
