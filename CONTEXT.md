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
- **Catalog**: YouTube Music's own (quota-free) web endpoints: search by
  category, Pages, home Shelves, radio, Related, lyrics, loudness
  (`catalog`).
- **Item**: a catalog entry — song, album, artist or playlist — shown as a
  card; songs carry a playable Track.
- **Page**: an album, artist or playlist opened from an Item: Tracks plus
  **Shelves** (titled rows of Items).
- **Search results**: the last online search in one category. Shown like a
  Playlist (songs) or as cards until another list is picked.
- **Library view**: what the list shows (a Playlist, Search results, a
  Page, Home, History, Downloads, saved albums or followed artists), the
  filter, the selected row, the action target and the way **Back**. One
  per frontend, same rules for both (`library_view`).
- **Radio**: YouTube Music's endless mix seeded by a song. **Autoplay**
  queues the radio of the last song when the Queue runs out.
- **Preload**: the next track opened shortly before the current one ends,
  so it starts gapless or crossfaded.
- **Listener**: something told about playback (Last.fm scrobbler, Discord
  presence).
- **Cast player**: the Playback adapter for a Chromecast; the Session swaps
  it in for the local player and back.
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
