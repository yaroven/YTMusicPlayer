//! The clip inside the window, the way YouTube Music does it: the system
//! web view (WebKit on macOS, WebView2 on Windows) as a **child view** of
//! the player's window, placed exactly over the big cover, showing the
//! song's YouTube page. Switching back to audio destroys it, so the browser
//! engine costs memory only while a video is on screen (its WebContent
//! helper processes go with it).
//!
//! Linux has no system web view (webkit2gtk would be a hard dependency of
//! the whole binary): there the clip opens in the browser
//! ([`crate::video::watch_url`]).

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, Result};
use slint::ComponentHandle;
use wry::{
    Rect, WebView, WebViewBuilder,
    dpi::{LogicalPosition, LogicalSize},
};

use super::MainWindow;

/// Runs on the video page: keeps reporting where the video is (so audio can
/// continue there), strips the watch page down to the player, and — when the
/// embed refuses to play, which owners can ask for — moves on to the watch
/// page, which plays everything.
fn script(watch_url: &str) -> String {
    format!(
        r#"(function () {{
    var watch = location.pathname === '/watch';
    if (watch) {{
        var css = document.createElement('style');
        css.textContent = 'ytd-masthead,#masthead-container,#secondary,#below,ytd-comments,#chat,tp-yt-app-drawer{{display:none!important}}'
            + 'ytd-watch-flexy #primary,ytd-watch-flexy #primary-inner,#player,#player-container{{max-width:100vw!important;padding:0!important;margin:0!important}}'
            + 'html,body{{overflow:hidden!important;background:#000!important}}';
        (document.head || document.documentElement).appendChild(css);
    }} else {{
        var css = document.createElement('style');
        css.textContent = 'html,body{{background:#000!important;margin:0!important}}';
        (document.head || document.documentElement).appendChild(css);
        // Embedding is blocked for some songs: their watch page still plays.
        setTimeout(function () {{
            var v = document.querySelector('video');
            if (!v || (v.paused && v.currentTime === 0)) {{ location.replace("{watch_url}"); }}
        }}, 3500);
    }}
    setInterval(function () {{
        var v = document.querySelector('video');
        if (v && window.ipc) {{ window.ipc.postMessage(String(v.currentTime)); }}
    }}, 500);
}})();"#
    )
}

/// A video playing over the window's cover area.
pub struct VideoView {
    webview: WebView,
    /// Last position the page reported.
    position: Arc<Mutex<Option<f64>>>,
    pub video_id: Arc<str>,
}

impl VideoView {
    /// Starts the clip of `video_id` at `at`, over `rect` (window
    /// coordinates, logical pixels).
    pub fn open(
        ui: &MainWindow,
        video_id: Arc<str>,
        at: Duration,
        rect: (f32, f32, f32, f32),
    ) -> Result<Self> {
        let position = Arc::new(Mutex::new(None));
        let reported = position.clone();
        // The embed player is just the video; it refuses songs whose owners
        // allow embedding only on their own sites (measured 2026-10-14:
        // `currentTime` stayed 0), and the script then moves to the watch
        // page, which plays everything.
        let watch = format!(
            "https://www.youtube.com/watch?v={video_id}&t={}s",
            at.as_secs()
        );
        let url = format!(
            "https://www.youtube.com/embed/{video_id}?autoplay=1&playsinline=1&rel=0&start={}",
            at.as_secs()
        );
        let script = script(&watch);
        let build = |parent: &slint::winit_030::winit::window::Window| {
            WebViewBuilder::new()
                .with_bounds(bounds(rect))
                .with_url(&url)
                .with_incognito(true)
                .with_initialization_script(&script)
                .with_ipc_handler(move |request| {
                    if let Ok(secs) = request.body().parse::<f64>() {
                        *reported.lock().unwrap_or_else(|e| e.into_inner()) = Some(secs);
                    }
                })
                // A child of the window's own view: replacing the content
                // view crashes winit when the window loses focus (0.9.1).
                .build_as_child(parent)
        };
        let webview = {
            use slint::winit_030::WinitWindowAccessor;
            ui.window()
                .with_winit_window(build)
                .context("the window isn't on screen yet")?
                .context("opening the video view")?
        };
        Ok(Self {
            webview,
            position,
            video_id,
        })
    }

    /// Moves the video with the window / layout.
    pub fn set_bounds(&self, rect: (f32, f32, f32, f32)) {
        let _ = self.webview.set_bounds(bounds(rect));
    }

    /// Where the video got to, for the audio to continue from.
    pub fn position(&self) -> Option<Duration> {
        let secs = (*self.position.lock().unwrap_or_else(|e| e.into_inner()))?;
        (secs.is_finite() && secs >= 0.0).then(|| Duration::from_secs_f64(secs))
    }
}

/// The 16:9 video, centred in the cover's place.
fn bounds((x, y, w, h): (f32, f32, f32, f32)) -> Rect {
    let (w, h) = (w.max(1.0), h.max(1.0));
    let video_w = w.min(h * 16.0 / 9.0);
    let video_h = video_w * 9.0 / 16.0;
    Rect {
        position: LogicalPosition::new(
            (x + (w - video_w) / 2.0).max(0.0) as f64,
            (y + (h - video_h) / 2.0).max(0.0) as f64,
        )
        .into(),
        size: LogicalSize::new(video_w as f64, video_h as f64).into(),
    }
}
