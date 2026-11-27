# Using the player

## Desktop window

Open **Yarek Music** from your applications, or run `yarek`.

- **Sidebar:** Home, **Explore** (charts, new releases, moods and genres),
  your Library (History, Downloads, Albums, Artists),
  Liked music and your playlists, **+** for a new playlist, sign-in, sync and
  **settings** at the bottom. Drag the sidebar's edge to resize it
  (double-click the edge to reset).
- **Search box:** typing filters the list you're looking at and shows what
  YouTube Music suggests; **Enter** (or a suggestion) searches it, with
  **Songs / Albums / Artists / Playlists** tabs over the results.
- **Pages:** albums, artists and playlists open with their art, **Play**,
  **Shuffle**, **Radio**, **Save** / **Follow** and **download all**; artist
  pages add shelves of albums, singles and similar artists. **←** (or
  `Backspace`) goes back.
- **Track rows:** click to play; click the artist's name for their page (also
  in the player bar and the queue); hover for like and save to playlist; **⋮**
  for start radio, play next, add to queue, download (again: delete the
  download), dislike and remove from playlist. ↓ marks downloaded songs,
  **E** explicit lyrics (18+).
- **Your playlists:** ⋮ → **Move up / Move down** reorders a song (on
  YouTube too, once the library has been synced). The **Up next** panel has
  **Save as playlist** for the whole queue.
- **Sort:** the **Sort** button over a list cycles through its own order,
  title, artist and **most played** (which shows ▶ and the count next to
  the artist). Playing from a sorted list follows what you see.
- **History:** ⋮ → **Remove from history**, or **Clear history** at the top
  (play counts are forgotten with it).
- **Downloads:** while songs are being saved a **Stop** button appears in the
  header (it also drops the half-written file); the Downloads list has
  **Delete all**, and ⋮ → Remove deletes a single song's file.
- **Player bar:** progress (click to seek), previous / play / next, the
  current track with dislike, like and ⋮, volume, repeat, shuffle, **Cast**,
  **sleep timer** and the full-screen view.
- **Song or video:** the switch over the cover in the full-screen view (♪ /
  ▶, like YouTube Music's), or `V`. The clip then plays in the window in
  place of the cover, with Up next / Lyrics / Related beside it.
  - **Linux:** the player plays the clip itself — just the picture, in step
    with the song, which keeps playing; the player bar drives both, and
    switching back (♪, `V`, closing the full-screen view) shows the cover
    again. It needs FFmpeg's libraries (`libavcodec` and `libswscale`, which
    most desktops already have; the packages recommend them); without them
    the clip opens in your browser.
  - **macOS, Windows:** YouTube's player in the window for now; the audio
    waits meanwhile, and switching back continues the song where the video
    stopped.
- **Now playing** (click the track or ▲): big cover — click it to play or
  pause — with **Up next**
  (reorder, remove, clear, autoplay switch), **Lyrics** and **Related**, over
  a blurred copy of the cover. Synced lyrics play karaoke style: the sung line
  lights up word by word, instrumental breaks show three dots, and a click
  on a line seeks there. `Esc` or ▼ closes it.
- **Settings:** language (English / Українська), autoplay, volume
  normalization, crossfade (0 = gapless), notifications, tray icon, the
  Google account. Saved to `config.toml`.
- Keys: `Space` play/pause, `n`/`p` next/previous, `f` like, `t` lyrics, `/`
  search box, `Backspace` back, `Esc` closes the full-screen view.
- The layout adapts to the window: narrower sidebar, then a library picker
  instead of the sidebar, and icon-only buttons on small windows.

Only one player runs at a time: launching it again brings the open window to
the front.

**Playing in the background:** with the tray icon on (default), closing the
window keeps the music playing and frees the window's memory. The tray / menu
bar icon has **Play / Pause**, **Next**, **Previous**, **Show Yarek Music** and
**Quit**; media keys keep working. On macOS the app also leaves the Dock until
the window is shown again. (Linux needs a desktop with tray icons — on GNOME
the AppIndicator extension; without one, closing the window quits.)

The bottom of the sidebar shows the player's memory use (plus yt-dlp's while
it fetches a track link).

## Commands

```bash
yarek | yarek gui             # open the player window
yarek sign-in [--from <browser>]        # sign in to YouTube: library, age-restricted songs
yarek sign-out                # forget the YouTube sign-in
yarek sync                    # refresh the library (unchanged playlists are skipped)
yarek play <url|id>           # play a single track without the UI
yarek import-client <json> [--device]   # OAuth client for library sync, from Google's JSON
yarek login [--device] | yarek logout     # the optional Data API sign-in (legacy library sync)
yarek update                  # install the latest release
yarek status                  # config, sign-in and library state
yarek devices                 # audio outputs (for `audio_device`)
yarek cast-devices            # Chromecasts on this network
yarek lastfm-login            # connect Last.fm scrobbling
yarek config                  # path of config.toml
yarek uninstall [--purge]     # remove the app
```
