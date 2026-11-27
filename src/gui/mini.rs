//! The tray's mini player (`MiniPlayer` in `ui.rs`): cover, song, like,
//! seek and ⏮ ⏯ ⏭ in a frameless window next to the tray icon. Built on
//! a click and destroyed when it loses focus, so it costs nothing while
//! closed (its frame at 280x420 is ~1.9 MB at 2x while open). The cover is
//! the main window's large one: one large image lives at a time.
//!
//! Placing it: the icon's position where the tray reports one (macOS,
//! Windows, X11). Wayland lets no client place its own window (it came up
//! in the middle of the screen, user's screenshot 2026-11-28); on KDE,
//! KWin is asked instead ([`kwin`]).

use std::{cell::Cell, cell::RefCell, rc::Rc, time::Instant};

use anyhow::{Context, Result};
use slint::{ComponentHandle, PhysicalPosition, SharedString};

use super::{
    Cmd, Cover, VIEW, View, art,
    art::ArtSize,
    current_ui, fmt_time, hue, initial, raise,
    tray::Anchor,
    ui::{L, MiniPlayer},
};
use crate::audio::player::PlayState;

/// A click on the icon right after the window lost focus to that very
/// click closes it instead of opening it again.
const REOPEN_GUARD_MS: u128 = 300;
/// Gap between the icon and the window, logical pixels.
const GAP: f64 = 6.0;
/// The window's title (`MiniPlayer` in `ui.rs`): KWin finds it by it.
#[cfg(target_os = "linux")]
const TITLE: &str = "Yarek Music mini player";

thread_local! {
    static MINI: RefCell<Option<MiniPlayer>> = const { RefCell::new(None) };
    static CLOSED_AT: Cell<Option<Instant>> = const { Cell::new(None) };
}

pub(super) fn current() -> Option<MiniPlayer> {
    MINI.with(|cell| cell.borrow().as_ref().map(|m| m.clone_strong()))
}

/// The tray icon was clicked: open the mini player, or close it.
pub(super) fn toggle(anchor: Option<Anchor>) {
    tracing::debug!(?anchor, "mini player: toggle");
    if MINI.with(|cell| cell.borrow().is_some()) {
        return close();
    }
    let just_closed = CLOSED_AT
        .with(Cell::take)
        .is_some_and(|at| at.elapsed().as_millis() < REOPEN_GUARD_MS);
    if just_closed {
        return;
    }
    if let Err(err) = open(anchor) {
        tracing::warn!("mini player: {err:#}");
    }
}

/// Destroys the window (its pixels go with it).
pub(super) fn close() {
    let Some(mini) = MINI.with(|cell| cell.borrow_mut().take()) else {
        return;
    };
    tracing::debug!("mini player: close");
    CLOSED_AT.with(|at| at.set(Some(Instant::now())));
    let _ = mini.hide();
    // Nothing shows the large cover any more: let it go.
    if current_ui().is_none() {
        VIEW.with(|cell| {
            if let Some(view) = cell.borrow().as_ref() {
                view.borrow().art.borrow_mut().large = None;
            }
        });
    }
}

fn open(anchor: Option<Anchor>) -> Result<()> {
    let view = VIEW
        .with(|cell| cell.borrow().clone())
        .context("GUI state missing")?;
    let mini = MiniPlayer::new().context("cannot open the mini player")?;
    wire(&mini, &view);
    mini.global::<L>()
        .set_nothing_playing(view.borrow().lang.text().nothing_playing.into());
    {
        let mut v = view.borrow_mut();
        v.render_mini(&mini);
        v.show_mini_art(&mini);
    }
    #[cfg(target_os = "linux")]
    let kwin_places = kwin::wanted() && kwin::place_at_pointer();
    #[cfg(not(target_os = "linux"))]
    let kwin_places = false;
    if let Some(anchor) = anchor.filter(|_| !kwin_places) {
        let scale = f64::from(mini.window().scale_factor());
        let size = mini.window().size();
        let (w, h) = (f64::from(size.width), f64::from(size.height));
        mini.window().set_position(place(anchor, w, h, GAP * scale));
    }
    {
        use slint::winit_030::{WinitWindowAccessor, winit::event::WindowEvent};
        // macOS reports `Focused(false)` while the window is being shown,
        // before it gets the focus: only a loss after a gain closes it.
        let focused = Cell::new(false);
        mini.window().on_winit_window_event(move |_, event| {
            match event {
                WindowEvent::Focused(true) => focused.set(true),
                WindowEvent::Focused(false) if focused.get() => {
                    // Not from inside the event handler that is destroying it.
                    let _ = slint::invoke_from_event_loop(close);
                }
                _ => {}
            }
            slint::winit_030::EventResult::Propagate
        });
    }
    mini.show().context("cannot show the mini player")?;
    {
        use slint::winit_030::WinitWindowAccessor;
        mini.window().with_winit_window(|w| w.focus_window());
    }
    MINI.with(|cell| *cell.borrow_mut() = Some(mini));
    Ok(())
}

/// Top-left corner for a `w` x `h` window next to the icon: centred under
/// it on a top menu bar, above it on a bottom taskbar (Windows).
fn place((x, y, iw, ih): Anchor, w: f64, h: f64, gap: f64) -> PhysicalPosition {
    let left = (x + iw / 2.0 - w / 2.0).max(0.0);
    let below = y < h;
    let top = if below { y + ih + gap } else { y - h - gap };
    PhysicalPosition::new(left as i32, top.max(0.0) as i32)
}

fn wire(mini: &MiniPlayer, view: &Rc<RefCell<View>>) {
    let send = {
        let tx = view.borrow().tx.clone();
        move |cmd: Cmd| {
            let _ = tx.send(cmd);
        }
    };
    let s = send.clone();
    mini.on_toggle_play(move || s(Cmd::TogglePause));
    let s = send.clone();
    mini.on_next(move || s(Cmd::Next));
    let s = send.clone();
    mini.on_prev(move || s(Cmd::Prev));
    let s = send.clone();
    mini.on_seek(move |ratio| s(Cmd::SeekRatio(f64::from(ratio))));
    let v = Rc::downgrade(view);
    mini.on_like(move || {
        if let Some(track) = v.upgrade().and_then(|v| v.borrow().now.clone()) {
            send(Cmd::Like(track));
        }
    });
    mini.on_open_app(|| {
        close();
        raise();
    });
    mini.on_dismiss(close);
}

impl View {
    /// The song, its progress and liked state from the last snapshot.
    pub(super) fn render_mini(&self, mini: &MiniPlayer) {
        let Some(view) = &self.last else {
            return;
        };
        mini.set_playing(view.state == PlayState::Playing);
        match &view.now {
            Some(t) => {
                mini.set_has_track(true);
                mini.set_song(SharedString::from(&*t.title));
                mini.set_artist(SharedString::from(&*t.artist));
                mini.set_initial(initial(&t.title));
                mini.set_hue(hue(&t.artist));
                mini.set_liked(self.library.borrow().is_liked(&t.video_id));
            }
            None => {
                mini.set_has_track(false);
                mini.set_artist("".into());
                mini.set_liked(false);
            }
        }
        match view.duration {
            Some(d) if !d.is_zero() => {
                mini.set_progress((view.position.as_secs_f64() / d.as_secs_f64()) as f32);
                mini.set_position_text(fmt_time(view.position).into());
                mini.set_duration_text(fmt_time(d).into());
            }
            _ => {
                mini.set_progress(0.0);
                mini.set_position_text("".into());
                mini.set_duration_text("".into());
            }
        }
    }

    /// The playing song's cover, fetched when no window has it yet.
    pub(super) fn show_mini_art(&mut self, mini: &MiniPlayer) {
        let Some(id) = self.now.as_ref().map(|t| t.video_id.clone()) else {
            mini.set_has_art(false);
            return;
        };
        let mut art = self.art.borrow_mut();
        if let Some((large_id, image, ..)) = &art.large
            && *large_id == id
        {
            mini.set_art(image.clone());
            mini.set_has_art(true);
            return;
        }
        mini.set_has_art(false);
        // The main window asks for its own (at its size) when it's open.
        if current_ui().is_some() {
            return;
        }
        let cover = match &self.cover {
            Cover::Pending => return,
            Cover::Video => None,
            Cover::Album(url) => Some(url.clone()),
        };
        art.large = None;
        let _ = art
            .tx
            .send(Cmd::FetchArt(id, ArtSize::Large, art::LARGE_MIN, cover));
    }
}

#[cfg(test)]
mod tests {
    use super::place;

    #[test]
    fn opens_under_a_menu_bar_icon_and_above_a_taskbar_one() {
        // macOS: icon at the top, 2x screen.
        let p = place((1000.0, 0.0, 44.0, 48.0), 560.0, 840.0, 12.0);
        assert_eq!((p.x, p.y), (742, 60));
        // Windows: icon in a bottom taskbar.
        let p = place((1800.0, 1040.0, 24.0, 40.0), 280.0, 420.0, 6.0);
        assert_eq!((p.x, p.y), (1672, 614));
        // Never off the left edge.
        assert_eq!(place((10.0, 0.0, 20.0, 20.0), 280.0, 420.0, 6.0).x, 0);
    }
}

/// KDE on Wayland: a one-off KWin script, loaded over D-Bus just before the
/// window shows, puts the mini player at the pointer — which is on the tray
/// icon that was just clicked — under it on a top panel, above it on a
/// bottom one, inside the screen. Checked 2026-11-28 on Plasma 6.3 (KWin
/// moved a test window, `workspace.cursorPos` and `clientArea` answered).
#[cfg(target_os = "linux")]
mod kwin {
    use std::time::Duration;

    const PLUGIN: &str = "yarek-mini-player";
    /// The script lives this long: enough for the window to appear.
    const LIFETIME: Duration = Duration::from_secs(3);

    pub fn wanted() -> bool {
        let var = |name| std::env::var(name).unwrap_or_default().to_lowercase();
        var("XDG_SESSION_TYPE") == "wayland" && var("XDG_CURRENT_DESKTOP").contains("kde")
    }

    /// Starts placing the window being opened; false when KWin can't be
    /// asked (the window then goes wherever the compositor puts it).
    pub fn place_at_pointer() -> bool {
        let path = std::env::temp_dir().join(format!("{PLUGIN}-{}.js", std::process::id()));
        if let Err(err) = std::fs::write(&path, script(std::process::id(), super::TITLE)) {
            tracing::debug!(%err, "mini player: KWin script");
            return false;
        }
        std::thread::spawn(move || {
            if let Err(err) = run(&path) {
                tracing::debug!(%err, "mini player: KWin placement");
            }
            let _ = std::fs::remove_file(&path);
        });
        true
    }

    fn run(path: &std::path::Path) -> zbus::Result<()> {
        let bus = zbus::blocking::Connection::session()?;
        // A script left from a run that stopped early would shadow ours.
        let loaded: bool = call(&bus, "isScriptLoaded", &(PLUGIN,))?
            .body()
            .deserialize()?;
        if loaded {
            call(&bus, "unloadScript", &(PLUGIN,))?;
        }
        call(
            &bus,
            "loadScript",
            &(path.to_string_lossy().as_ref(), PLUGIN),
        )?;
        call(&bus, "start", &())?;
        std::thread::sleep(LIFETIME);
        call(&bus, "unloadScript", &(PLUGIN,))?;
        Ok(())
    }

    /// A call to KWin's scripting object.
    fn call<B>(
        bus: &zbus::blocking::Connection,
        method: &str,
        body: &B,
    ) -> zbus::Result<zbus::message::Message>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
    {
        bus.call_method(
            Some("org.kde.KWin"),
            "/Scripting",
            Some("org.kde.kwin.Scripting"),
            method,
            body,
        )
    }

    /// The script: places our window (by pid and title) if it's there,
    /// else when it's added.
    fn script(pid: u32, title: &str) -> String {
        let title = serde_json::to_string(title).unwrap_or_default();
        format!(
            r#"const PID = {pid}, TITLE = {title}, GAP = 8, OFFSET = 28;
function place(w) {{
    if (w.pid !== PID || w.caption !== TITLE) return false;
    const c = workspace.cursorPos;
    const a = workspace.clientArea(KWin.MaximizeArea, w.output, workspace.currentDesktop);
    const g = w.frameGeometry;
    const below = c.y < a.y + a.height / 2;
    let x = c.x - g.width / 2;
    let y = below ? c.y + OFFSET : c.y - OFFSET - g.height;
    x = Math.max(a.x + GAP, Math.min(x, a.x + a.width - g.width - GAP));
    y = Math.max(a.y + GAP, Math.min(y, a.y + a.height - g.height - GAP));
    w.frameGeometry = {{ x: Math.round(x), y: Math.round(y), width: g.width, height: g.height }};
    return true;
}}
if (!workspace.windowList().some(place)) {{
    const added = function (w) {{ if (place(w)) workspace.windowAdded.disconnect(added); }};
    workspace.windowAdded.connect(added);
}}
"#
        )
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn the_script_names_our_window() {
            let script = super::script(4242, "Yarek \"mini\"");
            assert!(script.starts_with("const PID = 4242, TITLE = \"Yarek \\\"mini\\\"\","));
            assert!(script.contains("workspace.windowAdded.connect(added)"));
        }
    }
}
