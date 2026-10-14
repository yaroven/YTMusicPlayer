# Google account

One sign-in covers everything: your library, age-restricted songs and your
own YouTube Music. A second, optional sign-in exists only for Google's
official Data API, which the player no longer needs.

## Sign in with Google

**Sign in** at the bottom of the sidebar → **Sign in with Google**. Google's
own sign-in page opens in a window of its own; the password goes only to
Google. The resulting youtube.com session is kept in the system keychain
(macOS Keychain, Windows Credential Manager, GNOME Keyring / KWallet on
Linux).

With it:

- **Your library** is in the sidebar — liked music, your playlists, saved
  albums and the artists you follow — and likes, playlist changes and follows
  go back to YouTube. This uses the same endpoints as the YouTube Music site,
  so there is no API quota and the sign-in doesn't expire after a week.
- **Age-restricted songs** play with your account (YouTube serves them to
  signed-in adults only).
- **Home** becomes your own personalised one.

Notes:

- **Linux** has no built-in web view, so sign in at youtube.com in a browser
  and use **Import** in the same dialog (or `ytm sign-in --from firefox`).
- If Google refuses the in-app window with "this browser may not be secure",
  do the same: sign in in your normal browser, then `ytm sign-in --from <browser>`.
- `ytm sign-out` forgets it; `ytm sync` refreshes the library by hand (the
  player also syncs right after signing in).

Searching, pages, radio, lyrics and ordinary playback all work without
signing in at all.

## Library sync through the Data API (optional, legacy)

Before the sign-in above existed, the library came from Google's official
YouTube Data API with **your own** Google Cloud OAuth client. It still works
and is used when there is no youtube.com sign-in, but it needs a five-minute
setup and Google expires its login every 7 days, so it isn't recommended any
more. If you want it:

1. Open <https://console.cloud.google.com/> and create a project.
2. **APIs & Services → Library**: enable **YouTube Data API v3**.
3. **Google Auth Platform → Branding**: app name and your email.
   **Audience**: user type *External*, and add your Google account under
   **Test users**.
4. **Clients → Create client**, type **Desktop app**. In the dialog click
   **Download JSON** (the secret is shown only once).
5. Give it to the app:
   - **In the window:** **Sign in → Library sync → Set up → Import downloaded
     JSON** finds the newest `client_secret_*.json` in your Downloads; or
     paste the client ID and secret and click **Save client**.
   - **In a terminal:** `ytm import-client ~/Downloads/client_secret_….json`
6. Sign in: **Sign in with Google** inside that section, `L` in the terminal
   UI, or `ytm login`. Your browser opens; Google warns that the app is
   unverified — choose **Continue**, it is your own app. The app asks to
   manage your YouTube account so it can like tracks and add them to
   playlists; it never deletes anything.

While the Google app stays in *Testing* status, Google expires this sign-in
after 7 days and the player says so — sign in again the same way, or use the
youtube.com sign-in above instead, which doesn't expire. Publishing the app
(**Audience → Publish app**) removes the limit but needs a public home page
and privacy policy.

**Without a browser (SSH):** create a second client of type **TVs and Limited
Input devices**, import it with `ytm import-client <json> --device`, then
`ytm login --device` shows a code to approve from any phone or computer.

### API quota

Only this path uses quota. Google gives each OAuth client 10,000 API units a
day: a sync costs ~1 unit per 50 tracks (unchanged playlists are skipped), a
like or "save to playlist" 50, creating / renaming / deleting a playlist 50,
following an artist 50. Everything else — search, pages, home, radio, lyrics,
playback and, with the youtube.com sign-in, the whole library — uses YouTube
Music's own endpoints and no quota.
