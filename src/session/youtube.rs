//! The YouTube sign-in ("Sign in with Google"): Google's page in a window,
//! or a browser's sign-in; then age-restricted songs go through it and
//! YouTube Music answers as the account.

use super::{Background, Changes, Session};

impl Session {
    /// `browser`: take that browser's YouTube sign-in instead of the window.
    pub fn sign_in_youtube(&mut self, browser: Option<String>) {
        if self.youtube_signing_in {
            return;
        }
        self.youtube_signing_in = true;
        self.set_info(match &browser {
            None => "Sign in to Google in the window that opened…".to_owned(),
            Some(b) => format!("Reading {b}'s YouTube sign-in…"),
        });
        let (tx, youtube, source) = (
            self.tx.clone(),
            self.deps.youtube.clone(),
            self.deps.source.clone(),
        );
        tokio::spawn(async move {
            let cookies = match &browser {
                None => crate::signin::with_window().await,
                Some(browser) => match source.export_cookies(browser).await {
                    Ok(text) => crate::signin::from_browser_export(&text, browser),
                    Err(err) => Err(err),
                },
            };
            // The credential store blocks (D-Bus on Linux).
            let result = match cookies {
                Ok(cookies) => tokio::task::spawn_blocking(move || youtube.sign_in(cookies))
                    .await
                    .unwrap_or_else(|err| Err(err.into())),
                Err(err) => Err(err),
            };
            let _ = tx.send(Background::YouTubeSignedIn(result));
        });
    }

    pub fn sign_out_youtube(&mut self) {
        let (tx, youtube) = (self.tx.clone(), self.deps.youtube.clone());
        tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(move || youtube.sign_out())
                .await
                .unwrap_or_else(|err| Err(err.into()));
            let _ = tx.send(Background::YouTubeSignedOut(result));
        });
    }

    pub(super) fn youtube_event(&mut self, event: Background, changes: &mut Changes) {
        changes.account = true;
        match event {
            Background::YouTubeSignedIn(result) => {
                self.youtube_signing_in = false;
                match result {
                    Ok(()) => {
                        self.use_youtube_cookies();
                        self.set_info("Signed in to YouTube");
                        // Home becomes the account's own.
                        self.load_home(true);
                    }
                    Err(err) => self.set_error(format!("YouTube sign-in: {err:#}")),
                }
            }
            Background::YouTubeSignedOut(result) => {
                self.use_youtube_cookies();
                match result {
                    Ok(()) => {
                        self.set_info("Signed out of YouTube");
                        self.load_home(true);
                    }
                    Err(err) => self.set_error(format!("YouTube sign-out: {err:#}")),
                }
            }
            _ => unreachable!("not a YouTube sign-in event"),
        }
    }

    /// yt-dlp signs in with the YouTube sign-in, else the configured browser.
    fn use_youtube_cookies(&mut self) {
        let source = match self.deps.youtube.cookies_file() {
            Some(file) => file.to_string_lossy().into_owned(),
            None => self.cookies_browser.clone(),
        };
        self.deps.source.set_cookies_browser(&source);
    }
}
