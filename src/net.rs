//! Telling "no internet" and "too slow" apart from other failures, and
//! noticing when the connection is back.
//!
//! The session asks [`problem`] about every failed request: a network
//! problem makes it wait for the connection (and retry what failed) instead
//! of skipping through the queue or showing a raw error chain.

use std::time::Duration;

use crate::i18n::Lang;
use crate::{tr, trf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    /// No connection: DNS fails, nothing answers.
    Offline,
    /// Connected, but the answer didn't come in time.
    Slow,
    Other,
}

/// Phrases in errors (reqwest's, std's, and yt-dlp's Python ones) that mean
/// the request never reached the server or its answer never came back.
const OFFLINE: [&str; 17] = [
    "unable to download webpage",
    "nodename nor servname",
    "name or service not known",
    "temporary failure in name resolution",
    "getaddrinfo failed",
    "failed to resolve",
    "failed to lookup address",
    "dns error",
    "network is unreachable",
    "no route to host",
    "connection refused",
    "connection reset",
    "connection aborted",
    "urlopen error",
    "error sending request",
    "network is down",
    "internet connection appears to be offline",
];

const SLOW: [&str; 3] = ["timed out", "timeout", "deadline has elapsed"];

/// What kind of failure `err` is, from its whole chain.
pub fn problem(err: &anyhow::Error) -> Problem {
    for cause in err.chain() {
        if let Some(e) = cause.downcast_ref::<reqwest::Error>() {
            if e.is_timeout() {
                return Problem::Slow;
            }
            if e.is_connect() {
                return Problem::Offline;
            }
        }
        if let Some(e) = cause.downcast_ref::<std::io::Error>() {
            use std::io::ErrorKind::*;
            match e.kind() {
                TimedOut => return Problem::Slow,
                ConnectionRefused | ConnectionReset | ConnectionAborted | NotConnected
                | AddrNotAvailable | NetworkUnreachable | HostUnreachable | NetworkDown => {
                    return Problem::Offline;
                }
                _ => {}
            }
        }
        if let Some(crate::audio::extractor::ExtractorError::Timeout(_)) =
            cause.downcast_ref::<crate::audio::extractor::ExtractorError>()
        {
            return Problem::Slow;
        }
    }
    problem_in_text(&format!("{err:#}"))
}

/// The same, from an error's text (player errors arrive as strings).
pub fn problem_in_text(message: &str) -> Problem {
    let lower = message.to_lowercase();
    if OFFLINE.iter().any(|p| lower.contains(p)) {
        Problem::Offline
    } else if SLOW.iter().any(|p| lower.contains(p)) {
        Problem::Slow
    } else {
        Problem::Other
    }
}

pub fn is_network(err: &anyhow::Error) -> bool {
    problem(err) != Problem::Other
}

/// One line for the status bar: "No internet connection", or the error.
pub fn describe(lang: Lang, err: &anyhow::Error) -> String {
    match problem(err) {
        Problem::Offline => offline_text(lang).to_owned(),
        Problem::Slow => tr!(
            lang,
            "The connection is too slow",
            "Інтернет надто повільний"
        )
        .to_owned(),
        Problem::Other => format!("{err:#}"),
    }
}

pub fn offline_text(lang: Lang) -> &'static str {
    tr!(lang, "No internet connection", "Немає інтернету")
}

/// "Search failed: No internet connection".
pub fn failed(lang: Lang, what_en: &str, what_uk: &str, err: &anyhow::Error) -> String {
    let reason = describe(lang, err);
    trf!(lang, "{what_en}: {reason}", "{what_uk}: {reason}")
}

/// Whether YouTube Music answers at all: a TCP connection to it within a
/// few seconds (DNS included). Cheap enough to poll while offline.
pub async fn online() -> bool {
    matches!(
        tokio::time::timeout(
            Duration::from_secs(4),
            tokio::net::TcpStream::connect(("music.youtube.com", 443)),
        )
        .await,
        Ok(Ok(_))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_failures() {
        let dns = anyhow::anyhow!(
            "yt-dlp exited with exit status: 1: ERROR: [youtube] abc: Unable to download webpage: <urlopen error [Errno 8] nodename nor servname provided, or not known>"
        );
        assert_eq!(problem(&dns), Problem::Offline);
        let io = anyhow::Error::from(std::io::Error::from(std::io::ErrorKind::TimedOut));
        assert_eq!(problem(&io), Problem::Slow);
        let refused =
            anyhow::Error::from(std::io::Error::from(std::io::ErrorKind::ConnectionRefused))
                .context("opening the stream");
        assert_eq!(problem(&refused), Problem::Offline);
        assert_eq!(
            problem(&anyhow::anyhow!("video unavailable: Private video")),
            Problem::Other
        );
        assert_eq!(
            problem_in_text("media request failed: error sending request for url"),
            Problem::Offline
        );
    }

    #[test]
    fn describes_in_both_languages() {
        let err = anyhow::anyhow!("dns error: failed to lookup address information");
        assert_eq!(describe(Lang::En, &err), "No internet connection");
        assert_eq!(describe(Lang::Uk, &err), "Немає інтернету");
        assert_eq!(
            failed(Lang::En, "Search failed", "Пошук не вдався", &err),
            "Search failed: No internet connection"
        );
        let other = anyhow::anyhow!("HTTP 500");
        assert_eq!(describe(Lang::En, &other), "HTTP 500");
    }
}
