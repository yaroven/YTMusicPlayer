//! "Sign in with Google": Google's own sign-in page in the system web view
//! (WebKit on macOS, WebView2 on Windows), run as a child process
//! (`ytm sign-in-window`) so the player never loads a browser engine. When
//! youtube.com has a session, the child prints its cookies as JSON on
//! stdout and exits; the password only ever goes to Google. Linux (no
//! system web view without webkit2gtk) imports the sign-in from a browser.

use anyhow::{Context, Result, bail};

use crate::youtube_session::{Cookies, is_signed_in, parse_cookies_txt};

/// The YouTube sign-in in a browser's cookies, as yt-dlp exported them
/// (`export_cookies`; macOS: Full Disk Access for Safari / Firefox).
pub fn from_browser_export(text: &str, browser: &str) -> Result<Cookies> {
    let cookies = parse_cookies_txt(text);
    if !is_signed_in(&cookies) {
        bail!("{browser} isn't signed in to YouTube — sign in at youtube.com there first");
    }
    Ok(cookies)
}

/// Runs the sign-in window and returns the youtube.com cookies.
pub async fn with_window() -> Result<Cookies> {
    if !cfg!(all(feature = "gui", any(target_os = "macos", windows))) {
        bail!("sign in to YouTube in a browser, then use \"Import from browser\"");
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
        .arg("sign-in-window")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .context("opening the sign-in window")?;
    if !output.status.success() {
        bail!("sign-in window closed before signing in");
    }
    serde_json::from_slice(&output.stdout).context("reading the sign-in")
}

#[cfg(all(feature = "gui", any(target_os = "macos", windows)))]
pub use window::run as run_window;

#[cfg(all(feature = "gui", any(target_os = "macos", windows)))]
mod window {
    use anyhow::{Context, Result, bail};
    use winit::{
        application::ApplicationHandler,
        dpi::LogicalSize,
        event::WindowEvent,
        event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy},
        window::{Window, WindowId},
    };
    use wry::{PageLoadEvent, Rect, WebContext, WebView, WebViewBuilder};

    use crate::youtube_session::{Cookies, is_signed_in};

    /// After signing in, Google sends the page on to YouTube Music.
    const START: &str = "https://accounts.google.com/ServiceLogin?service=youtube&uilel=3&passive=true&continue=https%3A%2F%2Fwww.youtube.com%2Fsignin%3Faction_handle_signin%3Dtrue%26app%3Ddesktop%26next%3Dhttps%253A%252F%252Fmusic.youtube.com%252F";
    /// Google refuses sign-in in web views that don't look like a browser;
    /// WKWebView's own user agent lacks Safari's "Version/… Safari/…".
    #[cfg(target_os = "macos")]
    const USER_AGENT: Option<&str> = Some(
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_7_3) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15",
    );
    /// WebView2 already identifies as Edge.
    #[cfg(windows)]
    const USER_AGENT: Option<&str> = None;

    struct App {
        proxy: EventLoopProxy<()>,
        context: WebContext,
        window: Option<Window>,
        webview: Option<WebView>,
        cookies: Option<Cookies>,
        error: Option<String>,
    }

    impl ApplicationHandler<()> for App {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            if self.window.is_some() {
                return;
            }
            let attributes = Window::default_attributes()
                .with_title("Sign in to YouTube — ytm-player")
                .with_inner_size(LogicalSize::new(480.0, 720.0));
            let window = match event_loop.create_window(attributes) {
                Ok(window) => window,
                Err(err) => {
                    self.error = Some(err.to_string());
                    return event_loop.exit();
                }
            };
            let proxy = self.proxy.clone();
            let mut builder = WebViewBuilder::new_with_web_context(&mut self.context)
                .with_bounds(fill(&window))
                .with_url(START)
                .with_incognito(true)
                .with_on_page_load_handler(move |event, url| {
                    // Every finished youtube.com page: signed in yet?
                    if matches!(event, PageLoadEvent::Finished)
                        && url::Url::parse(&url)
                            .ok()
                            .and_then(|u| u.host_str().map(|h| h.ends_with("youtube.com")))
                            .unwrap_or(false)
                    {
                        let _ = proxy.send_event(());
                    }
                });
            if let Some(agent) = USER_AGENT {
                builder = builder.with_user_agent(agent);
            }
            // A child of winit's view, not the window's content view:
            // winit reads its own view on focus changes (a replaced one
            // crashed when the window lost focus, macOS, 0.9.1).
            match builder.build_as_child(&window) {
                Ok(webview) => self.webview = Some(webview),
                Err(err) => {
                    self.error = Some(err.to_string());
                    return event_loop.exit();
                }
            }
            // Started from the player: come to the front.
            window.focus_window();
            self.window = Some(window);
        }

        fn user_event(&mut self, event_loop: &ActiveEventLoop, _: ()) {
            let Some(webview) = &self.webview else { return };
            let cookies: Cookies = match webview.cookies() {
                Ok(all) => all
                    .into_iter()
                    .filter(|c| c.domain().is_some_and(|d| d.ends_with("youtube.com")))
                    .map(|c| (c.name().to_owned(), c.value().to_owned()))
                    .collect(),
                Err(err) => {
                    tracing::warn!(%err, "web view cookies");
                    return;
                }
            };
            if is_signed_in(&cookies) {
                self.cookies = Some(cookies);
                event_loop.exit();
            }
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

    /// `ytm sign-in-window`: prints the cookies as JSON, or fails.
    pub fn run() -> Result<()> {
        let event_loop = EventLoop::new().context("opening a window")?;
        // A throwaway profile; incognito keeps it out of the disk anyway
        // (WebView2 still wants a writable folder).
        let data = std::env::temp_dir().join(format!("ytm-sign-in-{}", std::process::id()));
        let mut app = App {
            proxy: event_loop.create_proxy(),
            context: WebContext::new(Some(data.clone())),
            window: None,
            webview: None,
            cookies: None,
            error: None,
        };
        let result = event_loop.run_app(&mut app);
        // Close the web view before deleting its folder.
        drop(app.webview.take());
        drop(app.window.take());
        std::fs::remove_dir_all(&data).ok();
        result.context("sign-in window")?;
        if let Some(error) = app.error {
            bail!(error);
        }
        let Some(cookies) = app.cookies else {
            bail!("closed before signing in");
        };
        println!("{}", serde_json::to_string(&cookies)?);
        Ok(())
    }
}
