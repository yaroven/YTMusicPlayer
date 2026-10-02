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

const SCHEMA_VERSION: i32 = 3;
/// Plays kept in the history (older ones are dropped).
const HISTORY_CAP: i64 = 500;

/// A synced playlist with its tracks and, per track, the API's playlist
/// item id (needed to remove it again; empty when unknown).
pub struct SyncedPlaylist {
    pub playlist: Playlist,
    pub tracks: Arc<[Track]>,
    pub item_ids: Vec<String>,
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
            "SELECT t.video_id, t.title, t.artist, t.duration_secs
             FROM playlist_tracks pt JOIN tracks t ON t.video_id = pt.video_id
             WHERE pt.playlist_id = ?1 ORDER BY pt.position",
        )?;
        let rows = stmt.query_map([playlist_id], |r| {
            Ok(Track {
                video_id: r.get::<_, String>(0)?.into(),
                title: r.get::<_, String>(1)?.into(),
                artist: r.get::<_, String>(2)?.into(),
                duration_secs: r.get(3)?,
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

    pub fn delete_playlist(&self, id: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM playlists WHERE id = ?1", [id])?;
        Ok(())
    }

    // --- history -------------------------------------------------------------------

    /// Records a play (kept: the last [`HISTORY_CAP`]).
    pub fn add_history(&self, track: &Track) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO history (video_id, title, artist, duration_secs, played_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                &*track.video_id,
                &*track.title,
                &*track.artist,
                track.duration_secs,
                chrono::Utc::now().timestamp()
            ],
        )?;
        conn.execute(
            "DELETE FROM history WHERE id <= (SELECT MAX(id) FROM history) - ?1",
            [HISTORY_CAP],
        )?;
        Ok(())
    }

    /// Played tracks, most recent first, each once.
    pub fn history(&self) -> Result<Arc<[Track]>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT video_id, title, artist, duration_secs FROM history
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

    /// Replaces the followed artists (from YouTube subscriptions), keeping
    /// ones followed only here.
    pub fn replace_subscriptions(&self, artists: &[Item]) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM saved WHERE kind = 'artist' AND synced = 1", [])?;
        for (i, a) in artists.iter().enumerate().rev() {
            tx.execute(
                "INSERT INTO saved (id, kind, title, subtitle, thumbnail, position, synced)
                 VALUES (?1, 'artist', ?2, ?3, ?4, ?5, 1)
                 ON CONFLICT(id) DO UPDATE SET synced = 1",
                params![
                    &*a.id,
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
            "INSERT INTO downloads (video_id, title, artist, duration_secs, path, bytes, downloaded_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(video_id) DO UPDATE SET path = excluded.path, bytes = excluded.bytes",
            params![
                &*t.video_id,
                &*t.title,
                &*t.artist,
                t.duration_secs,
                download.path.to_string_lossy(),
                download.bytes as i64,
                chrono::Utc::now().timestamp()
            ],
        )?;
        Ok(())
    }

    pub fn remove_download(&self, video_id: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM downloads WHERE video_id = ?1", [video_id])?;
        Ok(())
    }

    /// Downloaded tracks, newest first.
    pub fn downloads(&self) -> Result<Vec<Download>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT video_id, title, artist, duration_secs, path, bytes FROM downloads
             ORDER BY downloaded_at DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(Download {
                track: track_row(r)?,
                path: PathBuf::from(r.get::<_, String>(4)?),
                bytes: r.get::<_, i64>(5)? as u64,
            })
        })?;
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

fn track_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Track> {
    Ok(Track {
        video_id: r.get::<_, String>(0)?.into(),
        title: r.get::<_, String>(1)?.into(),
        artist: r.get::<_, String>(2)?.into(),
        duration_secs: r.get(3)?,
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
        "INSERT INTO tracks (video_id, title, artist, duration_secs) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(video_id) DO UPDATE SET title = excluded.title,
           artist = excluded.artist,
           duration_secs = COALESCE(excluded.duration_secs, tracks.duration_secs)",
    )?
    .execute(params![
        &*t.video_id,
        &*t.title,
        &*t.artist,
        t.duration_secs
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
    fn saved_items_and_synced_artists() {
        let lib = Library::open_in_memory().unwrap();
        let item = |id: &str, kind| Item {
            kind,
            id: id.into(),
            title: id.into(),
            subtitle: "".into(),
            thumbnail: None,
            track: None,
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
