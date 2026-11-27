# How the player works inside

A map of the program for anyone changing it (or curious what happens when
they press a button): the shape of the thing first, then the paths through
it, one per user action. File names are the Rust modules under `src/`;
`CLAUDE.md` holds the decisions behind them and `CONTEXT.md` the vocabulary.

## The shape of it

```
                 ┌───────────────────┐
                 │      window       │   ios/, android/ (separate apps)
                 │       gui::       │
                 └─────────┬─────────┘
                           │ Cmd / Snapshot
                           ▼
                 ┌───────────────────┐
                 │     session       │  queue, playback, background work,
                 │ (UI-independent)  │  status, listeners, media keys
                 └─┬───────┬───────┬─┘
      ┌────────────┘       │       └─────────────┐
      ▼                    ▼                     ▼
┌───────────┐     ┌────────────────┐     ┌──────────────┐
│ catalog   │     │ audio::source  │     │  storage     │
│ InnerTube │     │ yt-dlp, stream │     │  SQLite      │
└───────────┘     └────────────────┘     └──────────────┘
```

- **One core, thin frontend.** `session::Session` owns everything that isn't
  pixels: the queue, the player, the library view's data, background tasks.
  The window (`gui`) drives its methods and renders `session.view()`; the
  CLI commands use the same pieces.
- **Nothing blocks the UI.** Every network or disk call runs in a tokio task
  that ends by sending a `Background` event; `Session::next_event()` folds
  the event into the state and returns a `Changes` struct saying what to
  redraw.
- **The window never renders from the core thread.** `gui` runs the session
  on a "core" thread with its own runtime; the UI sends `Cmd`s to it and the
  core pushes `Snapshot`s back through `upgrade_in_event_loop`.
- **Memory is a feature.** Lists hold `Arc<[Track]>`, rows are built only for
  what's visible, the stream keeps 2 MiB at most, art caches are capped, and
  the window's own pixel buffers dominate what's left (see `CLAUDE.md`).

## Where things live

| Data | Where |
|---|---|
| Library, history, downloads, stream URLs, UI state | `<data_dir>/library.sqlite3` (`storage`) |
| Settings | `config.toml` (`config::settings`) |
| YouTube sign-in (cookies) | OS keyring + `<data_dir>/youtube-cookies.txt` (0600) for yt-dlp |
| OAuth tokens (optional Data API) | OS keyring |
| Downloaded songs | `<data_dir>/downloads/<id>.m4a` |
| Logs | `<cache_dir>/logs/yarek.log` |

## Starting up

1. `main` reads `config.toml`, opens the window (or runs a CLI command),
   and takes the single-instance lock (`instance`): a
   second launch just raises the first window and exits.
2. `bootstrap::deps` builds the HTTP client, the library, the YouTube session
   (keyring), yt-dlp's handle (without probing it — that costs seconds), the
   track source and the account.
3. `Session::startup` shows the stored library immediately and, when signed
   in and the library is empty, starts a sync. yt-dlp updates itself in the
   background at most once a day.

## Playing a song

1. The frontend calls `session.play(tracks, index)`. The queue
   (`audio::queue`) keeps the source list as `Arc<[Track]>` plus a play order
   for shuffle.
2. `audio::source::TrackSource::open` looks for a downloaded file, then the
   URL cache (16 in memory, the rest in SQLite), then the resolver's ladder
   (`audio::resolver`, the only place that decides what a failure means):
   - `audio::innertube` asks YouTube's player endpoint as the VISIONOS
     client with a `visitorData` (the phones' resolver): ~0.1 s, no process;
   - when YouTube refuses (bot check, age check, no plain AAC format), yt-dlp
     — one at a time, because each run costs ~90 MB while it lives — plain,
     then with a JS runtime, then after `yt-dlp -U`;
   - a network failure stops the ladder at once: yt-dlp would fail the same
     way, seconds later. The session waits for the connection instead.

   An InnerTube URL the media server rejects (403) sends that song to
   yt-dlp; three such songs within 10 minutes switch InnerTube off until
   they are older.
3. Playback is a bounded stream over googlevideo: 256 KiB range requests, 4
   ahead, 8 cached. A mid-track 403 (YouTube expiring the URL) asks the
   resolver for a fresh one and continues at the same byte.
4. `audio::player` decodes with rodio on its own thread. The next track is
   opened early and appended within 2 s of the end (gapless) or played over
   the tail (crossfade).
5. Every finished track goes to history; `Listener`s (Last.fm, Discord,
   notifications) see the same events.

**Age-restricted songs** need the YouTube sign-in: the retry runs yt-dlp with
cookies and a JS runtime and takes YouTube's HLS variant, whose MPEG-TS
segments carry the same AAC as the normal format; `audio::hls` demuxes them
into an `.aac` file (no ffmpeg) and the player treats it as a local file.

## Searching and browsing

Search, pages, home, Explore, radio, related and lyrics all go to YouTube
Music's own InnerTube endpoints (`catalog`): no quota, no sign-in needed,
and the answers are parsed leniently — unknown renderers are skipped, never
an error. Suggestions are debounced 250 ms. Signed in, the same calls are
signed with the account's cookies, so Home and the library are the user's
own.

## The library (signed in)

- **Sync** (`sync::sync_with_session`): liked music (`VLLM`), the account's
  playlists with each entry's `setVideoId`, saved albums and followed
  artists, written into SQLite in one transaction. It runs after signing in,
  on the first start with an empty library, and when asked — never on a
  timer.
- **Writes** go the same way (`session::library::Writer`): like / dislike,
  add to or remove from a playlist, create / rename / delete a playlist,
  follow an artist. The local copy is updated when YouTube accepts.
- Without the YouTube sign-in the older Data API path still works for
  accounts that set up an OAuth client; it costs quota and Google expires it
  every 7 days.

## Lyrics

`lyrics` asks LRCLIB for the song (exact match by duration first, then a
search, retried on 5xx), and falls back to YouTube Music's plain text. Word
times come from Enhanced LRC when the file has them, else they're spread by
word length. The window lights the sung line word by word from a 60 ms
timer; the phone does the same at 10 Hz.

## Downloads

The download button (or the row menu) saves the audio into
`<data_dir>/downloads`: the song's URL comes from the same ladder, and a
plain m4a file is fetched in 1 MiB ranges (googlevideo throttles one big
read to the song's bitrate) and written as it arrives; yt-dlp downloads only
what isn't a plain file (HLS, age-restricted). A batch is one task: **Stop**
aborts it (and a running yt-dlp with it), and the half-written file is
cleaned up along with anything else in the folder the library doesn't know.
**Delete all** removes the files and the records; one song goes with its
download button again or the row menu.

## Watching the clip

**Our own player** (`clip`, Linux for now): the switch over the cover (♪ /
▶, or `V`) shows the clip's picture in the video area while the studio song
keeps playing. A thread of its own asks InnerTube for the clip's HLS master
playlist (VISIONOS: H.264 + AAC up to 1080p, every segment answers),
takes the H.264 variant that fits the area (switching at a segment boundary
when the window grows or shrinks), fetches the MPEG-TS segments one ahead,
pulls the H.264 access units and their PTS out with our TS demuxer and
decodes them with the system's FFmpeg (`clip::ffmpeg`, `dlopen`ed: no
link-time dependency) into RGB pictures scaled to the area; three wait in
a queue. `gui::clip_view`'s 15 ms timer shows the one due at the song's
position plus the clip's intro offset, from the same snapshot clock the
lyrics use; a jump in the position is a seek. Without the libraries, or
when YouTube has no clip for it, the old ways below apply.

**Elsewhere** (macOS, Windows, until their decoders are written) the switch
pauses the audio and puts the system web view — WebKit on macOS, WebView2 on Windows — in the
window as a **child view** over the cover's place (`gui::video`): Slint keeps
the layout, the system draws the video into the rectangle Slint reports. The
page is YouTube's watch page, stripped to the player by an injected script
that also reports `currentTime`; switching back destroys the view (its
WebContent processes go too) and seeks the audio to where the video stopped.
Linux without FFmpeg's libraries opens it in the browser.

## Signing in

"Sign in with Google" runs Google's own page in the system web view inside a
**child process** (`yarek sign-in-window`), so the player itself never loads a
browser engine for it. When youtube.com has a session the child prints the
cookies; they go to the OS keyring and to a 0600 cookies.txt for yt-dlp.
Linux imports a browser's sign-in instead (`yarek sign-in --from firefox`).

## Casting

`cast` finds Chromecasts with a hand-rolled mDNS query and speaks Cast v2
over TLS. Picking a device swaps the `Playback` implementation under the
session: the same queue, now driven by the receiver, which gets the
googlevideo URL directly.

## Updating

At startup the player asks GitHub for the latest release, compares versions
and, if the user agrees, replaces itself the way it was installed: in place
(macOS bundles are re-signed ad hoc), through the `.pkg` installer, the
Windows silent setup, or by printing what to run for Linux packages.

## The iPhone app

`ios/YTMKit` is the same catalog and lyrics logic ported to Swift, plus a
resolver that gets stream URLs on the device (no yt-dlp). `ios/App` is the
SwiftUI player: AVQueuePlayer with the next song resolved ahead, the library
synced from and written back to the account, karaoke lyrics, and the queue
saved as JSON. CI builds the unsigned `.ipa`; SideStore installs it.

## The Android app

`android/core` is the same catalog, lyrics and resolver logic in plain
Kotlin (it runs and is tested on the desktop JVM: `./gradlew :core:test`);
`android/app` is the Compose UI and the player. The whole queue sits in
ExoPlayer's playlist as `yarek://<video id>` items; a `ResolvingDataSource`
turns each into its googlevideo URL (the resolver runs on ExoPlayer's
loader thread) when ExoPlayer starts loading it. So preloading the next
song, gapless changes and the notification's next / previous are
ExoPlayer's own. A song that only plays as HLS (age-restricted, signed in)
fails its first load with `NeedsHls` and is put back as an HLS item.
Crossfade starts a short-lived second player on the old song a moment
ahead and lets the main one move on, so the media session never switches
players. `PlaybackService` only puts the session in the notification.

## No internet, bad internet

Every frontend tells a network failure apart from other errors and acts on
it instead of showing the error chain:

- **Desktop** (`net`): `net::problem` reads reqwest / io errors and yt-dlp's
  messages ("Unable to download webpage", "nodename nor servname"…) as
  *offline* or *slow*. The session then marks itself `offline` (the window
  shows a banner with Retry) and
  probes YouTube Music with a TCP connect every 4 s. A song that couldn't
  be opened, or that stopped well before its end for want of data, waits
  (`wait_for_network`, at most 5 times in a row) instead of being skipped;
  when the probe succeeds it continues at the same position, and Home,
  Explore or the page that failed load again.
- **iPhone** (`Connectivity`, NWPathMonitor) and **Android** (`Network`,
  ConnectivityManager's default-network callback) know the connection
  state directly: a banner, error screens with Retry that reload by
  themselves on reconnect, a "slow connection" hint on long loads and long
  buffering, offline taps that play only downloaded songs, and a player
  that waits and resumes in place.

## Testing it

- `cargo test` — unit tests and window tests
  on Slint's headless backend.
- `cargo test -- --ignored` — the ones that hit the network (nightly CI runs
  them).
- `ios/player-check.sh`, `swift run YTMKitChecks [--live]` — the phone's core
  and player without Xcode.
- `android/`: `./gradlew :core:test` (`YTM_LIVE=1` for the live half), and
  the app on an emulator (`emulator -avd … -no-window`, `adb shell svc wifi
  disable` / `svc data disable` for the offline paths).
- Manual: real audio, memory (`footprint`), macOS-only behaviour.
