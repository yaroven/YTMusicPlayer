//! Browsing YouTube Music: search by category, album / artist / playlist
//! pages, the home feed, a song's radio and Related shelves, and lyrics.
//! Catalog requests are quota-free; song search falls back to the Data API
//! (signed in) and then yt-dlp when the catalog fails.

use std::{collections::HashSet, sync::Arc};

use anyhow::Result;

use super::{Background, Changes, Session};
use crate::{
    api::models::Track,
    catalog::{Item, SearchKind},
    lyrics,
};

/// Results per Data API search (one page, 100 quota units).
const API_RESULTS: u8 = 25;
/// Radio tracks already played or queued in the last this-many aren't
/// queued again.
const RADIO_RECENT: usize = 200;

/// "Eminem, Rihanna" / "A & B" / "X - Topic" → the first artist's name.
fn main_artist(artist: &str) -> String {
    let artist = artist.strip_suffix(" - Topic").unwrap_or(artist);
    let cut = [", ", " & ", " feat. ", " ft. ", " x ", " и ", " і "]
        .iter()
        .filter_map(|sep| artist.find(sep))
        .min()
        .unwrap_or(artist.len());
    artist[..cut].trim().to_owned()
}

/// What a search found: songs (playable) and/or cards to open.
#[derive(Debug, Clone, Default)]
pub struct Found {
    pub tracks: Vec<Track>,
    pub items: Vec<Item>,
}

/// The last online search.
#[derive(Debug, Clone)]
pub struct SearchResults {
    pub query: String,
    pub kind: SearchKind,
    pub tracks: Arc<[Track]>,
    pub items: Arc<[Item]>,
}

impl Session {
    pub(super) fn discover_event(&mut self, event: Background, changes: &mut Changes) {
        match event {
            Background::Searched {
                generation,
                query,
                kind,
                result,
            } if generation == self.search_generation => {
                self.searching = false;
                match result {
                    Ok(found) => {
                        let n = found.tracks.len() + found.items.len();
                        self.set_info(format!("{n} {} for “{query}”", kind.label().to_lowercase()));
                        self.search = Some(SearchResults {
                            query,
                            kind,
                            tracks: found.tracks.into(),
                            items: found.items.into(),
                        });
                        changes.search = true;
                    }
                    Err(err) => self.set_error(format!("Search failed: {err:#}")),
                }
            }
            Background::Searched { .. } => {} // a newer search replaced it
            Background::PageLoaded { generation, result } if generation == self.page_generation => {
                self.page_loading = false;
                match result {
                    Ok(page) => {
                        self.status = None;
                        self.page = Some(Arc::new(page));
                        changes.page = true;
                    }
                    Err(err) => self.set_error(format!("Couldn't open it: {err:#}")),
                }
            }
            Background::PageLoaded { .. } => {}
            Background::Home(result) => match result {
                Ok(shelves) => {
                    self.home = Some(shelves.into());
                    changes.home = true;
                }
                Err(err) => self.set_error(format!("Home: {err:#}")),
            },
            Background::Related { video_id, result } => {
                if self.queue.current().is_some_and(|t| t.video_id == video_id) {
                    match result {
                        Ok(shelves) => {
                            self.related = Some((video_id, shelves.into()));
                            changes.related = true;
                        }
                        Err(err) => tracing::debug!(%err, "related"),
                    }
                }
            }
            Background::Lyrics { video_id, result } => {
                if self.queue.current().is_some_and(|t| t.video_id == video_id) {
                    let lyrics = result.unwrap_or_else(|err| {
                        tracing::debug!(%err, "lyrics");
                        None
                    });
                    self.lyrics = Some((video_id, lyrics.map(Arc::new)));
                    changes.lyrics = true;
                }
            }
            _ => unreachable!("not a discover event"),
        }
    }

    /// Searches YouTube Music in one category.
    pub fn search(&mut self, query: &str, kind: SearchKind) {
        let query = query.trim().to_owned();
        if query.is_empty() {
            return;
        }
        self.search_generation += 1;
        self.searching = true;
        self.set_info(format!("Searching “{query}”…"));
        let (catalog, youtube, source, tx, generation) = (
            self.deps.catalog.clone(),
            self.deps.account.youtube(),
            self.deps.source.clone(),
            self.tx.clone(),
            self.search_generation,
        );
        tokio::spawn(async move {
            let result = async {
                let items = match catalog.search(&query, kind).await {
                    Ok(items) => items,
                    Err(err) if kind == SearchKind::Songs => {
                        tracing::warn!(%err, "catalog search failed; falling back");
                        let tracks = match &youtube {
                            Some(yt) => match yt.search(&query, API_RESULTS).await {
                                Ok(tracks) => tracks,
                                Err(_) => source.search(&query, API_RESULTS).await?,
                            },
                            None => source.search(&query, API_RESULTS).await?,
                        };
                        return Ok(Found {
                            tracks,
                            items: Vec::new(),
                        });
                    }
                    Err(err) => return Err(err),
                };
                let tracks = items.iter().filter_map(|i| i.track.clone()).collect();
                let items = if kind == SearchKind::Songs {
                    Vec::new()
                } else {
                    items
                };
                Ok::<_, anyhow::Error>(Found { tracks, items })
            }
            .await;
            let _ = tx.send(Background::Searched {
                generation,
                query,
                kind,
                result,
            });
        });
    }

    /// Opens an album, artist or playlist page.
    pub fn open_page(&mut self, browse_id: &str) {
        self.page_generation += 1;
        self.page_loading = true;
        self.set_info("Loading…");
        let (catalog, tx, generation, id) = (
            self.deps.catalog.clone(),
            self.tx.clone(),
            self.page_generation,
            browse_id.to_owned(),
        );
        tokio::spawn(async move {
            let result = catalog.page(&id).await;
            let _ = tx.send(Background::PageLoaded { generation, result });
        });
    }

    /// Opens the page of `track`'s artist: the one YouTube Music links
    /// under the song, else the best name match.
    pub fn open_artist(&mut self, track: &Track) {
        self.page_generation += 1;
        self.page_loading = true;
        let name = main_artist(&track.artist);
        self.set_info(format!("Opening {name}…"));
        let (catalog, tx, generation, video_id) = (
            self.deps.catalog.clone(),
            self.tx.clone(),
            self.page_generation,
            track.video_id.clone(),
        );
        tokio::spawn(async move {
            let result = async {
                let id = match catalog.artist_id(&video_id).await {
                    Ok(Some(id)) => id,
                    other => {
                        if let Err(err) = other {
                            tracing::debug!(%err, "artist from the song");
                        }
                        let found = catalog.search(&name, SearchKind::Artists).await?;
                        let pick = found
                            .iter()
                            .find(|i| i.title.eq_ignore_ascii_case(&name))
                            .or(found.first())
                            .ok_or_else(|| anyhow::anyhow!("no artist named {name}"))?;
                        pick.id.to_string()
                    }
                };
                catalog.page(&id).await
            }
            .await;
            let _ = tx.send(Background::PageLoaded { generation, result });
        });
    }

    /// Loads the home feed (once; again with `refresh`).
    pub fn load_home(&mut self, refresh: bool) {
        if self.home.is_some() && !refresh {
            return;
        }
        let (catalog, tx) = (self.deps.catalog.clone(), self.tx.clone());
        tokio::spawn(async move {
            let _ = tx.send(Background::Home(catalog.home().await));
        });
    }

    /// Plays `track` followed by its radio.
    pub fn start_radio(&mut self, track: Track) {
        self.play(vec![track.clone()].into(), 0);
        self.set_info(format!("Radio: {}", track.title));
        self.fetch_radio(track, false);
    }

    pub(super) fn fetch_radio(&mut self, seed: Track, extend: bool) {
        if extend {
            if self.extending {
                return;
            }
            self.extending = true;
        }
        let (catalog, tx) = (self.deps.catalog.clone(), self.tx.clone());
        tokio::spawn(async move {
            let result = catalog.radio(&seed.video_id).await;
            let _ = tx.send(Background::Radio {
                seed,
                extend,
                result,
            });
        });
    }

    /// Queues radio tracks not heard recently. `extend`: autoplay at the
    /// end of the queue — start the first one if playback had stopped.
    pub(super) fn radio_arrived(&mut self, seed: Track, extend: bool, result: Result<Vec<Track>>) {
        if extend {
            self.extending = false;
        }
        let tracks = match result {
            Ok(tracks) => tracks,
            Err(err) => {
                self.set_error(format!("Radio: {err:#}"));
                return;
            }
        };
        let mut seen: HashSet<Arc<str>> = self
            .queue
            .upcoming()
            .take(RADIO_RECENT)
            .map(|t| t.video_id.clone())
            .collect();
        seen.insert(seed.video_id.clone());
        if let Ok(history) = self.deps.library.history() {
            seen.extend(
                history
                    .iter()
                    .take(RADIO_RECENT)
                    .map(|t| t.video_id.clone()),
            );
        }
        let fresh: Vec<Track> = tracks
            .into_iter()
            .filter(|t| seen.insert(t.video_id.clone()))
            .collect();
        if fresh.is_empty() {
            return;
        }
        let was_idle =
            self.player_status.state == crate::audio::player::PlayState::Idle && !self.loading;
        self.queue.append(fresh);
        self.queue_changed();
        if extend && was_idle && self.queue.advance().is_some() {
            self.play_current();
        }
    }

    /// Show lyrics of the playing track (fetched now and on every change).
    pub fn want_lyrics(&mut self, on: bool) {
        self.want_lyrics = on;
        if on {
            self.fetch_lyrics();
        }
    }

    /// Show Related shelves of the playing track.
    pub fn want_related(&mut self, on: bool) {
        self.want_related = on;
        if on {
            self.fetch_related();
        }
    }

    /// The playing track changed: refresh what's shown about it.
    pub(super) fn now_changed(&mut self) {
        if self.want_lyrics {
            self.fetch_lyrics();
        }
        if self.want_related {
            self.fetch_related();
        }
    }

    fn fetch_lyrics(&mut self) {
        let Some(track) = self.queue.current().cloned() else {
            return;
        };
        if self
            .lyrics
            .as_ref()
            .is_some_and(|(id, _)| *id == track.video_id)
        {
            return;
        }
        self.lyrics = None;
        let duration = track
            .duration_secs
            .map(|s| std::time::Duration::from_secs(s.into()))
            .or(self.player_status.duration);
        let (http, catalog, tx) = (
            self.deps.http.clone(),
            self.deps.catalog.clone(),
            self.tx.clone(),
        );
        tokio::spawn(async move {
            let result = lyrics::find(&http, &catalog, &track, duration).await;
            let _ = tx.send(Background::Lyrics {
                video_id: track.video_id,
                result,
            });
        });
    }

    fn fetch_related(&mut self) {
        let Some(track) = self.queue.current().cloned() else {
            return;
        };
        if self
            .related
            .as_ref()
            .is_some_and(|(id, _)| *id == track.video_id)
        {
            return;
        }
        let (catalog, tx) = (self.deps.catalog.clone(), self.tx.clone());
        tokio::spawn(async move {
            let result = catalog.related(&track.video_id).await;
            let _ = tx.send(Background::Related {
                video_id: track.video_id,
                result,
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_artist_of_a_credit() {
        assert_eq!(main_artist("Eminem, Rihanna"), "Eminem");
        assert_eq!(main_artist("Simon & Garfunkel"), "Simon");
        assert_eq!(main_artist("ABBA - Topic"), "ABBA");
        assert_eq!(main_artist("Queen"), "Queen");
    }
}
