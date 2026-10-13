# Google account

The player has two separate sign-ins. The first is one click and covers
playback; the second is optional and only needed to sync your library through
Google's official API.

## 1. Sign in with Google (playback, your YouTube Music)

**Sign in** at the bottom of the sidebar → **Sign in with Google**. Google's
own sign-in page opens in a window of its own; the password goes only to
Google. The resulting youtube.com session is kept in the system keychain
(macOS Keychain, Windows Credential Manager, GNOME Keyring / KWallet on
Linux).

With it, age-restricted songs play with your account — YouTube serves them to
signed-in adults only.

- **Linux** has no built-in web view, so sign in at youtube.com in a browser
  and use **Import** in the same dialog (or `ytm sign-in --from firefox`).
- If Google refuses the in-app window with "this browser may not be secure",
  do the same: sign in in your normal browser, then `ytm sign-in --from <browser>`.
- `ytm sign-out` forgets it.

Searching, pages, radio, lyrics and ordinary playback all work without
signing in at all.

## 2. Library sync (YouTube Data API, optional)

This is what puts your liked music and playlists in the sidebar and sends
likes and "save to playlist" back to YouTube. It needs **your own** Google
Cloud OAuth client, so your account is shared with no one else. The client is
free and takes about five minutes once.

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
after 7 days and the player says so — sign in again the same way. Publishing
it (**Audience → Publish app**) removes the limit but needs a public home page
and privacy policy.

**Without a browser (SSH):** create a second client of type **TVs and Limited
Input devices**, import it with `ytm import-client <json> --device`, then
`ytm login --device` shows a code to approve from any phone or computer.

## API quota

Google gives each OAuth client 10,000 API units a day. A sync costs ~1 unit
per 50 tracks (unchanged playlists are skipped), a like or "save to playlist"
50, creating / renaming / deleting a playlist 50, following an artist 50.
Search, pages, home, radio, lyrics and playback use YouTube Music's web
endpoints and never use quota; song search falls back to the API and then to
yt-dlp if those fail.
