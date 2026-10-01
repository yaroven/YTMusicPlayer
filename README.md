# ytm-player

A lightweight player for your YouTube Music library — a desktop window styled
after YouTube Music, and a terminal UI. macOS, Linux and Windows.

- **Your library:** liked music and playlists, synced through the official
  YouTube Data API with your own Google OAuth client (no third party sees
  your account). Likes and "save to playlist" go back to YouTube.
- **Search YouTube Music**, play, like and save results like any list.
- **Light:** the terminal UI uses ~10 MB of memory idle and ~15 MB while
  playing, the window ~55–75 MB (macOS, Activity Monitor "Memory");
  near-zero CPU. A YouTube Music browser tab typically takes several hundred MB.
- **Desktop window:** album art, full-screen "now playing" with the queue,
  resizable sidebar, layout that adapts to small windows.
- **Terminal UI:** keyboard and mouse, works over SSH.
- Shuffle / repeat / play next, media keys and the system "Now Playing"
  widget (macOS, Linux), one player instance at a time.
- Audio comes from [yt-dlp](https://github.com/yt-dlp/yt-dlp), which the
  app downloads and keeps updated by itself.

```
┌ Library ──────────────┐┌ Liked music · 120 ──────────────────────────────┐
│› Liked music 120      ││#      Title                     Artist     Time │
│  Road trip 42         ││    ▶  Never Gonna Give You Up   Rick Astley 3:33│
│  Focus 87             ││    2  ♥ Despacito               Luis Fonsi  4:41│
└───────────────────────┘└─────────────────────────────────────────────────┘
┌ ▶ Never Gonna Give You Up — Rick Astley ──── shuffle repeat vol  80% ┐
│0:22 / 3:33 ━━━━━━━━━──────────────────────────────────────────────────── │
└──────────────────────────────────────────────────────────────────── 1/120 ┘
```

## Quick start

1. **Install** the package for your system from the
   [latest release](https://github.com/yaroven/YTMusicPlayer/releases/latest)
   (see [Install](#install)).
2. **Create a Google OAuth client** once (~5 minutes, free):
   [Set up Google sign-in](#set-up-google-sign-in-once-5-minutes).
3. **Open ytm-player** from your applications. The **Google account** dialog
   opens by itself: click **Import downloaded JSON** (or paste the client ID
   and secret), then **Sign in with Google**. Your library syncs and you can
   play.

## Install

### Installers

Download the file for your system from the
[latest release](https://github.com/yaroven/YTMusicPlayer/releases/latest).
The repository is private: sign in to GitHub in the browser first, or use
`gh release download --repo yaroven/YTMusicPlayer --pattern '<file>'`.

| System | File | Install |
|---|---|---|
| Debian, Ubuntu, Mint | `ytm-player_<version>_amd64.deb` | `sudo apt install ./ytm-player_*_amd64.deb` |
| Fedora | `ytm-player-<version>-1.x86_64.rpm` | `sudo dnf install ./ytm-player-*.x86_64.rpm` |
| Arch, Manjaro | `ytm-player-<version>-1-x86_64.pkg.tar.zst` | `sudo pacman -U ./ytm-player-*.pkg.tar.zst` |
| Windows 10/11 | `ytm-player-<version>-setup-x64.exe` | run it (no admin rights needed) |
| macOS 11+ (Apple Silicon and Intel) | `ytm-player-<version>-macos.pkg` | open it |

Each one adds **ytm-player** to your applications (opens the window) and the
`ytm` command (terminal UI and the [commands](#commands)). Linux packages
need glibc 2.35+ (Debian 12, Ubuntu 22.04, current Fedora and Arch).

The installers aren't signed with a paid developer certificate, so the
system asks once:

- **macOS**: "Apple could not verify…" → **System Settings → Privacy &
  Security → Open Anyway**, then the installer runs normally.
- **Windows**: SmartScreen "Windows protected your PC" → **More info →
  Run anyway**.

To update, install the newer file over the old one.

### Install scripts (from the repository)

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

The scripts download a prebuilt binary from the latest release (through the
[GitHub CLI](https://cli.github.com/) login, since the repository is
private), or build from source when there is none for your platform,
installing build dependencies and Rust as needed. They add the app to your
applications (Linux menu entry, `~/Applications/ytm-player.app` on macOS),
remove older copies so only one remains, import your Google OAuth client,
sign you in and start the player. Re-run to update.

`install.sh` options: `--yes`, `--from-source`, `--no-deps`, `--no-launch`,
`--launcher-only` (only re-create the app entry).
`install.ps1` options: `-Yes`, `-FromSource`, `-NoLaunch`.

### Build manually

Requires Rust 1.85+ ([rustup](https://rustup.rs)) and a C toolchain
(SQLite is compiled in):

| OS | Install first |
|---|---|
| macOS | `xcode-select --install` |
| Debian/Ubuntu | `sudo apt install build-essential pkg-config libasound2-dev libfontconfig1-dev` |
| Fedora | `sudo dnf install gcc pkgconf-pkg-config alsa-lib-devel fontconfig-devel` |
| Arch | `sudo pacman -S base-devel alsa-lib fontconfig` |
| Windows | Visual Studio Build Tools, workload "Desktop development with C++" |

```bash
cargo install --path . --locked                       # window + terminal UI
cargo install --path . --locked --no-default-features  # terminal UI only (smaller)
```

## Set up Google sign-in (once, ~5 minutes)

The app signs in with **your own** Google Cloud OAuth client, so your
account is shared with no one else. The client is free.

1. Open <https://console.cloud.google.com/> and create a project.
2. **APIs & Services → Library**: enable **YouTube Data API v3**.
3. **Google Auth Platform → Branding**: app name and your email.
   **Audience**: user type *External*, and add your Google account under
   **Test users**.
4. **Clients → Create client**, type **Desktop app**. In the dialog click
   **Download JSON** (the secret is shown only once).
5. Give it to the app:
   - **In the window:** **Sign in** (bottom of the sidebar) → **Import
     downloaded JSON** finds the newest `client_secret_*.json` in your
     Downloads; or paste the client ID and secret and click **Save client**.
   - **In a terminal:** `ytm import-client ~/Downloads/client_secret_….json`
6. Sign in: **Sign in with Google** in the window, `L` in the terminal UI,
   or `ytm login`. Your browser opens; Google warns that the app is
   unverified — choose **Continue**, it is your own app. The app asks to
   manage your YouTube account so it can like tracks and add them to
   playlists; it never deletes anything.

The sign-in is stored in the system keychain (macOS Keychain, Windows
Credential Manager, GNOME Keyring / KWallet on Linux).

While the Google app stays in *Testing* status, Google expires the sign-in
after 7 days; the player then says so — sign in again the same way.
Publishing it (**Audience → Publish app**) removes the limit but needs a
public home page and privacy policy.

**Without a browser (SSH):** create a second client of type **TVs and
Limited Input devices**, import it with `ytm import-client <json> --device`,
then `ytm login --device` shows a code to approve from any phone or
computer.

## Use

### Desktop window

Open **ytm-player** from your applications, or run `ytm gui`.

- **Library** on the left: Liked music and your playlists. Drag the
  sidebar's edge to resize it (double-click the edge to reset).
- **Track list:** double-click to play. Hovering a row shows **save to
  playlist**, **play next** and **like**; liked songs keep a filled heart.
- **Search box:** typing filters the list you're looking at; **Enter**
  searches YouTube Music. Results get their own entry at the top of the
  sidebar.
- **Player bar:** progress line (click to seek), previous / play / next,
  the current track with a like button, volume, repeat, shuffle, play next
  and save to playlist. Click the track or ▲ for the full-screen **now
  playing** view (big cover, "Up next" queue; `Esc` or ▼ closes it).
- **Account:** **Sign in** / **Account** at the bottom of the sidebar.
- Keys: `Space` play/pause, `n`/`p` next/previous, `f` like, `/` search
  box, `Esc` closes the full-screen view.
- The layout adapts to the window: narrower sidebar, then a playlist picker
  instead of the sidebar, and icon-only buttons on small windows.

Only one player runs at a time: launching it again brings the open window
to the front. The bottom of the sidebar shows the player's memory use (plus
yt-dlp's while it fetches a track link). To make `ytm` open the window
instead of the terminal UI, set `ui = "gui"` in the config.

### Terminal UI

Run `ytm` in a terminal.

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
| `o` | search YouTube Music (results replace the track list) |
| `s` / `e` | shuffle / repeat (off → all → one) |
| `v` | show the queue |
| `u` | play the selected track next |
| `f` | like / unlike (liked tracks show ♥) |
| `a` | add to a playlist |
| `r` | sync library |
| `L` | sign in with Google (opens the browser) |
| `?` | help |
| `q` | quit |

Volume, shuffle, repeat and the selected playlist are remembered.

### Commands

```bash
ytm                         # open the player (terminal UI, or the window with ui = "gui")
ytm gui | ytm tui           # open the window / the terminal UI
ytm sync                    # refresh the library (unchanged playlists are skipped)
ytm play <url|id>           # play a single track without the UI
ytm import-client <json> [--device]   # set the OAuth client from Google's JSON
ytm login [--device] | ytm logout
ytm status                  # config, sign-in and library state
ytm devices                 # audio outputs (for `audio_device`)
ytm config                  # path of config.toml
ytm uninstall [--purge]     # remove the app (see Uninstall)
```

### Search and API quota

Google gives each OAuth client 10,000 API units a day. A sync costs ~1
unit per 50 tracks (unchanged playlists are skipped), a like or "save to
playlist" 50, a search 100 — about 90 searches a day next to normal use.
Playing and browsing never use quota. Without sign-in, or when the quota
runs out, search falls back to yt-dlp (no quota, ~2 s).

## Configuration

`config.toml` (`ytm config` prints its path) is created with comments on
first run. The OAuth client is usually set from the app; the rest is
optional:

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

Logs: `ytm-player/logs` in the OS cache directory (`YTM_LOG=debug` for
more detail).

## Uninstall

Packages remove like any other package (your library and sign-in stay):

| Installed with | Remove |
|---|---|
| `.deb` | `sudo apt remove ytm-player` |
| `.rpm` | `sudo dnf remove ytm-player` |
| Arch package | `sudo pacman -R ytm-player` |
| Windows setup | **Settings → Apps → ytm-player → Uninstall** (asks whether to delete your library and sign-in) |

To remove everything else — the macOS .pkg, `install.sh` / `cargo install`
copies, menu entries, the PATH line, and optionally your library,
settings, logs and the stored Google sign-in — use the uninstaller:

```bash
ytm uninstall              # built into the app; asks before deleting your data
ytm uninstall --purge -y   # delete everything, no questions
./uninstall.sh             # the same script (repository or release download)
```

On Windows, `ytm uninstall` opens the setup program's uninstaller; for
`install.ps1` installs run
`powershell -ExecutionPolicy Bypass -File .\uninstall.ps1 [-Purge]`.

By hand: the binary (`/usr/bin/ytm`, `/usr/local/bin/ytm`,
`~/.cargo/bin/ytm` or `%LOCALAPPDATA%\Programs\ytm-player`), the app
(`/Applications` or `~/Applications/ytm-player.app`), the sign-in
(`ytm logout`), and the data:

| OS | Data |
|---|---|
| Linux | `~/.config/ytm-player`, `~/.local/share/ytm-player`, `~/.cache/ytm-player` |
| macOS | `~/Library/Application Support/dev.ytm-player.ytm-player`, `~/Library/Caches/dev.ytm-player.ytm-player` |
| Windows | `%APPDATA%\ytm-player`, `%LOCALAPPDATA%\ytm-player` |

## Troubleshooting

- **No sound on Linux:** install your sound server's ALSA plugin
  (`pipewire-alsa` on PipeWire systems, `libasound2-plugins` / `alsa-plugins-pulseaudio`
  on PulseAudio), or pick an output from `ytm devices` as `audio_device`.
- **"Sign-in expired"** after a week: normal for Google apps in *Testing*
  status — sign in again (see above).
- **Sign-in isn't remembered on Linux:** a Secret Service provider is
  needed — GNOME Keyring or KWallet, unlocked.
- **A track won't play:** yt-dlp updates itself when YouTube changes; the
  player retries automatically. Age-restricted tracks need browser cookies
  (`ytdlp_extra_args`). Details are in the log.
- **The window doesn't open on Linux:** it needs `libxkbcommon` plus X11 or
  Wayland libraries (installed with the desktop; the packages recommend them).
- `ytm status` shows the config path, whether the OAuth client is set,
  sign-in, library size and the yt-dlp in use.

## Releases

Pushing a tag builds the installers above (plus plain binaries for the
install scripts) and publishes them as a GitHub Release. The workflow also
installs every package in a clean Debian, Ubuntu, Fedora, Arch, Windows and
macOS environment and runs it.

```bash
git tag v0.4.0 && git push origin v0.4.0
```

**Actions → Release → Run workflow** builds everything as workflow
artifacts without publishing a release.

## Notes and limits

- "Liked music" comes from YouTube Music's own liked list (`LM`) when the
  API returns it; otherwise from your liked YouTube videos, filtered to the
  Music category unless `liked_music_only = false`. The sync message says
  which.
- Audio is AAC ~128 kbps. Opus would need a C library (libopus via cmake)
  on every platform for no audible gain, so it's not used.
- yt-dlp runs as a short-lived child process, one at a time (~90 MB for a
  couple of seconds while it fetches a track link).
- Media keys aren't supported on Windows yet (they need a window handle).
- Linux packages are x86_64 only; other architectures can use
  `install.sh` (builds from source).
