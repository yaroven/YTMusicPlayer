# Troubleshooting

`ytm status` shows the config path, whether the OAuth client is set, the
sign-in, the library size and the yt-dlp in use. Details of any failure are in
the log (`ytm-player/logs` in the OS cache directory, `YTM_LOG=debug` for
more).

## "age-restricted: YouTube wants a signed-in account"

YouTube plays such songs to signed-in adults only. **Sign in → Sign in with
Google** (or `ytm sign-in`) — see [google-account.md](google-account.md). The
session is kept in the OS credential store and used only for those songs.

They also start a few seconds slower than other songs: YouTube hands them over
in a form the player has to convert before playing.

Instead of the in-app sign-in you can borrow a browser's: pick it under
**Settings → Or use a browser's sign-in**, or set `cookies_from_browser` in
the config.

- **macOS** keeps other apps' data private: allow **ytm-player** in System
  Settings → Privacy & Security → **Full Disk Access**, then restart it.
  Chrome also asks once for its "Safe Storage" key — choose Always Allow.
- **Without that permission** (or on Windows, where Chrome encrypts its
  cookies for itself): export YouTube's cookies with a browser extension such
  as "Get cookies.txt LOCALLY" and set `cookies_from_browser` to the file's
  path, e.g. `"~/Documents/youtube-cookies.txt"`. Keep that file private: it
  signs in as you.
- "isn't signed in to YouTube": that browser has no YouTube login, or the
  account isn't age-verified.

## Other

- **"Sign-in expired" after a week:** that is the library-sync sign-in, normal
  for Google apps in *Testing* status — sign in again
  ([google-account.md](google-account.md)). Playback is unaffected.
- **No sound on Linux:** install your sound server's ALSA plugin
  (`pipewire-alsa` on PipeWire systems, `libasound2-plugins` /
  `alsa-plugins-pulseaudio` on PulseAudio), or pick an output from
  `ytm devices` as `audio_device`.
- **Sign-in isn't remembered on Linux:** a Secret Service provider is needed —
  GNOME Keyring or KWallet, unlocked.
- **A track won't play:** yt-dlp updates itself when YouTube changes and the
  player retries automatically. Age-restricted tracks need a sign-in (above).
- **The window doesn't open on Linux:** it needs `libxkbcommon` plus X11 or
  Wayland libraries (installed with any desktop; the packages recommend them).
- **No synced lyrics for a song:** the word timings come from
  [LRCLIB](https://lrclib.net), which doesn't have every song; the player then
  shows YouTube's plain text.
- **iPhone:** see [iphone.md](iphone.md#if-something-doesnt-work).
