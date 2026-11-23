//! Local SQLite cache of the library, so startup and browsing cost no API
//! quota. Calls are small (ms); async callers may wrap them in `spawn_blocking`.
//! Also stores resolved stream URLs and small UI state (`meta`).

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};

use crate::{
    api::models::{LIKED_PLAYLIST_ID, Playlist, Track},
    catalog::{Item, ItemKind},
};

const SCHEMA_VERSION: i32 = 6;
/// Plays kept in the history (older ones are dropped).
const HISTORY_CAP: i64 = 500;

/// A synced playlist with its tracks and, per track, the API's playlist
/// item id (needed to remove it again; empty when unknown).
pub struct SyncedPlaylist {
    pub playlist: Playlist,
    pub tracks: Arc<[Track]>,
    pub item_ids: Vec<String>,
}

/// A list kept offline: downloaded whole, and every song added to it later
/// downloaded too. One of the user's playlists (its songs come from the
/// library, so syncs and edits reach it) or a page — an album, someone
/// else's playlist — whose songs are kept as they were when it was chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfflineList {
    /// Playlist id (library) or browse id (page).
    pub id: String,
    pub title: String,
    pub thumbnail: Option<String>,
}

/// A downloaded track and where its audio is.
#[derive(Debug, Clone)]
pub struct Download {
    pub track: Track,
    pub path: PathBuf,
    pub bytes: u64,
}

pub struct Library {
    conn: Mutex<Connection>,
}

impl Library {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 2000;
             PRAGMA cache_size = -256;",
        )?;
        migrate(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Liked first, then the user's playlists in API order.
    pub fn playlists(&self) -> Result<Vec<Playlist>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, title, item_count, etag FROM playlists ORDER BY id != ?1, position",
        )?;
        let rows = stmt.query_map([LIKED_PLAYLIST_ID], |r| {
            Ok(Playlist {
                id: r.get(0)?,
                title: r.get(1)?,
                item_count: r.get(2)?,
                etag: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Tracks in playlist order, as one shared allocation for view and queue.
    pub fn tracks(&self, playlist_id: &str) -> Result<Arc<[Track]>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT t.video_id, t.title, t.artist, t.duration_secs, t.explicit
             FROM playlist_tracks pt JOIN tracks t ON t.video_id = pt.video_id
             WHERE pt.playlist_id = ?1 ORDER BY pt.position",
        )?;
        let rows = stmt.query_map([playlist_id], |r| {
            Ok(Track {
                video_id: r.get::<_, String>(0)?.into(),
                title: r.get::<_, String>(1)?.into(),
                artist: r.get::<_, String>(2)?.into(),
                duration_secs: r.get(3)?,
                explicit: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn contains(&self, playlist_id: &str, video_id: &str) -> Result<bool> {
        Ok(self
            .conn()
            .query_row(
                "SELECT 1 FROM playlist_tracks WHERE playlist_id = ?1 AND video_id = ?2",
                [playlist_id, video_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// Adds a track at the front (`front`) or end of a playlist.
    pub fn add_track(&self, playlist_id: &str, track: &Track, front: bool) -> Result<()> {
        self.add_item(playlist_id, track, front, None)
    }

    /// [`add_track`](Self::add_track), remembering the API's playlist item id.
    pub fn add_item(
        &self,
        playlist_id: &str,
        track: &Track,
        front: bool,
        item_id: Option<&str>,
    ) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        upsert_track(&tx, track)?;
        // Positions may go negative; only their order matters.
        let edge = if front {
            "SELECT COALESCE(MIN(position), 0) - 1 FROM playlist_tracks WHERE playlist_id = ?1"
        } else {
            "SELECT COALESCE(MAX(position), -1) + 1 FROM playlist_tracks WHERE playlist_id = ?1"
        };
        let pos: i64 = tx.query_row(edge, [playlist_id], |r| r.get(0))?;
        tx.execute(
            "INSERT INTO playlist_tracks (playlist_id, position, video_id, item_id)
             VALUES (?1, ?2, ?3, ?4)",
            params![playlist_id, pos, &*track.video_id, item_id],
        )?;
        tx.execute(
            "UPDATE playlists SET item_count = item_count + 1 WHERE id = ?1",
            [playlist_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn remove_track(&self, playlist_id: &str, video_id: &str) -> Result<()> {
        let conn = self.conn();
        let removed = conn.execute(
            "DELETE FROM playlist_tracks WHERE playlist_id = ?1 AND video_id = ?2",
            [playlist_id, video_id],
        )?;
        conn.execute(
            "UPDATE playlists SET item_count = MAX(item_count - ?2, 0) WHERE id = ?1",
            params![playlist_id, removed as i64],
        )?;
        Ok(())
    }

    /// Moves the song at `from` to `to` (indices in playlist order) and
    /// renumbers the playlist.
    pub fn move_track(&self, playlist_id: &str, from: usize, to: usize) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let mut rows: Vec<(String, Option<String>)> = {
            let mut stmt = tx.prepare(
                "SELECT video_id, item_id FROM playlist_tracks
                 WHERE playlist_id = ?1 ORDER BY position",
            )?;
            let rows = stmt.query_map([playlist_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<Result<_, _>>()?
        };
        if from >= rows.len() || to >= rows.len() {
            return Ok(());
        }
        let row = rows.remove(from);
        rows.insert(to, row);
        tx.execute(
            "DELETE FROM playlist_tracks WHERE playlist_id = ?1",
            [playlist_id],
        )?;
        for (position, (video_id, item_id)) in rows.iter().enumerate() {
            tx.execute(
                "INSERT INTO playlist_tracks (playlist_id, position, video_id, item_id)
                 VALUES (?1, ?2, ?3, ?4)",
                params![playlist_id, position as i64, video_id, item_id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Playlist item ids in track order ("" where unknown).
    pub fn item_ids(&self, playlist_id: &str) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT COALESCE(item_id, '') FROM playlist_tracks
             WHERE playlist_id = ?1 ORDER BY position",
        )?;
        let rows = stmt.query_map([playlist_id], |r| r.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The API's playlist item id of `video_id` in `playlist_id`.
    pub fn item_id(&self, playlist_id: &str, video_id: &str) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT item_id FROM playlist_tracks
                 WHERE playlist_id = ?1 AND video_id = ?2 AND item_id IS NOT NULL",
                [playlist_id, video_id],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// A new (empty) playlist after the others.
    pub fn add_playlist(&self, playlist: &Playlist) -> Result<()> {
        self.conn().execute(
            "INSERT INTO playlists (id, title, item_count, position, etag)
             VALUES (?1, ?2, 0, (SELECT COALESCE(MAX(position), 0) + 1 FROM playlists), NULL)",
            params![playlist.id, playlist.title],
        )?;
        Ok(())
    }

    pub fn rename_playlist(&self, id: &str, title: &str) -> Result<()> {
        self.conn()
            .execute("UPDATE playlists SET title = ?2 WHERE id = ?1", [id, title])?;
        Ok(())
    }

    /// Deletes a playlist (and stops keeping it offline).
    pub fn delete_playlist(&self, id: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute("DELETE FROM playlists WHERE id = ?1", [id])?;
        conn.execute("DELETE FROM offline_lists WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Whether `id` names one of the user's playlists.
    pub fn has_playlist(&self, id: &str) -> Result<bool> {
        Ok(self
            .conn()
            .query_row("SELECT 1 FROM playlists WHERE id = ?1", [id], |_| Ok(()))
            .optional()?
            .is_some())
    }

    // --- lists kept offline --------------------------------------------------------

    /// Keeps `list` offline (from now on, at the end of the others). A page
    /// passes its `tracks`, kept as they are; a library playlist passes
    /// `None` and is read from the library.
    pub fn keep_offline(&self, list: &OfflineList, tracks: Option<&[Track]>) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO offline_lists (id, title, thumbnail, added_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET title = excluded.title,
               thumbnail = COALESCE(excluded.thumbnail, offline_lists.thumbnail)",
            params![
                list.id,
                list.title,
                list.thumbnail,
                chrono::Utc::now().timestamp()
            ],
        )?;
        if let Some(tracks) = tracks {
            tx.execute("DELETE FROM offline_tracks WHERE list_id = ?1", [&list.id])?;
            let mut insert = tx.prepare(
                "INSERT INTO offline_tracks
                   (list_id, position, video_id, title, artist, duration_secs, explicit)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            for (i, t) in tracks.iter().enumerate() {
                insert.execute(params![
                    list.id,
                    i as i64,
                    &*t.video_id,
                    &*t.title,
                    &*t.artist,
                    t.duration_secs,
                    t.explicit
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Stops keeping `id` offline (its downloads stay; the caller decides).
    pub fn forget_offline(&self, id: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM offline_lists WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn is_kept_offline(&self, id: &str) -> Result<bool> {
        Ok(self
            .conn()
            .query_row(
                "SELECT 1 FROM offline_lists WHERE id = ?1",
                [id],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// The lists kept offline, in the order they were chosen; a library
    /// playlist under its current name.
    pub fn offline_lists(&self) -> Result<Vec<OfflineList>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT o.id, COALESCE(p.title, o.title), o.thumbnail
             FROM offline_lists o LEFT JOIN playlists p ON p.id = o.id
             ORDER BY o.added_at, o.rowid",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(OfflineList {
                id: r.get(0)?,
                title: r.get(1)?,
                thumbnail: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The songs of a kept list in its own order: the library playlist's
    /// as they are now, else the page's as they were kept.
    pub fn offline_tracks(&self, id: &str) -> Result<Arc<[Track]>> {
        if self.has_playlist(id)? {
            return self.tracks(id);
        }
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT video_id, title, artist, duration_secs, explicit FROM offline_tracks
             WHERE list_id = ?1 ORDER BY position",
        )?;
        let rows = stmt.query_map([id], track_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Songs of the kept lists that aren't downloaded yet: list by list
    /// (in the order they were chosen), each in its own order, once each.
    pub fn offline_missing(&self) -> Result<Vec<Track>> {
        let mut seen: std::collections::HashSet<Arc<str>> = self
            .downloads()?
            .into_iter()
            .map(|d| d.track.video_id)
            .collect();
        let mut missing = Vec::new();
        for list in self.offline_lists()? {
            for t in self.offline_tracks(&list.id)?.iter() {
                if seen.insert(t.video_id.clone()) {
                    missing.push(t.clone());
                }
            }
        }
        Ok(missing)
    }

    // --- history -------------------------------------------------------------------

    /// Records a play (kept: the last [`HISTORY_CAP`]).
    pub fn add_history(&self, track: &Track) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO history (video_id, title, artist, duration_secs, played_at, explicit)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                &*track.video_id,
                &*track.title,
                &*track.artist,
                track.duration_secs,
                chrono::Utc::now().timestamp(),
                track.explicit
            ],
        )?;
        conn.execute(
            "DELETE FROM history WHERE id <= (SELECT MAX(id) FROM history) - ?1",
            [HISTORY_CAP],
        )?;
        conn.execute(
            "INSERT INTO plays (video_id, count, last_played) VALUES (?1, 1, ?2)
             ON CONFLICT(video_id) DO UPDATE SET count = count + 1,
               last_played = excluded.last_played",
            params![&*track.video_id, chrono::Utc::now().timestamp()],
        )?;
        Ok(())
    }

    /// Forgets a song's plays: out of the history and its play count.
    pub fn remove_history(&self, video_id: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute("DELETE FROM history WHERE video_id = ?1", [video_id])?;
        conn.execute("DELETE FROM plays WHERE video_id = ?1", [video_id])?;
        Ok(())
    }

    /// Forgets every play.
    pub fn clear_history(&self) -> Result<()> {
        self.conn()
            .execute_batch("DELETE FROM history; DELETE FROM plays;")?;
        Ok(())
    }

    /// How many times each song was played (songs never played are absent).
    pub fn play_counts(&self) -> Result<HashMap<String, u32>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT video_id, count FROM plays")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Played tracks, most recent first, each once.
    pub fn history(&self) -> Result<Arc<[Track]>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT video_id, title, artist, duration_secs, explicit FROM history
             WHERE id IN (SELECT MAX(id) FROM history GROUP BY video_id)
             ORDER BY id DESC",
        )?;
        let rows = stmt.query_map([], track_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    // --- saved albums and followed artists -----------------------------------------

    /// Albums (`Album`) or artists (`Artist`) the user keeps, newest first.
    pub fn saved(&self, kind: ItemKind) -> Result<Vec<Item>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, title, subtitle, thumbnail FROM saved WHERE kind = ?1
             ORDER BY position DESC",
        )?;
        let rows = stmt.query_map([kind_name(kind)], |r| {
            Ok(Item {
                kind,
                id: r.get::<_, String>(0)?.into(),
                title: r.get::<_, String>(1)?.into(),
                subtitle: r.get::<_, String>(2)?.into(),
                thumbnail: r.get::<_, Option<String>>(3)?.map(Into::into),
                track: None,
                explicit: false,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn is_saved(&self, id: &str) -> Result<bool> {
        Ok(self
            .conn()
            .query_row("SELECT 1 FROM saved WHERE id = ?1", [id], |_| Ok(()))
            .optional()?
            .is_some())
    }

    pub fn save(&self, item: &Item) -> Result<()> {
        self.conn().execute(
            "INSERT INTO saved (id, kind, title, subtitle, thumbnail, position)
             VALUES (?1, ?2, ?3, ?4, ?5, (SELECT COALESCE(MAX(position), 0) + 1 FROM saved))
             ON CONFLICT(id) DO UPDATE SET title = excluded.title,
               subtitle = excluded.subtitle, thumbnail = excluded.thumbnail",
            params![
                &*item.id,
                kind_name(item.kind),
                &*item.title,
                &*item.subtitle,
                item.thumbnail.as_deref()
            ],
        )?;
        Ok(())
    }

    pub fn unsave(&self, id: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM saved WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Replaces the followed artists (from the account), keeping ones
    /// followed only here.
    pub fn replace_subscriptions(&self, artists: &[Item]) -> Result<()> {
        self.replace_saved_synced(ItemKind::Artist, artists)
    }

    /// Replaces the account's items of one kind (followed artists, saved
    /// albums), keeping the ones saved only here.
    pub fn replace_saved_synced(&self, kind: ItemKind, items: &[Item]) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM saved WHERE kind = ?1 AND synced = 1",
            [kind_name(kind)],
        )?;
        for (i, a) in items.iter().enumerate().rev() {
            tx.execute(
                "INSERT INTO saved (id, kind, title, subtitle, thumbnail, position, synced)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1)
                 ON CONFLICT(id) DO UPDATE SET synced = 1",
                params![
                    &*a.id,
                    kind_name(kind),
                    &*a.title,
                    &*a.subtitle,
                    a.thumbnail.as_deref(),
                    -(i as i64)
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    // --- downloads -----------------------------------------------------------------

    pub fn add_download(&self, download: &Download) -> Result<()> {
        let t = &download.track;
        self.conn().execute(
            "INSERT INTO downloads (video_id, title, artist, duration_secs, path, bytes, downloaded_at, explicit)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(video_id) DO UPDATE SET path = excluded.path, bytes = excluded.bytes",
            params![
                &*t.video_id,
                &*t.title,
                &*t.artist,
                t.duration_secs,
                download.path.to_string_lossy(),
                download.bytes as i64,
                chrono::Utc::now().timestamp(),
                t.explicit
            ],
        )?;
        Ok(())
    }

    pub fn remove_download(&self, video_id: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM downloads WHERE video_id = ?1", [video_id])?;
        Ok(())
    }

    /// Forgets every download (the files are deleted by the caller).
    pub fn clear_downloads(&self) -> Result<()> {
        self.conn().execute("DELETE FROM downloads", [])?;
        Ok(())
    }

    /// Downloaded tracks in download order, so a downloaded playlist keeps
    /// its order (newest first looked shuffled; user's report 2026-10-26).
    pub fn downloads(&self) -> Result<Vec<Download>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT video_id, title, artist, duration_secs, path, bytes, explicit FROM downloads
             ORDER BY downloaded_at, rowid",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(Download {
                track: Track {
                    explicit: r.get(6)?,
                    ..track_row(r)?
                },
                path: PathBuf::from(r.get::<_, String>(4)?),
                bytes: r.get::<_, i64>(5)? as u64,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Records which video's sound a download holds (the studio song's, for
    /// a clip).
    pub fn set_download_audio(&self, video_id: &str, audio_id: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE downloads SET audio_id = ?2 WHERE video_id = ?1",
            [video_id, audio_id],
        )?;
        Ok(())
    }

    /// Downloads made before the player recorded whose sound they hold: a
    /// clip's may have its own sound instead of the studio song's.
    pub fn downloads_without_audio(&self) -> Result<Vec<Track>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT video_id, title, artist, duration_secs, explicit FROM downloads
             WHERE audio_id IS NULL ORDER BY downloaded_at, rowid",
        )?;
        let rows = stmt.query_map([], track_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn download_path(&self, video_id: &str) -> Result<Option<PathBuf>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT path FROM downloads WHERE video_id = ?1",
                [video_id],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .map(PathBuf::from))
    }

    pub fn get_meta(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()?)
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn().execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [key, value],
        )?;
        Ok(())
    }

    /// Cached stream JSON for `video_id` if it expires after `min_expiry` (unix s).
    pub fn cached_stream(&self, video_id: &str, min_expiry: i64) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT json FROM stream_cache WHERE video_id = ?1 AND expires_at > ?2",
                params![video_id, min_expiry],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn put_stream(&self, video_id: &str, json: &str, expires_at: i64) -> Result<()> {
        self.conn().execute(
            "INSERT INTO stream_cache (video_id, json, expires_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(video_id) DO UPDATE SET json = excluded.json, expires_at = excluded.expires_at",
            params![video_id, json, expires_at],
        )?;
        Ok(())
    }

    pub fn purge_expired_streams(&self, now: i64) -> Result<()> {
        self.conn()
            .execute("DELETE FROM stream_cache WHERE expires_at <= ?1", [now])?;
        Ok(())
    }

    /// Playlist id -> ETag from the last sync, to skip unchanged playlists.
    pub fn playlist_etags(&self) -> Result<HashMap<String, String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT id, etag FROM playlists WHERE etag IS NOT NULL")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn is_empty(&self) -> Result<bool> {
        let count: i64 = self
            .conn()
            .query_row("SELECT COUNT(*) FROM playlists", [], |r| r.get(0))?;
        Ok(count == 0)
    }

    /// Unix seconds of the last completed sync.
    pub fn last_sync(&self) -> Result<Option<i64>> {
        Ok(self
            .conn()
            .query_row("SELECT value FROM meta WHERE key = 'last_sync'", [], |r| {
                r.get::<_, String>(0)
            })
            .optional()?
            .and_then(|v| v.parse().ok()))
    }

    /// Replaces the whole library atomically: a failed sync leaves the old one.
    pub fn replace_library(&self, playlists: &[(Playlist, Arc<[Track]>)]) -> Result<()> {
        let synced: Vec<SyncedPlaylist> = playlists
            .iter()
            .map(|(playlist, tracks)| SyncedPlaylist {
                playlist: playlist.clone(),
                tracks: tracks.clone(),
                item_ids: Vec::new(),
            })
            .collect();
        self.replace_synced(&synced)
    }

    /// [`replace_library`](Self::replace_library) with playlist item ids.
    pub fn replace_synced(&self, playlists: &[SyncedPlaylist]) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute_batch("DELETE FROM playlist_tracks; DELETE FROM playlists;")?;
        {
            let mut add_playlist = tx.prepare(
                "INSERT INTO playlists (id, title, item_count, position, etag)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            let mut link = tx.prepare(
                "INSERT OR IGNORE INTO playlist_tracks (playlist_id, position, video_id, item_id)
                 VALUES (?1, ?2, ?3, ?4)",
            )?;
            for (pos, synced) in playlists.iter().enumerate() {
                let (playlist, tracks) = (&synced.playlist, &synced.tracks);
                add_playlist.execute(params![
                    playlist.id,
                    playlist.title,
                    tracks.len() as u32,
                    pos as i64,
                    playlist.etag
                ])?;
                for (i, t) in tracks.iter().enumerate() {
                    upsert_track(&tx, t)?;
                    let item_id = synced.item_ids.get(i).filter(|id| !id.is_empty());
                    link.execute(params![playlist.id, i as i64, &*t.video_id, item_id])?;
                }
            }
        }
        tx.execute(
            "INSERT INTO meta (key, value) VALUES ('last_sync', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [chrono::Utc::now().timestamp().to_string()],
        )?;
        // Drop tracks no playlist references any more.
        tx.execute(
            "DELETE FROM tracks WHERE video_id NOT IN (SELECT video_id FROM playlist_tracks)",
            [],
        )?;
        tx.commit()?;
        Ok(())
    }
}

/// Columns 0..=3 are the track; `explicit` is read when the query selects
/// it as column 4 (`downloads` puts it elsewhere and sets it itself).
fn track_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Track> {
    Ok(Track {
        video_id: r.get::<_, String>(0)?.into(),
        title: r.get::<_, String>(1)?.into(),
        artist: r.get::<_, String>(2)?.into(),
        duration_secs: r.get(3)?,
        explicit: r.get(4).unwrap_or(false),
    })
}

fn kind_name(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::Song => "song",
        ItemKind::Album => "album",
        ItemKind::Artist => "artist",
        ItemKind::Playlist => "playlist",
    }
}

/// Keeps a known duration if this listing lacks one.
fn upsert_track(conn: &Connection, t: &Track) -> Result<()> {
    conn.prepare_cached(
        "INSERT INTO tracks (video_id, title, artist, duration_secs, explicit) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(video_id) DO UPDATE SET title = excluded.title,
           artist = excluded.artist,
           duration_secs = COALESCE(excluded.duration_secs, tracks.duration_secs),
           explicit = tracks.explicit | excluded.explicit",
    )?
    .execute(params![
        &*t.video_id,
        &*t.title,
        &*t.artist,
        t.duration_secs,
        t.explicit
    ])?;
    Ok(())
}

fn migrate(conn: &Connection) -> Result<()> {
    let version: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version < 1 {
        conn.execute_batch(
            "BEGIN;
             CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE playlists (
               id TEXT PRIMARY KEY, title TEXT NOT NULL,
               item_count INTEGER NOT NULL, position INTEGER NOT NULL);
             CREATE TABLE tracks (
               video_id TEXT PRIMARY KEY, title TEXT NOT NULL,
               artist TEXT NOT NULL, duration_secs INTEGER);
             CREATE TABLE playlist_tracks (
               playlist_id TEXT NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
               position INTEGER NOT NULL,
               video_id TEXT NOT NULL REFERENCES tracks(video_id),
               PRIMARY KEY (playlist_id, position));
             COMMIT;",
        )?;
    }
    if version < 2 {
        conn.execute_batch(
            "BEGIN;
             ALTER TABLE playlists ADD COLUMN etag TEXT;
             CREATE TABLE stream_cache (
               video_id TEXT PRIMARY KEY, json TEXT NOT NULL,
               expires_at INTEGER NOT NULL);
             COMMIT;",
        )?;
    }
    if version < 3 {
        conn.execute_batch(
            "BEGIN;
             ALTER TABLE playlist_tracks ADD COLUMN item_id TEXT;
             CREATE TABLE history (
               id INTEGER PRIMARY KEY AUTOINCREMENT, video_id TEXT NOT NULL,
               title TEXT NOT NULL, artist TEXT NOT NULL, duration_secs INTEGER,
               played_at INTEGER NOT NULL);
             CREATE TABLE saved (
               id TEXT PRIMARY KEY, kind TEXT NOT NULL, title TEXT NOT NULL,
               subtitle TEXT NOT NULL, thumbnail TEXT, position INTEGER NOT NULL,
               synced INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE downloads (
               video_id TEXT PRIMARY KEY, title TEXT NOT NULL, artist TEXT NOT NULL,
               duration_secs INTEGER, path TEXT NOT NULL, bytes INTEGER NOT NULL,
               downloaded_at INTEGER NOT NULL);
             COMMIT;",
        )?;
    }
    if version < 4 {
        // YouTube Music's "E" badge; unknown (0) for rows synced before.
        conn.execute_batch(
            "BEGIN;
             ALTER TABLE tracks ADD COLUMN explicit INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE history ADD COLUMN explicit INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE downloads ADD COLUMN explicit INTEGER NOT NULL DEFAULT 0;
             COMMIT;",
        )?;
    }
    if version < 5 {
        // Plays per song, kept apart from the capped history (it forgets).
        conn.execute_batch(
            "BEGIN;
             CREATE TABLE plays (
               video_id TEXT PRIMARY KEY, count INTEGER NOT NULL,
               last_played INTEGER NOT NULL);
             INSERT INTO plays (video_id, count, last_played)
               SELECT video_id, COUNT(*), MAX(played_at) FROM history GROUP BY video_id;
             COMMIT;",
        )?;
    }
    if version < 6 {
        // Lists kept offline (playlists downloaded whole so far become
        // ones), and whose sound each download holds (unknown before).
        conn.execute_batch(
            "BEGIN;
             ALTER TABLE downloads ADD COLUMN audio_id TEXT;
             CREATE TABLE offline_lists (
               id TEXT PRIMARY KEY, title TEXT NOT NULL, thumbnail TEXT,
               added_at INTEGER NOT NULL);
             CREATE TABLE offline_tracks (
               list_id TEXT NOT NULL REFERENCES offline_lists(id) ON DELETE CASCADE,
               position INTEGER NOT NULL, video_id TEXT NOT NULL,
               title TEXT NOT NULL, artist TEXT NOT NULL, duration_secs INTEGER,
               explicit INTEGER NOT NULL DEFAULT 0,
               PRIMARY KEY (list_id, position));
             INSERT INTO offline_lists (id, title, added_at)
               SELECT id, title, 0 FROM playlists p
               WHERE EXISTS (SELECT 1 FROM playlist_tracks pt WHERE pt.playlist_id = p.id)
                 AND NOT EXISTS (
                   SELECT 1 FROM playlist_tracks pt
                   LEFT JOIN downloads d ON d.video_id = pt.video_id
                   WHERE pt.playlist_id = p.id AND d.video_id IS NULL)
               ORDER BY id != '__liked__', position;
             COMMIT;",
        )?;
    }
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(id: &str, dur: Option<u32>) -> Track {
        Track {
            video_id: id.into(),
            title: id.into(),
            artist: "A".into(),
            duration_secs: dur,
            explicit: false,
        }
    }

    #[test]
    fn replace_and_read_back() {
        let lib = Library::open_in_memory().unwrap();
        assert!(lib.is_empty().unwrap());
        let mine = Playlist {
            id: "PL1".into(),
            title: "Mine".into(),
            item_count: 0,
            etag: Some("e1".into()),
        };
        let liked = Playlist {
            id: LIKED_PLAYLIST_ID.into(),
            title: "Liked".into(),
            item_count: 0,
            etag: None,
        };
        lib.replace_library(&[
            (
                mine.clone(),
                vec![track("aaaaaaaaaaa", None), track("bbbbbbbbbbb", None)].into(),
            ),
            (liked, vec![track("aaaaaaaaaaa", Some(200))].into()),
        ])
        .unwrap();

        let playlists = lib.playlists().unwrap();
        assert_eq!(playlists[0].id, LIKED_PLAYLIST_ID, "liked sorts first");
        assert_eq!(playlists[1].item_count, 2);
        let tracks = lib.tracks("PL1").unwrap();
        assert_eq!(tracks.len(), 2);
        assert_eq!(
            tracks[0].duration_secs,
            Some(200),
            "duration kept from liked"
        );
        assert!(lib.last_sync().unwrap().is_some());
        assert_eq!(lib.playlist_etags().unwrap()["PL1"], "e1");

        lib.add_track(LIKED_PLAYLIST_ID, &track("ccccccccccc", None), true)
            .unwrap();
        let liked = lib.tracks(LIKED_PLAYLIST_ID).unwrap();
        assert_eq!(&*liked[0].video_id, "ccccccccccc", "added at the front");
        assert!(lib.contains(LIKED_PLAYLIST_ID, "ccccccccccc").unwrap());
        lib.remove_track(LIKED_PLAYLIST_ID, "ccccccccccc").unwrap();
        assert_eq!(lib.tracks(LIKED_PLAYLIST_ID).unwrap().len(), 1);

        lib.replace_library(&[(mine, Vec::new().into())]).unwrap();
        assert!(lib.tracks("PL1").unwrap().is_empty());
    }

    #[test]
    fn history_is_recent_first_and_unique() {
        let lib = Library::open_in_memory().unwrap();
        for id in ["aaaaaaaaaaa", "bbbbbbbbbbb", "aaaaaaaaaaa"] {
            lib.add_history(&track(id, None)).unwrap();
        }
        let ids: Vec<_> = lib
            .history()
            .unwrap()
            .iter()
            .map(|t| t.video_id.to_string())
            .collect();
        assert_eq!(ids, ["aaaaaaaaaaa", "bbbbbbbbbbb"]);
    }

    #[test]
    fn plays_are_counted_and_can_be_forgotten() {
        let lib = Library::open_in_memory().unwrap();
        for id in ["aaaaaaaaaaa", "bbbbbbbbbbb", "aaaaaaaaaaa"] {
            lib.add_history(&track(id, None)).unwrap();
        }
        let counts = lib.play_counts().unwrap();
        assert_eq!(counts["aaaaaaaaaaa"], 2);
        assert_eq!(counts["bbbbbbbbbbb"], 1);

        lib.remove_history("aaaaaaaaaaa").unwrap();
        assert_eq!(lib.history().unwrap().len(), 1);
        assert!(!lib.play_counts().unwrap().contains_key("aaaaaaaaaaa"));
        lib.clear_history().unwrap();
        assert!(lib.history().unwrap().is_empty() && lib.play_counts().unwrap().is_empty());
    }

    #[test]
    fn songs_move_inside_a_playlist() {
        let lib = Library::open_in_memory().unwrap();
        let tracks: Vec<Track> = ["aaaaaaaaaaa", "bbbbbbbbbbb", "ccccccccccc"]
            .into_iter()
            .map(|id| track(id, None))
            .collect();
        lib.replace_synced(&[SyncedPlaylist {
            playlist: Playlist {
                id: "PL1".into(),
                title: "Road trip".into(),
                item_count: 3,
                etag: None,
            },
            tracks: tracks.into(),
            item_ids: vec!["s1".into(), "s2".into(), "s3".into()],
        }])
        .unwrap();
        lib.move_track("PL1", 2, 0).unwrap();
        let ids: Vec<_> = lib
            .tracks("PL1")
            .unwrap()
            .iter()
            .map(|t| t.video_id.to_string())
            .collect();
        assert_eq!(ids, ["ccccccccccc", "aaaaaaaaaaa", "bbbbbbbbbbb"]);
        assert_eq!(
            lib.item_ids("PL1").unwrap(),
            ["s3", "s1", "s2"],
            "ids move along"
        );
    }

    #[test]
    fn saved_items_and_synced_artists() {
        let lib = Library::open_in_memory().unwrap();
        let item = |id: &str, kind| Item {
            kind,
            id: id.into(),
            title: id.into(),
            subtitle: "".into(),
            thumbnail: None,
            track: None,
            explicit: false,
        };
        lib.save(&item("MPREb_a", ItemKind::Album)).unwrap();
        lib.save(&item("UClocal", ItemKind::Artist)).unwrap();
        lib.replace_subscriptions(&[
            item("UCsub1", ItemKind::Artist),
            item("UCsub2", ItemKind::Artist),
        ])
        .unwrap();
        lib.replace_subscriptions(&[item("UCsub2", ItemKind::Artist)])
            .unwrap();
        let artists: Vec<_> = lib
            .saved(ItemKind::Artist)
            .unwrap()
            .iter()
            .map(|a| a.id.to_string())
            .collect();
        assert!(
            artists.contains(&"UClocal".to_owned()),
            "followed here: kept"
        );
        assert!(artists.contains(&"UCsub2".to_owned()));
        assert!(
            !artists.contains(&"UCsub1".to_owned()),
            "unsubscribed: gone"
        );
        assert_eq!(lib.saved(ItemKind::Album).unwrap().len(), 1);
        assert!(lib.is_saved("MPREb_a").unwrap());
        lib.unsave("MPREb_a").unwrap();
        assert!(!lib.is_saved("MPREb_a").unwrap());
    }

    /// Downloads list in download order; which sound each holds is
    /// recorded.
    #[test]
    fn downloads_in_order_and_their_sound() {
        let lib = Library::open_in_memory().unwrap();
        for id in ["bbbbbbbbbbb", "aaaaaaaaaaa"] {
            lib.add_download(&Download {
                track: track(id, None),
                path: PathBuf::from(format!("/tmp/{id}.m4a")),
                bytes: 1,
            })
            .unwrap();
        }
        let order: Vec<_> = lib
            .downloads()
            .unwrap()
            .into_iter()
            .map(|d| d.track.video_id.to_string())
            .collect();
        assert_eq!(order, ["bbbbbbbbbbb", "aaaaaaaaaaa"], "download order");
        assert_eq!(lib.downloads_without_audio().unwrap().len(), 2);
        lib.set_download_audio("aaaaaaaaaaa", "aaaaaaaaaaa")
            .unwrap();
        assert_eq!(lib.downloads_without_audio().unwrap().len(), 1);
    }

    #[test]
    fn lists_kept_offline() {
        let lib = Library::open_in_memory().unwrap();
        let ids = |tracks: &[Track]| -> Vec<String> {
            tracks.iter().map(|t| t.video_id.to_string()).collect()
        };
        lib.replace_library(&[(
            Playlist {
                id: "PL1".into(),
                title: "Road trip".into(),
                item_count: 2,
                etag: None,
            },
            vec![track("bbbbbbbbbbb", None), track("aaaaaaaaaaa", None)].into(),
        )])
        .unwrap();
        let list = |id: &str| OfflineList {
            id: id.into(),
            title: "old name".into(),
            thumbnail: None,
        };
        lib.keep_offline(&list("PL1"), None).unwrap();
        // A page keeps its songs as they were.
        let album = [track("ccccccccccc", None), track("aaaaaaaaaaa", None)];
        lib.keep_offline(&list("MPREb_x"), Some(&album)).unwrap();
        assert!(lib.is_kept_offline("PL1").unwrap());
        let lists = lib.offline_lists().unwrap();
        assert_eq!(lists[0].title, "Road trip", "a playlist under its own name");
        assert_eq!(lists[1].id, "MPREb_x");
        assert_eq!(
            ids(&lib.offline_tracks("MPREb_x").unwrap()),
            ["ccccccccccc", "aaaaaaaaaaa"]
        );
        // Every missing song once, list by list, each in its own order.
        assert_eq!(
            ids(&lib.offline_missing().unwrap()),
            ["bbbbbbbbbbb", "aaaaaaaaaaa", "ccccccccccc"]
        );
        // A song added to the playlist is missing too, downloads aren't.
        lib.add_track("PL1", &track("ddddddddddd", None), false)
            .unwrap();
        lib.add_download(&Download {
            track: track("aaaaaaaaaaa", None),
            path: PathBuf::from("/tmp/a.m4a"),
            bytes: 1,
        })
        .unwrap();
        assert_eq!(
            ids(&lib.offline_missing().unwrap()),
            ["bbbbbbbbbbb", "ddddddddddd", "ccccccccccc"]
        );
        lib.forget_offline("MPREb_x").unwrap();
        lib.delete_playlist("PL1").unwrap();
        assert!(lib.offline_lists().unwrap().is_empty());
    }

    #[test]
    fn playlists_downloaded_whole_become_kept_offline() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE playlists (id TEXT PRIMARY KEY, title TEXT NOT NULL,
               item_count INTEGER NOT NULL, position INTEGER NOT NULL, etag TEXT);
             CREATE TABLE playlist_tracks (playlist_id TEXT NOT NULL, position INTEGER NOT NULL,
               video_id TEXT NOT NULL, item_id TEXT);
             CREATE TABLE downloads (video_id TEXT PRIMARY KEY, title TEXT NOT NULL,
               artist TEXT NOT NULL, duration_secs INTEGER, path TEXT NOT NULL,
               bytes INTEGER NOT NULL, downloaded_at INTEGER NOT NULL,
               explicit INTEGER NOT NULL DEFAULT 0);
             INSERT INTO playlists VALUES ('PLhalf', 'Half', 2, 0, NULL), ('PLfull', 'Full', 1, 1, NULL);
             INSERT INTO playlist_tracks VALUES ('PLhalf', 0, 'a', NULL), ('PLhalf', 1, 'b', NULL),
               ('PLfull', 0, 'a', NULL);
             INSERT INTO downloads VALUES ('a', 'a', 'A', NULL, '/a', 1, 0, 0);
             PRAGMA user_version = 5;",
        )
        .unwrap();
        let lib = Library::init(conn).unwrap();
        let kept: Vec<_> = lib
            .offline_lists()
            .unwrap()
            .into_iter()
            .map(|l| l.id)
            .collect();
        assert_eq!(kept, ["PLfull"]);
        assert_eq!(lib.downloads_without_audio().unwrap().len(), 1);
    }

    #[test]
    fn playlist_items_and_downloads() {
        let lib = Library::open_in_memory().unwrap();
        let mine = Playlist {
            id: "PL1".into(),
            title: "Mine".into(),
            item_count: 0,
            etag: None,
        };
        lib.replace_synced(&[SyncedPlaylist {
            playlist: mine,
            tracks: vec![track("aaaaaaaaaaa", None)].into(),
            item_ids: vec!["item-a".into()],
        }])
        .unwrap();
        assert_eq!(
            lib.item_id("PL1", "aaaaaaaaaaa").unwrap().as_deref(),
            Some("item-a")
        );
        lib.add_item("PL1", &track("bbbbbbbbbbb", None), false, Some("item-b"))
            .unwrap();
        assert_eq!(
            lib.item_id("PL1", "bbbbbbbbbbb").unwrap().as_deref(),
            Some("item-b")
        );

        lib.add_playlist(&Playlist {
            id: "PL2".into(),
            title: "New".into(),
            item_count: 0,
            etag: None,
        })
        .unwrap();
        lib.rename_playlist("PL2", "Renamed").unwrap();
        assert_eq!(lib.playlists().unwrap().last().unwrap().title, "Renamed");
        lib.delete_playlist("PL2").unwrap();
        assert_eq!(lib.playlists().unwrap().len(), 1);

        lib.add_download(&Download {
            track: track("ccccccccccc", Some(10)),
            path: "/tmp/c.m4a".into(),
            bytes: 1000,
        })
        .unwrap();
        assert_eq!(lib.downloads().unwrap().len(), 1);
        assert!(lib.download_path("ccccccccccc").unwrap().is_some());
        lib.remove_download("ccccccccccc").unwrap();
        assert!(lib.download_path("ccccccccccc").unwrap().is_none());
    }

    #[test]
    fn stream_cache_and_meta() {
        let lib = Library::open_in_memory().unwrap();
        lib.put_stream("aaaaaaaaaaa", "{}", 100).unwrap();
        assert!(lib.cached_stream("aaaaaaaaaaa", 50).unwrap().is_some());
        assert!(lib.cached_stream("aaaaaaaaaaa", 100).unwrap().is_none());
        lib.purge_expired_streams(200).unwrap();
        assert!(lib.cached_stream("aaaaaaaaaaa", 0).unwrap().is_none());
        lib.set_meta("volume", "0.5").unwrap();
        assert_eq!(lib.get_meta("volume").unwrap().as_deref(), Some("0.5"));
    }
}
