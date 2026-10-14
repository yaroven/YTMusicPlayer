# Configuration and integrations

## config.toml

`ytm config` prints its path; the file is created with comments on first run.
Everything in it is optional — the player works with the defaults.

| Key | Default | Meaning |
|---|---|---|
| `ui` | `"tui"` | what `ytm` opens: `"tui"` or `"gui"` |
| `language` | `"auto"` | language of the window and the terminal UI: `"auto"` (the system's), `"en"` or `"uk"` |
| `volume` | `0.8` | volume for the very first start |
| `autoplay` | `true` | when the queue runs out, keep playing the last song's radio |
| `normalize_volume` | `true` | turn loud tracks down to YouTube's loudness target |
| `crossfade` | `0` | seconds tracks overlap (0 = gapless, up to 12) |
| `notifications` | `true` | desktop notification on track change (window) |
| `tray` | `true` | tray / menu bar icon (window) |
| `media_controls` | `true` | media keys and system "Now Playing" (macOS, Linux) |
| `audio_device` | `""` | output device name from `ytm devices`; empty = automatic (Linux: PipeWire, then PulseAudio, then ALSA default) |
| `check_updates` | `true` | look for a newer release at startup |
| `download_limit_mb` | `0` | stop downloading for offline beyond this size (0 = no limit) |
| `liked_music_only` | `true` | keep only "Music"-category videos in the liked list |
| `js_fallback` | `true` | if yt-dlp fails, retry with a JS runtime (system deno/node, or a ~2 MB QuickJS download). `false` = never run JS |
| `cookies_from_browser` | `""` | age-restricted songs without the in-app sign-in: a browser (`chrome`, `firefox`, `safari`, `edge`, `brave`, …) or a cookies.txt path |
| `ytdlp_extra_args` | `[]` | extra yt-dlp flags |
| `client_id`, `client_secret` | — | OAuth client for [library sync](google-account.md) (or `YTM_CLIENT_ID` / `YTM_CLIENT_SECRET`) |
| `device_client_id`, `device_client_secret` | — | optional TV-type client for `ytm login --device` |
| `lastfm_api_key`, `lastfm_api_secret` | — | Last.fm scrobbling (below) |
| `discord_client_id` | — | Discord "Listening to" status (below) |

Logs: `ytm-player/logs` in the OS cache directory (`YTM_LOG=debug` for more
detail).

## Integrations

- **Chromecast:** the Cast button in the player bar (or `C` in the terminal
  UI) lists devices on your network; the device streams the audio itself, and
  "This computer" brings playback back. Downloaded files are cast from YouTube
  too. On macOS allow "Local Network" access when asked.
- **Last.fm:** create an API account at
  <https://www.last.fm/api/account/create>, put its key and secret into
  `lastfm_api_key` / `lastfm_api_secret`, run `ytm lastfm-login` and allow
  access in the browser. Songs count after half their length or 4 minutes.
- **Discord:** create an application at
  <https://discord.com/developers/applications> (any name, e.g. "YouTube
  Music") and put its Application ID into `discord_client_id`. The desktop
  Discord app must be running.
- **Offline:** downloads are stored in the data directory's `downloads` folder
  (AAC, ~1 MB per minute) and play without network.
