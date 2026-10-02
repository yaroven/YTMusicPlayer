//! Lyrics for a track: time-synced from LRCLIB (lrclib.net, free and open)
//! when it knows the song, else YouTube Music's plain lyrics.
//!
//! YouTube titles are noisy ("ABBA - Dancing Queen (Official Video)"), so
//! the query is cleaned up first and the match is checked by duration.

use std::time::Duration;

use anyhow::Result;
use serde::Deserialize;

use crate::{api::models::Track, catalog::Catalog};

const LRCLIB: &str = "https://lrclib.net/api/search";
/// A lyrics entry this far off the track's length is another recording.
const DURATION_SLACK: f64 = 4.0;

/// One line; `at` is when it starts (synced lyrics only).
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub at: Option<Duration>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Lyrics {
    pub lines: Vec<Line>,
    /// "LRCLIB" or YouTube Music's credit ("Source: Musixmatch").
    pub source: String,
}

impl Lyrics {
    pub fn synced(&self) -> bool {
        self.lines.first().is_some_and(|l| l.at.is_some())
    }

    /// Index of the line being sung at `position` (synced lyrics).
    pub fn current_line(&self, position: Duration) -> Option<usize> {
        if !self.synced() {
            return None;
        }
        self.lines
            .iter()
            .rposition(|l| l.at.is_some_and(|at| at <= position))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LrclibEntry {
    duration: Option<f64>,
    synced_lyrics: Option<String>,
    plain_lyrics: Option<String>,
}

/// Synced lyrics if LRCLIB has them, else YouTube Music's, else `None`.
pub async fn find(
    http: &reqwest::Client,
    catalog: &Catalog,
    track: &Track,
    duration: Option<Duration>,
) -> Result<Option<Lyrics>> {
    match lrclib(http, track, duration).await {
        Ok(Some(lyrics)) if lyrics.synced() => return Ok(Some(lyrics)),
        Ok(_) => {}
        Err(err) => tracing::debug!(%err, "LRCLIB"),
    }
    Ok(catalog.lyrics(&track.video_id).await?.map(|l| Lyrics {
        lines: l
            .text
            .lines()
            .map(|text| Line {
                at: None,
                text: text.to_owned(),
            })
            .collect(),
        source: l.source,
    }))
}

async fn lrclib(
    http: &reqwest::Client,
    track: &Track,
    duration: Option<Duration>,
) -> Result<Option<Lyrics>> {
    let (artist, title) = clean_title(&track.artist, &track.title);
    let entries: Vec<LrclibEntry> = http
        .get(LRCLIB)
        .query(&[
            ("track_name", title.as_str()),
            ("artist_name", artist.as_str()),
        ])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let length = duration.map(|d| d.as_secs_f64());
    let fits = |e: &LrclibEntry| match (length, e.duration) {
        (Some(want), Some(have)) => (want - have).abs() <= DURATION_SLACK,
        _ => true,
    };
    let best = entries
        .iter()
        .filter(|e| fits(e))
        .find(|e| e.synced_lyrics.as_deref().is_some_and(|s| !s.is_empty()))
        .or_else(|| {
            entries
                .iter()
                .filter(|e| fits(e))
                .find(|e| e.plain_lyrics.is_some())
        });
    Ok(best.and_then(|e| {
        let lines = match (&e.synced_lyrics, &e.plain_lyrics) {
            (Some(lrc), _) if !lrc.is_empty() => parse_lrc(lrc),
            (_, Some(plain)) => plain
                .lines()
                .map(|t| Line {
                    at: None,
                    text: t.to_owned(),
                })
                .collect(),
            _ => return None,
        };
        (!lines.is_empty()).then(|| Lyrics {
            lines,
            source: "LRCLIB".into(),
        })
    }))
}

/// `[mm:ss.xx] text` lines; metadata tags (`[ar:…]`) and blanks between
/// timestamps are kept only when timed.
fn parse_lrc(lrc: &str) -> Vec<Line> {
    lrc.lines()
        .filter_map(|line| {
            let rest = line.strip_prefix('[')?;
            let (stamp, text) = rest.split_once(']')?;
            let (min, sec) = stamp.split_once(':')?;
            let at =
                Duration::from_secs_f64(min.parse::<f64>().ok()? * 60.0 + sec.parse::<f64>().ok()?);
            Some(Line {
                at: Some(at),
                text: text.trim().to_owned(),
            })
        })
        .collect()
}

/// Artist and song title without YouTube decorations: "ABBA - Dancing Queen
/// (Official Music Video)" by "ABBA" → ("ABBA", "Dancing Queen").
fn clean_title(artist: &str, title: &str) -> (String, String) {
    let mut artist = artist.trim().to_owned();
    let mut title = title.trim().to_owned();
    // "Artist - Title" in the title (music videos).
    if let Some((left, right)) = title.split_once(" - ") {
        if artist.is_empty()
            || left.to_lowercase().contains(&artist.to_lowercase())
            || artist.ends_with("VEVO")
        {
            artist = left.trim().to_owned();
            title = right.trim().to_owned();
        }
    }
    // Drop bracketed decorations: (Official Video), [Lyrics], (Remastered 2009)…
    loop {
        let cut = ["(", "["].iter().filter_map(|open| {
            let start = title.find(open)?;
            let close = if *open == "(" { ')' } else { ']' };
            let end = title[start..].find(close)? + start;
            Some((start, end))
        });
        let Some((start, end)) = cut.min() else {
            break;
        };
        title.replace_range(start..=end, "");
    }
    for noise in ["ft.", "feat."] {
        if let Some(i) = title.to_lowercase().find(noise) {
            title.truncate(i);
        }
    }
    (artist.trim().to_owned(), title.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_music_video_titles() {
        assert_eq!(
            clean_title(
                "ABBA",
                "ABBA - Dancing Queen (Official Music Video Remastered)"
            ),
            ("ABBA".into(), "Dancing Queen".into())
        );
        assert_eq!(
            clean_title(
                "RickAstleyVEVO",
                "Rick Astley - Never Gonna Give You Up [4K]"
            ),
            ("Rick Astley".into(), "Never Gonna Give You Up".into())
        );
        assert_eq!(
            clean_title("Daft Punk", "Get Lucky feat. Pharrell Williams"),
            ("Daft Punk".into(), "Get Lucky".into())
        );
        assert_eq!(
            clean_title("Queen", "Bohemian Rhapsody"),
            ("Queen".into(), "Bohemian Rhapsody".into())
        );
    }

    #[test]
    fn parses_lrc_and_finds_the_current_line() {
        let lyrics = Lyrics {
            lines: parse_lrc(
                "[ar:ABBA]\n[00:17.37] Ooh, you can dance\n[00:25.18] Having the time\n[01:02.00]",
            ),
            source: "LRCLIB".into(),
        };
        assert_eq!(
            lyrics.lines.len(),
            3,
            "tags without a timestamp are dropped"
        );
        assert!(lyrics.synced());
        assert_eq!(lyrics.current_line(Duration::from_secs(10)), None);
        assert_eq!(lyrics.current_line(Duration::from_secs(20)), Some(0));
        assert_eq!(lyrics.current_line(Duration::from_secs(30)), Some(1));
    }
}
