//! Desktop notification when the track changes (window frontend).

use crate::api::models::Track;

/// Bundle id of the app `install.sh` creates (`~/Applications/ytm-player.app`).
#[cfg(target_os = "macos")]
const BUNDLE_ID: &str = "dev.ytm-player.app";

/// macOS posts notifications on behalf of an app bundle; outside one the
/// system asks the user to pick an application ("Where is use_default?").
/// So notify only when running from the bundle.
#[cfg(target_os = "macos")]
fn available() -> bool {
    static READY: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *READY.get_or_init(|| {
        let bundled = std::env::current_exe()
            .is_ok_and(|p| p.to_string_lossy().contains(".app/Contents/MacOS/"));
        bundled && notify_rust::set_application(BUNDLE_ID).is_ok()
    })
}

#[cfg(not(target_os = "macos"))]
fn available() -> bool {
    true
}

/// Shows "title — artist" without blocking the caller (the macOS and
/// Windows backends are synchronous).
pub fn track_changed(track: &Track) {
    if !available() {
        return;
    }
    let (title, artist) = (track.title.to_string(), track.artist.to_string());
    let spawned = std::thread::Builder::new()
        .name("notify".into())
        .stack_size(256 * 1024)
        .spawn(move || {
            let mut n = notify_rust::Notification::new();
            n.summary(&title).body(&artist).appname("ytm-player");
            #[cfg(all(unix, not(target_os = "macos")))]
            n.icon("ytm-player")
                .timeout(notify_rust::Timeout::Milliseconds(4000));
            if let Err(err) = n.show() {
                tracing::debug!(%err, "notification");
            }
        });
    if let Err(err) = spawned {
        tracing::debug!(%err, "notification thread");
    }
}
