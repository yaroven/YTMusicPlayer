# ytm-player on iPhone

A native app (SwiftUI, `ios/` in this repository) with the same core as the
desktop player. It isn't on the App Store, so you install it yourself with
**SideStore** — which signs apps with your own free Apple ID and re-signs
them every 7 days on the phone, with no computer involved after the first
setup.

- Home, search (songs / albums / artists / playlists) with YouTube Music's
  own suggestions, Explore (charts, new releases, moods and genres), album,
  artist and playlist pages, song radio and autoplay.
- Songs with explicit lyrics carry YouTube Music's **E** badge.
- Playback that keeps going with the screen locked, gapless track changes,
  lock screen and Control Center controls, AirPlay.
- Karaoke lyrics: the sung line lights up word by word, a tap on a line
  seeks there. A tap on the big cover plays or pauses.
- Your YouTube Music library once you sign in: liked music, playlists,
  saved albums and followed artists, with likes, dislikes, playlist changes
  (reordering too) written back to YouTube. History and the editable queue
  stay on the phone.
- **Offline:** long-press a song → **Download**; **Library → Downloads**
  lists them, swipe to delete. Downloaded songs play without network.
- **Song / video switch** over the cover (♪ / ▶): the clip plays in place
  of the cover and the song carries on where it stopped when you switch
  back.
- Sleep timer (🌙 in Now Playing: 15 / 30 / 60 min or end of song),
  crossfade (Library → Playback), **Related** next to the lyrics.
- English or Ukrainian: **Settings → YTM Player → Language** (or Library →
  Language).
- No yt-dlp and no server: the app asks YouTube for the audio link itself.
- **No or bad internet:** a bar says "No internet connection"; screens say
  what's wrong with **Retry** and load again by themselves when it's back;
  offline only downloaded songs play (the rest are dimmed), and a song cut
  off by a lost connection continues where it stopped once it returns.

## Requirements

| | |
|---|---|
| iPhone | iOS 17 or newer, with a passcode set |
| Apple ID | any free one (a free account allows 3 sideloaded apps at a time, SideStore itself being one) |
| Computer | once, for the SideStore setup only |

## 1. Install SideStore (once)

Follow <https://docs.sidestore.io/docs/installation/prerequisites> — their
steps change from time to time, so use their current ones rather than a copy
here. In short: install their VPN app from the App Store, connect the phone
to a computer once to put SideStore on it, then sign in with your Apple ID
inside SideStore.

The VPN must be on whenever you install, update or refresh an app. It routes
nothing to the internet — it only lets SideStore talk to the phone itself.

## 2. Add ytm-player as a source

In SideStore: **Sources → + (top right)** and paste

```
https://github.com/yaroven/YTMusicPlayer/releases/latest/download/sidestore.json
```

Open the new **ytm-player** source and install **YTM Player**. Every new
release shows up there, and SideStore offers it as an update.

**Without a source:** download `ytm-player-<version>.ipa` from the
[latest release](https://github.com/yaroven/YTMusicPlayer/releases/latest)
and open it with SideStore (or AltStore) through the Files app or the share
sheet. You then have to repeat this for each update.

## 3. First run

The app works right away: Home, search and playback need no account.

**Sign in with Google (optional)** — **Library → Sign in with Google**:

- Google's own sign-in page opens inside the app. The password goes only to
  Google; the app keeps the resulting youtube.com session in the iPhone's
  Keychain and nowhere else.
- Signed in, the **Library** tab becomes your account's: liked music, your
  playlists, saved albums and followed artists, synced from YouTube Music
  (pull down to sync again, or **Sync library**). Likes, playlist changes,
  follows and playlist deletions go back to YouTube. Home becomes your own,
  and age-restricted songs play with your account (YouTube serves them to
  signed-in adults only).
- **Library → Sign out** forgets the session and the account's lists;
  history and anything made while signed out stays.

## Keeping it installed

A free Apple ID signs apps for 7 days. SideStore re-signs them on the phone
by itself — open SideStore (with its VPN on) at least once a week, or let it
refresh in the background. If an app has expired, it stops opening until
SideStore refreshes it; nothing is lost.

## Limits compared to the desktop player

- No Chromecast (AirPlay works), Last.fm or Discord yet.
- Age-restricted songs can't be downloaded (YouTube serves them only as a
  stream the app converts while playing).
- Reordering songs inside a playlist stays on the phone (as on the desktop
  player).
- Age-restricted songs need the Google sign-in, and start a few seconds
  slower than normal ones (YouTube only serves them in a form the app has
  to convert first).

## If something doesn't work

- **"Unable to install" / 3-app limit:** a free Apple ID allows three
  sideloaded apps including SideStore. Remove one in SideStore first.
- **The app won't open after a week:** open SideStore with its VPN on and
  refresh it.
- **An age-restricted song won't play:** check that Library shows you as
  signed in, then run **Library → Test age-restricted playback**. It prints
  one line per YouTube client; send a screenshot of it to an issue — the
  client list is fixed remotely (`ios/clients.json` in this repository),
  without a new release.
- **A song has no synced lyrics:** the words come from
  [LRCLIB](https://lrclib.net), which doesn't have every song; the app then
  shows YouTube's plain text.

## Building the app yourself

CI does it on every push: `.github/workflows/ios.yml` runs the core's checks,
generates the Xcode project with [XcodeGen](https://github.com/yonaskolb/XcodeGen)
and builds an unsigned `.ipa` (SideStore signs it on the phone). Locally,
with Xcode installed:

```bash
cd ios && xcodegen generate && open YTMPlayer.xcodeproj
```

The core (`ios/YTMKit`) builds and tests without Xcode:

```bash
cd ios/YTMKit && swift run YTMKitChecks          # offline checks
swift run YTMKitChecks --live                    # also against YouTube and LRCLIB
../player-check.sh                               # drives the player on macOS, muted
```
