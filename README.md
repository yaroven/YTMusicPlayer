# ytm-player

A lightweight terminal music player for your YouTube Music library.
macOS, Linux and Windows.

- Your liked music and playlists, synced via the official YouTube Data API.
- Streams audio through [yt-dlp](https://github.com/yt-dlp/yt-dlp) (downloaded
  automatically on first run if it isn't installed).
- ~20–25 MB RAM while playing, near-zero CPU. Single 8 MB binary.

```
┌ Library ──────────────┐┌ Liked music · 120 ──────────────────────────────┐
│› Liked music 120      ││#     Title                     Artist      Time │
│  Road trip 42         ││   ▶  Never Gonna Give You Up   Rick Astley 3:33 │
│  Focus 87             ││   2  Despacito                 Luis Fonsi  4:41 │
└───────────────────────┘└─────────────────────────────────────────────────┘
┌ ▶ Never Gonna Give You Up — Rick Astley ─────────────────────── vol  80% ┐
│0:22 / 3:33 ━━━━━━━━━───────────────────────────────────────────────────── │
└──────────────────────────────────────────────────────────────────── 1/120 ┘
```

## Build

Requires Rust 1.85+ ([rustup](https://rustup.rs)) plus a C toolchain
(SQLite is compiled in).

| OS | Install first |
|---|---|
| macOS | `xcode-select --install` |
| Debian/Ubuntu | `sudo apt install build-essential pkg-config libasound2-dev` |
| Fedora | `sudo dnf install gcc pkgconf-pkg-config alsa-lib-devel` |
| Arch | `sudo pacman -S base-devel alsa-lib` |
| Windows | Visual Studio Build Tools, workload "Desktop development with C++" |

```bash
cargo install --path .     # installs the `ytm` binary
```

Linux at runtime also needs a Secret Service provider (GNOME Keyring or
KWallet) to store the sign-in; PulseAudio/PipeWire work through ALSA.

## Set up Google sign-in (once, ~5 minutes)

The app reads your library with your own Google Cloud OAuth client, so no
third party ever sees your account.

1. Open <https://console.cloud.google.com/>, create a project.
2. **APIs & Services → Library**: enable **YouTube Data API v3**.
3. **Google Auth Platform → Branding**: fill in app name and your email.
   **Audience**: user type *External*, then **Publish app** (status
   *In production*). Without publishing, Google expires the sign-in every
   7 days. No verification is needed for personal use.
4. **Clients → Create client**: type **Desktop app**. Copy the client ID and
   client secret.
5. Put them into the config file (`ytm config` prints its path):

   ```toml
   client_id = "1234567890-abc.apps.googleusercontent.com"
   client_secret = "GOCSPX-..."
   ```

6. Run `ytm login`. The browser opens; Google warns the app is unverified —
   choose **Advanced → Go to … (unsafe)**, since it is your own app. Access is
   read-only (`youtube.readonly`).

## Use

```bash
ytm                 # open the player; first start syncs your library
ytm sync            # refresh the library
ytm play <url|id>   # play a single track without the UI
ytm logout
```

| Key | Action |
|---|---|
| `↑/↓` `k/j` | move |
| `Tab` `h/l` | switch pane |
| `Enter` | open playlist / play track |
| `Space` | play / pause |
| `n` / `p` | next / previous |
| `←` / `→` | seek ∓5 s |
| `+` / `-` | volume |
| `r` | sync library |
| `?` | help |
| `q` | quit |

## Configuration

`config.toml` (see `ytm config`) is created with comments on first run:

| Key | Default | Meaning |
|---|---|---|
| `client_id`, `client_secret` | — | Google OAuth client (or `YTM_CLIENT_ID` / `YTM_CLIENT_SECRET`) |
| `js_fallback` | `true` | if yt-dlp fails, retry with a JS runtime (system deno/node, or a ~2 MB QuickJS download). `false` = never run JS |
| `ytdlp_extra_args` | `[]` | e.g. `["--cookies-from-browser", "firefox"]` for age-restricted tracks |
| `liked_music_only` | `true` | keep only "Music"-category videos in the liked list |
| `volume` | `0.8` | startup volume |

Logs: the OS cache directory under `ytm-player/logs` (`YTM_LOG=debug` for more).

## Notes and limits

- The "Liked music" list is your liked YouTube videos filtered to the Music
  category; the API has no exact YouTube Music likes list.
- Audio is AAC ~128 kbps (Opus isn't decodable by the audio stack yet).
- API quota: 10,000 units/day; a sync costs about 1 unit per 50 tracks.
  Browsing and playing never use quota.
- Each track's audio is held in memory while it plays (~1 MB per minute).
