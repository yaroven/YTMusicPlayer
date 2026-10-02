# Domain glossary

Names used in code, docs and reviews. Architecture vocabulary (module,
interface, seam, adapter, depth) follows the codebase-design glossary.

- **Library**: the user's synced YouTube Music data in SQLite: Playlists,
  their Tracks, plus cached Media URLs and small UI state (`storage`).
- **Playlist**: one of the user's YouTube playlists, or **Liked music**
  (pseudo id `__liked__`, always first). Liked music is never a "save to
  playlist" target: liking is how tracks get into it.
- **Track**: one video in a Playlist, Search results or the Queue
  (`Arc<str>` fields; shared, never copied per view).
- **Search results**: the last online search (YouTube Data API, or yt-dlp
  without sign-in). Shown like a Playlist until another list is picked.
- **Library view**: what the track list shows (a Playlist or Search
  results), the filter typed over it, the selected row, and the action
  target. One per frontend, same rules for both (`library_view`).
- **Action target**: the Track that like / play next / save to playlist
  apply to: the selected row, else the playing Track. The player bar's
  heart always belongs to the playing Track.
- **Queue**: the play order over a shared track list, plus "play next"
  items; shuffle and repeat live here (`audio::queue`).
- **Session**: the UI-independent player core: Queue, playback, background
  work, media keys; reports **Changes** to the frontends (`session`).
- **Track source**: turns a Track into a playable, bounded audio stream and
  owns the **Media URL** lifecycle: resolve, cache, refresh on 403/410,
  JS fallback, yt-dlp updates (`audio::source`).
- **Media URL**: the time-limited googlevideo URL yt-dlp resolves for a
  Track (~6 h); may be rejected mid-track and is then replaced.
- **Extractor**: whatever resolves video ids into Media URLs — yt-dlp in
  the app.
- **Account**: the user's own Google **OAuth client** (Desktop app, optional
  device client), the **sign-in** (stored token) and the YouTube API client
  built from them (`account`).
- **OAuth client**: client ID + secret the user creates in Google Cloud; in
  `config.toml`.
- **Sign-in**: browser flow (loopback) or device flow (code); the token
  lives in the OS keyring. Google expires it after 7 days while the app is
  in "Testing".
