# Yarek Music on Android

A native app (Kotlin, Jetpack Compose, Media3 — `android/` in this
repository) with the same features as the iPhone app. It isn't on Google
Play: you install the APK from the releases page.

- Home, search (songs / albums / artists / playlists) with YouTube Music's
  own suggestions, Explore (charts, new releases, moods and genres), album,
  artist and playlist pages, song radio and autoplay.
- Playback in the background with the notification, lock screen,
  Bluetooth and headset controls; gapless track changes or a crossfade
  (Library → Playback), volume normalisation like YouTube Music.
- Karaoke lyrics (the sung line lights up word by word, a tap seeks there),
  **Related** music, the editable queue, a sleep timer (🌙: 15 / 30 / 60 min
  or end of song). A tap on the big cover plays or pauses.
- **Song / video switch** over the cover (♪ / ▶): the clip plays in place of
  the cover; switching back, the song continues where the video stopped.
- Your YouTube Music library once you sign in: liked music, playlists,
  saved albums and followed artists; likes, dislikes, playlist changes
  (moving songs too) and follows go back to YouTube. History stays on the
  phone.
- **Downloads** for offline: a song's ⋮ menu → **Download**, or a whole
  album / playlist / list with its ⬇ button (one after another, "Downloading
  3 of 12", tap to stop). **Library → Downloads** lists them, **Delete all**
  clears them. On Android 10+ each download is also copied to **Music/YTM
  Player** ("Artist - Title.m4a"), where other players and file managers
  see it (Settings → "Also save downloads to Music").
- Lists sort by your own order, title, artist or **most played** (play
  counts are kept on the phone); **Up next → Save as playlist** keeps the
  queue.
- **Updates itself:** a bar says when a new version is out; **Update**
  downloads it, checks it against the release's SHA256SUMS and opens
  Android's installer (the first time Android asks to allow installs from
  Yarek Music). Settings → **Check for updates** checks now.
- English or Ukrainian: **Library → Language** (on Android 13+ also
  **Settings → Apps → Yarek Music → Language**).
- No yt-dlp and no server: the app asks YouTube for the audio link itself.

## Install

1. On the phone, open the latest release:
   <https://github.com/yaroven/YTMusicPlayer/releases/latest> and download
   `yarek-music-<version>.apk`.
2. Open the downloaded file. Android asks to allow installs from this source
   (the browser or Files) once — allow it, go back, **Install**.
3. Google Play Protect may say it doesn't know the developer: **More
   details → Install anyway**. The app isn't signed by a paid store
   account; the source is all in this repository.

Android 8.0 or newer. Later versions arrive through the app itself
(above); installing a newer APK by hand works too — your library,
downloads and sign-in stay.

## Sign in

**Library → Sign in with Google** opens Google's own sign-in page inside the
app; the password goes only to Google. Once YouTube has a session, the app
keeps its cookies (encrypted with a key in the Android Keystore, never in a
backup) and the page's own data is thrown away. **Sign out** forgets them.

## No internet, bad internet

- A grey bar at the top says **No internet connection**; when it returns, a
  green **Back online** shows for a moment.
- A screen that couldn't load says why — offline, too slow, YouTube Music
  not answering — with **Retry**, and loads again by itself as soon as the
  connection is back. Offline it also offers **Downloads**.
- A long load says "Slow connection: still loading…" after a few seconds;
  a song stuck buffering says "Slow connection, buffering…".
- Offline, songs that aren't downloaded are dimmed; tapping a downloaded
  one plays the downloaded songs of that list.
- If the connection drops mid-song, the song plays what's already buffered
  (about 30 seconds), then waits — it doesn't skip through the queue — and
  continues from the same place when the connection is back.

## Limits

- Age-restricted songs need the sign-in, and can't be downloaded.
- The video needs the internet, also for downloaded songs.
- Memory, measured on the emulator (Android 15, release build, playing,
  `dumpsys meminfo` PSS): ~80 MB with Now Playing open, ~73 MB in the
  background. The emulator draws with a software GPU, which adds to these.

## Building it

```bash
cd android
./gradlew :core:test               # parsing, lyrics, signing (offline)
YTM_LIVE=1 ./gradlew :core:test    # also against YouTube / LRCLIB
./gradlew :app:assembleRelease     # app/build/outputs/apk/release/app-release.apk
```

JDK 17 and the Android SDK (platform 35) are needed. CI builds the APK on
every Android change, and the Release workflow attaches it to releases.
Releases are signed with the project's key (the `ANDROID_KEYSTORE_*`
repository secrets; certificate SHA-256 `6f693e44…cbc7`), so each one
installs over the last. A local build without those variables is signed
with your debug key, which won't install over a release (uninstall first).
