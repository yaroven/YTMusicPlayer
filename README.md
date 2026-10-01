# ytm-player

A lightweight terminal music player for your YouTube Music library.
macOS, Linux and Windows.

- Your liked music and playlists, synced via the official YouTube Data API.
- Streams audio through [yt-dlp](https://github.com/yt-dlp/yt-dlp) (downloaded
  automatically on first run; the unpacked "onedir" build starts in ~2 s
  instead of ~7 s for the single-file one).
- ~10 MB of memory idle, ~15 MB while playing (macOS, Activity Monitor
  "Memory"), near-zero CPU. Memory doesn't grow with track length.
- Search, shuffle/repeat, queue, like and add-to-playlist, mouse support,
  media keys and the system "Now Playing" widget (macOS, Linux).
- Two interfaces: the terminal UI (default) or a desktop window
  (`ytm gui`, or `ui = "gui"` in the config).

```
┌ Library ──────────────┐┌ Liked music · 120 ──────────────────────────────┐
│› Liked music 120      ││#      Title                     Artist     Time │
│  Road trip 42         ││    ▶  Never Gonna Give You Up   Rick Astley 3:33│
│  Focus 87             ││    2  Despacito                 Luis Fonsi  4:41│
└───────────────────────┘└─────────────────────────────────────────────────┘
┌ ▶ Never Gonna Give You Up — Rick Astley ──── shuffle repeat vol  80% ┐
│0:22 / 3:33 ━━━━━━━━━──────────────────────────────────────────────────── │
└──────────────────────────────────────────────────────────────────── 1/120 ┘
```

## Install

Linux, macOS:

```bash
git clone https://github.com/yaroven/YTMusicPlayer.git
cd YTMusicPlayer
./install.sh
```

Windows (PowerShell):

```powershell
git clone https://github.com/yaroven/YTMusicPlayer.git
cd YTMusicPlayer
powershell -ExecutionPolicy Bypass -File .\install.ps1
```

The installers download a prebuilt binary from GitHub Releases when one
exists for your platform (Debian/Ubuntu x86_64, macOS; Windows builds from
source) (the repository is private, so this uses the
[GitHub CLI](https://cli.github.com/) login), and otherwise build from source,
installing build dependencies and Rust as needed. They then import your Google
OAuth client (see below), sign you in and start the player. Re-run to update.

The installer also adds the player to your applications: a menu entry with
an icon on Linux, and `~/Applications/ytm-player.app` on macOS (Launchpad,
Spotlight, Dock; it shows up as "ytm-player" in Activity Monitor). Re-create
just that with `./install.sh --launcher-only`. Copies left by earlier
installs (another `ytm` on PATH, an older `ytm-player.app`) are removed, so
one binary and one app remain.

`install.sh` options: `--yes`, `--from-source`, `--no-deps`, `--no-launch`,
`--launcher-only`.
`install.ps1` options: `-Yes`, `-FromSource`, `-NoLaunch`.

### Build manually

Requires Rust 1.85+ ([rustup](https://rustup.rs)) plus a C toolchain
(SQLite is compiled in).

| OS | Install first |
|---|---|
| macOS | `xcode-select --install` |
| Debian/Ubuntu | `sudo apt install build-essential pkg-config libasound2-dev libfontconfig1-dev` |
| Fedora | `sudo dnf install gcc pkgconf-pkg-config alsa-lib-devel fontconfig-devel` |
| Arch | `sudo pacman -S base-devel alsa-lib fontconfig` |
| Windows | Visual Studio Build Tools, workload "Desktop development with C++" |

```bash
cargo install --path . --locked                       # terminal + window
cargo install --path . --locked --no-default-features  # terminal only
```

Linux at runtime also needs a Secret Service provider (GNOME Keyring or
KWallet) to store the sign-in. Sound goes to PipeWire or PulseAudio through
their ALSA plugins, picked automatically.

## Set up Google sign-in (once, ~5 minutes)

The app uses your own Google Cloud OAuth client, so no third party ever sees
your account.

1. Open <https://console.cloud.google.com/>, create a project.
2. **APIs & Services → Library**: enable **YouTube Data API v3**.
3. **Google Auth Platform → Branding**: app name and your email.
   **Audience**: user type *External*, and add your Google account under
   **Test users**.
4. **Clients → Create client**: type **Desktop app**. In the dialog, click
   **Download JSON** (the secret is shown only once).
5. Import it: `ytm import-client ~/Downloads/client_secret_….json`
   (the installers offer this automatically).
6. Run `ytm login`. Google warns the app is unverified — choose
   **Continue**, since it is your own app. The app asks for permission to
   manage your YouTube account so it can like tracks and add them to
   playlists; it never deletes anything.

While the app is in *Testing* status, Google expires the sign-in after 7 days;
the player then says so and `ytm login` renews it. Publishing the app
(**Audience → Publish app**) removes the limit but requires a public home
page and privacy policy.

**Signing in over SSH / without a browser:** create a second client of type
**TVs and Limited Input devices**, import it with
`ytm import-client <json> --device`, then `ytm login --device` shows a code
to approve from any phone or computer.

## Use

```bash
ytm                         # open the player; first start syncs your library
ytm sync                    # refresh the library (unchanged playlists are skipped)
ytm play <url|id>           # play a single track without the UI
ytm status                  # config, sign-in and library state
ytm devices                 # audio outputs (for `audio_device`)
ytm import-client <json>    # set the OAuth client from Google's JSON
ytm login [--device] | ytm logout
```

### Desktop window

`ytm gui` opens the same player in a window styled after YouTube Music:
library on the left, search box, playlist header with Play / Shuffle,
track list (double-click to play), and a player bar with a red progress
line, previous/play/next, the current track with a like button, volume,
repeat, shuffle, play next and save to playlist. Album art comes from the
YouTube thumbnails, cropped to a square and fetched only for rows on screen
(a small bounded cache). Click the playing track or ▲ to open the full-screen
"now playing" view (big cover, "Up next" queue; Esc or ▼ closes it). The
layout adapts to the window: narrower sidebar, then no sidebar (a playlist
picker instead) and icon-only buttons on small windows. Keys: `Space` play/pause, `n`/`p`
next/previous, `f` like, `/` filter. Set `ui = "gui"` to make it the
default.

Only one player runs at a time: launching it again brings the open window
to the front. The status bar shows the player's memory use, plus yt-dlp's
while it is fetching a track link (it runs as a short-lived child process,
one at a time).

The window costs more memory than the terminal UI (~55–70 MB on macOS vs
~15 MB; most of it is the window's pixel buffers, so it grows with window
size). Builds without the `gui` cargo feature are terminal-only.

### Terminal keys

| Key | Action |
|---|---|
| `↑/↓` `k/j`, `PgUp/PgDn`, `g/G` | move |
| `Tab` `h/l` | switch pane |
| `Enter`, double-click | open playlist / play track |
| `Space` | play / pause |
| `n` / `p` | next / previous (restarts the track if > 3 s in) |
| `←` / `→`, click the bar | seek |
| `+` / `-` | volume |
| `/` | filter tracks (`Esc` clears) |
| `s` / `e` | shuffle / repeat (off → all → one) |
| `v` | show the queue |
| `u` | play the selected track next |
| `f` | like / unlike |
| `a` | add to a playlist |
| `r` | sync library |
| `?` | help |
| `q` | quit |

Volume, shuffle, repeat and the selected playlist are remembered.

## Configuration

`config.toml` (see `ytm config`) is created with comments on first run:

| Key | Default | Meaning |
|---|---|---|
| `client_id`, `client_secret` | — | Google OAuth client (or `YTM_CLIENT_ID` / `YTM_CLIENT_SECRET`) |
| `device_client_id`, `device_client_secret` | — | optional TV-type client for `ytm login --device` |
| `ui` | `"tui"` | what `ytm` opens: `"tui"` or `"gui"` |
| `media_controls` | `true` | media keys and system "Now Playing" (macOS, Linux) |
| `audio_device` | `""` | output device name from `ytm devices`; empty = automatic (Linux: PipeWire, then PulseAudio, then ALSA default) |
| `js_fallback` | `true` | if yt-dlp fails, retry with a JS runtime (system deno/node, or a ~2 MB QuickJS download). `false` = never run JS |
| `ytdlp_extra_args` | `[]` | e.g. `["--cookies-from-browser", "firefox"]` for age-restricted tracks |
| `liked_music_only` | `true` | keep only "Music"-category videos in the liked list |
| `volume` | `0.8` | volume for the very first start |

Logs: the OS cache directory under `ytm-player/logs` (`YTM_LOG=debug` for more).

## Releases

Push a tag to build binaries for Debian/Ubuntu (x86_64, Debian 12+) and
macOS (Apple Silicon, Intel) and publish them as a GitHub Release:

```bash
git tag v0.2.0 && git push origin v0.2.0
```

## Notes and limits

- "Liked music" comes from YouTube Music's own liked list (`LM`) when the API
  returns it; otherwise from your liked YouTube videos, filtered to the Music
  category unless `liked_music_only = false`. The sync message says which.
- Audio is AAC ~128 kbps. Opus would need a C library (libopus via cmake) on
  every platform for no audible gain, so it's not used.
- API quota: 10,000 units/day. A sync costs ~1 unit per 50 tracks, and
  playlists that didn't change are skipped; a like or add costs 50.
  Browsing and playing never use quota.
- Media keys aren't supported on Windows yet (they need a window handle).
