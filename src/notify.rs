//! Desktop notification when the track changes (window frontend).

use crate::api::models::Track;

/// Shows "title — artist" without blocking the caller (the macOS and
/// Windows backends are synchronous).
pub fn track_changed(track: &Track) {
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
