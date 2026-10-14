# Using the player

## Desktop window

Open **ytm-player** from your applications, or run `ytm gui`.

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
- **Downloads:** while songs are being saved a **Stop** button appears in the
  header (it also drops the half-written file); the Downloads list has
  **Delete all**, and ⋮ → Remove deletes a single song's file.
- **Player bar:** progress (click to seek), previous / play / next, the
  current track with dislike, like and ⋮, volume, repeat, shuffle, **video**,
  **Cast**, **sleep timer** and the full-screen view.
- **Video** (the ▶ in a frame, or `V`): the song's clip plays in the window
  in place of the cover, with Up next / Lyrics / Related beside it. The audio
  waits meanwhile; switching back (the same button, or closing the
  full-screen view) continues the song where the video stopped. On Linux,
  which has no system web view, the clip opens in your browser.
- **Now playing** (click the track or ▲): big cover — click it to play or
  pause — with **Up next**
  (reorder, remove, clear, autoplay switch), **Lyrics** and **Related**, over
  a blurred copy of the cover. Synced lyrics play karaoke style: the sung line
  lights up letter by letter, instrumental breaks show three dots, and a click
  on a line seeks there. `Esc` or ▼ closes it.
- **Settings:** autoplay, volume normalization, crossfade (0 = gapless),
  notifications, tray icon, the Google account. Saved to `config.toml`.
- Keys: `Space` play/pause, `n`/`p` next/previous, `f` like, `t` lyrics, `/`
  search box, `Backspace` back, `Esc` closes the full-screen view.
- The layout adapts to the window: narrower sidebar, then a library picker
  instead of the sidebar, and icon-only buttons on small windows.

Only one player runs at a time: launching it again brings the open window to
the front.

**Playing in the background:** with the tray icon on (default), closing the
window keeps the music playing and frees the window's memory. The tray / menu
bar icon has **Play / Pause**, **Next**, **Previous**, **Show ytm-player** and
**Quit**; media keys keep working. On macOS the app also leaves the Dock until
the window is shown again. (Linux needs a desktop with tray icons — on GNOME
the AppIndicator extension; without one, closing the window quits.)

The bottom of the sidebar shows the player's memory use (plus yt-dlp's while
it fetches a track link). To make `ytm` open the window instead of the
terminal UI, set `ui = "gui"` in the [config](configuration.md).

## Terminal UI

Run `ytm` in a terminal.

| Key | Action |
|---|---|
| `↑/↓` `k/j`, `PgUp/PgDn`, `g/G` | move |
| `Tab` `h/l` | switch pane |
| `Enter`, click | open / play |
| `b`, `Backspace` | back |
| `1` … `6` | home, history, downloads, albums, artists, explore |
| `Space` | play / pause |
| `n` / `p` | next / previous (restarts the track if > 3 s in) |
| `←` / `→`, click the bar | seek |
| `+` / `-` | volume |
| `/` | filter the list (`Esc` clears) |
| `o`, then `[` / `]` | search YouTube Music, switch category |
| `i` | albums / artists / shelves on this page |
| `s` / `e` / `A` | shuffle / repeat (off → all → one) / autoplay |
| `v` | queue (`x` remove, `J`/`K` move, `X` clear) |
| `u` / `R` / `w` | play next / start radio / the artist's page |
| `f` / `d` | like / dislike |
| `a` / `x` | add to / remove from a playlist |
| `D` | download for offline (again: delete the download) |
| `S` | save album / follow artist |
| `m` | menu: new / rename / delete playlist, download all, stop downloading, delete all downloads |
| `t` | lyrics |
| `z` | sleep timer (15 / 30 / 60 min, end of song, off) |
| `C` | play on a Chromecast |
| `r` / `L` | sync library / sign in |
| `?` / `q` | help / quit |

Volume, shuffle, repeat and the selected playlist are remembered.

## Commands

```bash
ytm                         # open the player (terminal UI, or the window with ui = "gui")
ytm gui | ytm tui           # open the window / the terminal UI
ytm sign-in [--from <browser>]        # sign in to YouTube: library, age-restricted songs
ytm sign-out                # forget the YouTube sign-in
ytm sync                    # refresh the library (unchanged playlists are skipped)
ytm play <url|id>           # play a single track without the UI
ytm import-client <json> [--device]   # OAuth client for library sync, from Google's JSON
ytm login [--device] | ytm logout     # the optional Data API sign-in (legacy library sync)
ytm update                  # install the latest release
ytm status                  # config, sign-in and library state
ytm devices                 # audio outputs (for `audio_device`)
ytm cast-devices            # Chromecasts on this network
ytm lastfm-login            # connect Last.fm scrobbling
ytm config                  # path of config.toml
ytm uninstall [--purge]     # remove the app
```
