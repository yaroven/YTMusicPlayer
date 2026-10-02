//! Menu bar / system tray icon: play-pause, next, previous, show, quit.
//! macOS and Windows use `tray-icon` (native, main thread); Linux uses
//! `ksni` (StatusNotifierItem over D-Bus, on the core runtime).
//!
//! Clicks arrive on other threads; [`Tray::new`] takes a handler that runs
//! them on the UI thread.

use std::sync::Arc;

use anyhow::{Context, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    TogglePause,
    Next,
    Prev,
    Show,
    Quit,
}

const ITEMS: [(&str, Action); 5] = [
    ("Play / Pause", Action::TogglePause),
    ("Next", Action::Next),
    ("Previous", Action::Prev),
    ("Show ytm-player", Action::Show),
    ("Quit", Action::Quit),
];

/// The app icon as RGBA pixels.
fn icon_rgba() -> Result<(Vec<u8>, u32, u32)> {
    let png = include_bytes!("../../assets/ytm-player-64.png");
    let img = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .context("decoding the tray icon")?
        .to_rgba8();
    let (w, h) = img.dimensions();
    Ok((img.into_raw(), w, h))
}

/// Forwards an action to the UI thread.
type Handler = Arc<dyn Fn(Action) + Send + Sync>;

fn on_ui_thread(handler: impl Fn(Action) + 'static) -> Handler {
    // The handler stays on the UI thread; other threads post actions to it.
    UI_HANDLER.with(|h| *h.borrow_mut() = Some(std::rc::Rc::new(handler)));
    Arc::new(|action| {
        let _ = slint::invoke_from_event_loop(move || {
            UI_HANDLER.with(|h| {
                if let Some(handler) = h.borrow().clone() {
                    handler(action);
                }
            });
        });
    })
}

type UiHandler = std::rc::Rc<dyn Fn(Action)>;

thread_local! {
    static UI_HANDLER: std::cell::RefCell<Option<UiHandler>> = const { std::cell::RefCell::new(None) };
}

#[cfg(not(target_os = "linux"))]
pub use native::Tray;

#[cfg(target_os = "linux")]
pub use sni::Tray;

#[cfg(not(target_os = "linux"))]
mod native {
    use tray_icon::{
        Icon, TrayIcon, TrayIconBuilder,
        menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem},
    };

    use super::*;

    pub struct Tray {
        icon: TrayIcon,
    }

    impl Tray {
        /// Must run on the main thread once the event loop is running.
        pub fn new(
            _rt: &tokio::runtime::Handle,
            handler: impl Fn(Action) + 'static,
        ) -> Result<Self> {
            let handler = on_ui_thread(handler);
            let menu = Menu::new();
            let mut ids: Vec<(MenuId, Action)> = Vec::new();
            for (i, (label, action)) in ITEMS.iter().enumerate() {
                if i == 3 {
                    menu.append(&PredefinedMenuItem::separator())?;
                }
                let item = MenuItem::new(*label, true, None);
                ids.push((item.id().clone(), *action));
                menu.append(&item)?;
            }
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                if let Some((_, action)) = ids.iter().find(|(id, _)| *id == event.id) {
                    handler(*action);
                }
            }));
            let (rgba, w, h) = icon_rgba()?;
            let icon = TrayIconBuilder::new()
                .with_menu(Box::new(menu))
                .with_tooltip("ytm-player")
                .with_icon(Icon::from_rgba(rgba, w, h)?)
                .build()?;
            Ok(Self { icon })
        }

        pub fn set_tooltip(&self, text: &str) {
            let _ = self.icon.set_tooltip(Some(text));
        }

        /// The icon is on screen (so a closed window can come back).
        pub fn available(&self) -> bool {
            true
        }
    }
}

#[cfg(target_os = "linux")]
mod sni {
    use ksni::{
        TrayMethods,
        menu::{MenuItem, StandardItem},
    };

    use super::*;

    struct Item {
        handler: Handler,
        tooltip: String,
        icon: ksni::Icon,
    }

    impl ksni::Tray for Item {
        fn id(&self) -> String {
            "ytm-player".into()
        }
        fn title(&self) -> String {
            "ytm-player".into()
        }
        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            vec![self.icon.clone()]
        }
        fn tool_tip(&self) -> ksni::ToolTip {
            ksni::ToolTip {
                title: self.tooltip.clone(),
                ..Default::default()
            }
        }
        fn activate(&mut self, _x: i32, _y: i32) {
            (self.handler)(Action::Show);
        }
        fn menu(&self) -> Vec<MenuItem<Self>> {
            let mut items = Vec::new();
            for (i, (label, action)) in ITEMS.iter().enumerate() {
                if i == 3 {
                    items.push(MenuItem::Separator);
                }
                let action = *action;
                items.push(
                    StandardItem {
                        label: (*label).into(),
                        activate: Box::new(move |this: &mut Self| (this.handler)(action)),
                        ..Default::default()
                    }
                    .into(),
                );
            }
            items
        }
    }

    pub struct Tray {
        handle: std::sync::Arc<std::sync::Mutex<Option<ksni::Handle<Item>>>>,
        rt: tokio::runtime::Handle,
    }

    impl Tray {
        /// Registers the item from the core runtime (D-Bus is async).
        pub fn new(
            rt: &tokio::runtime::Handle,
            handler: impl Fn(Action) + 'static,
        ) -> Result<Self> {
            let (rgba, w, h) = icon_rgba()?;
            // RGBA -> ARGB, as StatusNotifierItem wants.
            let mut argb = rgba;
            for px in argb.chunks_exact_mut(4) {
                px.rotate_right(1);
            }
            let item = Item {
                handler: on_ui_thread(handler),
                tooltip: "ytm-player".into(),
                icon: ksni::Icon {
                    width: w as i32,
                    height: h as i32,
                    data: argb,
                },
            };
            let handle = std::sync::Arc::new(std::sync::Mutex::new(None));
            let slot = handle.clone();
            rt.spawn(async move {
                match item.spawn().await {
                    Ok(h) => *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(h),
                    Err(err) => tracing::warn!(%err, "tray icon (no StatusNotifier host?)"),
                }
            });
            Ok(Self {
                handle,
                rt: rt.clone(),
            })
        }

        /// Registered with a StatusNotifier host (GNOME without the
        /// AppIndicator extension has none).
        pub fn available(&self) -> bool {
            self.handle
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_some()
        }

        pub fn set_tooltip(&self, text: &str) {
            let handle = self.handle.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(h) = handle.as_ref() {
                let (h, text) = (h.clone(), text.to_owned());
                self.rt.spawn(async move {
                    h.update(|item| item.tooltip = text).await;
                });
            }
        }
    }
}
