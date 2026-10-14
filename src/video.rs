//! Watching the clip: the song's video in the system web view (WebKit on
//! macOS, WebView2 on Windows), run as a child process
//! (`ytm video-window <id> <seconds>`) so the player itself never loads a
//! browser engine — the same trick as the sign-in window (see `signin`).
//! The child prints the position the video stopped at, so switching back to
//! audio carries on there. Linux has no system web view: the clip opens in
//! the browser instead.

use std::time::Duration;

use anyhow::{Context, Result, bail};

/// Whether the clip can play in a window of ours (else: the browser).
pub const fn in_window() -> bool {
    cfg!(all(feature = "gui", any(target_os = "macos", windows)))
}

/// The song on YouTube Music, at `at` — what the browser gets.
pub fn watch_url(video_id: &str, at: Duration) -> String {
    format!(
        "https://music.youtube.com/watch?v={video_id}&t={}s",
        at.as_secs()
    )
}

/// Opens the clip and waits for the window to close; the result is where
/// the video stopped, for audio to continue from.
pub async fn watch(video_id: &str, at: Duration) -> Result<Duration> {
    if !in_window() {
        bail!("no video window on this system");
    }
    let exe = std::env::current_exe().context("finding the ytm binary")?;
    let mut command = tokio::process::Command::new(exe);
    #[cfg(windows)]
    {
        // ytm.exe is a console program: no console window for the child.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let output = command
        .args(["video-window", video_id, &at.as_secs().to_string()])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .context("opening the video window")?;
    if !output.status.success() {
        bail!("the video window closed with an error");
    }
    Ok(parse_position(&String::from_utf8_lossy(&output.stdout)).unwrap_or(at))
}

/// The seconds the child printed ("83.4"), if it printed any.
fn parse_position(text: &str) -> Option<Duration> {
    let secs: f64 = text.trim().parse().ok()?;
    (secs.is_finite() && secs >= 0.0).then(|| Duration::from_secs_f64(secs))
}

#[cfg(all(feature = "gui", any(target_os = "macos", windows)))]
pub use window::run as run_window;

#[cfg(all(feature = "gui", any(target_os = "macos", windows)))]
mod window {
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    use anyhow::{Context, Result, bail};
    use winit::{
        application::ApplicationHandler,
        dpi::LogicalSize,
        event::WindowEvent,
        event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy},
        window::{Window, WindowId},
    };
    use wry::{Rect, WebContext, WebView, WebViewBuilder};

    /// The song's YouTube page, at the second the audio is on. (The embed
    /// player refuses to play videos whose owners only allow embedding on
    /// their own sites — checked 2026-10-14 with Vevo's: `currentTime`
    /// stayed at 0 — the watch page plays everything.)
    fn video_url(video_id: &str, at: Duration) -> String {
        format!(
            "https://www.youtube.com/watch?v={video_id}&t={}s",
            at.as_secs()
        )
    }

    /// Reports where the video is, so audio can continue there.
    const WATCH_POSITION: &str = r#"
        setInterval(function () {
            var v = document.querySelector('video');
            if (v && window.ipc) { window.ipc.postMessage(String(v.currentTime)); }
        }, 500);
    "#;

    struct App {
        url: String,
        context: WebContext,
        position: Arc<Mutex<Option<f64>>>,
        window: Option<Window>,
        webview: Option<WebView>,
        error: Option<String>,
    }

    impl ApplicationHandler<()> for App {
        /// Only sent by the test timer below.
        fn user_event(&mut self, event_loop: &ActiveEventLoop, _: ()) {
            event_loop.exit();
        }

        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            if self.window.is_some() {
                return;
            }
            let attributes = Window::default_attributes()
                .with_title("Video — ytm-player")
                .with_inner_size(LogicalSize::new(960.0, 540.0));
            let window = match event_loop.create_window(attributes) {
                Ok(window) => window,
                Err(err) => {
                    self.error = Some(err.to_string());
                    return event_loop.exit();
                }
            };
            let position = self.position.clone();
            let builder = WebViewBuilder::new_with_web_context(&mut self.context)
                .with_bounds(fill(&window))
                .with_url(&self.url)
                .with_incognito(true)
                .with_initialization_script(WATCH_POSITION)
                .with_ipc_handler(move |request| {
                    if let Ok(secs) = request.body().parse::<f64>() {
                        *position.lock().unwrap_or_else(|e| e.into_inner()) = Some(secs);
                    }
                });
            // A child of winit's view, not the window's content view (see
            // `signin`: a replaced content view crashed winit on focus loss).
            match builder.build_as_child(&window) {
                Ok(webview) => self.webview = Some(webview),
                Err(err) => {
                    self.error = Some(err.to_string());
                    return event_loop.exit();
                }
            }
            window.focus_window();
            self.window = Some(window);
        }

        fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
            match event {
                WindowEvent::CloseRequested => event_loop.exit(),
                WindowEvent::Resized(_) => {
                    if let (Some(window), Some(webview)) = (&self.window, &self.webview) {
                        let _ = webview.set_bounds(fill(window));
                    }
                }
                _ => {}
            }
        }
    }

    /// The whole inside of the window.
    fn fill(window: &Window) -> Rect {
        let size = window.inner_size().to_logical::<f64>(window.scale_factor());
        Rect {
            position: winit::dpi::LogicalPosition::new(0.0, 0.0).into(),
            size: size.into(),
        }
    }

    /// `ytm video-window <id> <seconds>`: plays the clip, then prints where
    /// it stopped.
    pub fn run(video_id: &str, at: Duration) -> Result<()> {
        let event_loop = EventLoop::new().context("opening a window")?;
        // WebView2 wants a writable folder even when nothing is stored.
        let data = std::env::temp_dir().join(format!("ytm-video-{}", std::process::id()));
        let mut app = App {
            url: video_url(video_id, at),
            context: WebContext::new(Some(data.clone())),
            position: Arc::new(Mutex::new(None)),
            window: None,
            webview: None,
            error: None,
        };
        // Testing the window without a hand on the mouse (`YTM_VIDEO_EXIT_AFTER=15`):
        // it closes by itself, which also prints the position.
        if let Ok(secs) = std::env::var("YTM_VIDEO_EXIT_AFTER")
            .unwrap_or_default()
            .parse::<u64>()
        {
            let proxy: EventLoopProxy<()> = event_loop.create_proxy();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(secs));
                let _ = proxy.send_event(());
            });
        }
        let result = event_loop.run_app(&mut app);
        // Close the web view before deleting its folder.
        drop(app.webview.take());
        drop(app.window.take());
        std::fs::remove_dir_all(&data).ok();
        result.context("video window")?;
        if let Some(error) = app.error {
            bail!(error);
        }
        if let Some(secs) = *app.position.lock().unwrap_or_else(|e| e.into_inner()) {
            println!("{secs}");
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_video_starts_where_the_song_is() {
            assert_eq!(
                video_url("YkLLcIKhJ64", Duration::from_secs(42)),
                "https://www.youtube.com/watch?v=YkLLcIKhJ64&t=42s"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_travel_through_the_child_process() {
        assert_eq!(
            parse_position("83.4\n"),
            Some(Duration::from_secs_f64(83.4))
        );
        assert_eq!(parse_position(""), None);
        assert_eq!(parse_position("-1"), None);
        assert_eq!(
            watch_url("YkLLcIKhJ64", Duration::from_secs(90)),
            "https://music.youtube.com/watch?v=YkLLcIKhJ64&t=90s"
        );
    }
}
