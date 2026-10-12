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
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Line {
    pub at: Option<Duration>,
    pub text: String,
    /// Word timings when the lyrics have them (Enhanced LRC); else they're
    /// estimated, see [`Lyrics::progress`].
    pub words: Vec<Word>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    pub at: Duration,
    pub text: String,
}

impl Line {
    /// No words: an instrumental break ("", "♪").
    pub fn instrumental(&self) -> bool {
        self.text
            .chars()
            .all(|c| c.is_whitespace() || "♪♫♬…•".contains(c))
    }

    /// The line's words, each with its trailing space.
    pub fn split_words(&self) -> Vec<&str> {
        let mut words = Vec::new();
        let mut start = 0;
        let text = self.text.as_str();
        let bytes = text.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            // A word ends after its trailing whitespace.
            if bytes[i].is_ascii_whitespace() {
                while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                words.push(&text[start..i]);
                start = i;
            } else {
                i += 1;
            }
        }
        if start < text.len() {
            words.push(&text[start..]);
        }
        words
    }
}

/// Karaoke state at a moment: the line being sung and how far.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub line: usize,
    /// Words sung so far (instrumental lines: dots lit, 0..=3).
    pub words: usize,
}

/// Estimated singing pace when the lyrics only time lines.
const PER_CHAR: Duration = Duration::from_millis(85);
const LINE_EXTRA: Duration = Duration::from_millis(400);
/// A gap this long before the first line gets an instrumental line.
pub const INTRO_GAP: Duration = Duration::from_secs(4);

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

    /// When each word of line `i` starts: the lyrics' own word timings,
    /// else spread by length over the time the line is sung (it ends
    /// before the next line, and isn't sung slower than [`PER_CHAR`]).
    pub fn word_times(&self, i: usize) -> Vec<Duration> {
        let Some(line) = self.lines.get(i) else {
            return Vec::new();
        };
        let words = line.split_words();
        if line.words.len() == words.len() && !words.is_empty() {
            return line.words.iter().map(|w| w.at).collect();
        }
        let Some(start) = line.at else {
            return Vec::new();
        };
        let chars: Vec<usize> = words
            .iter()
            .map(|w| w.trim().chars().count().max(1))
            .collect();
        let total: usize = chars.iter().sum();
        let mut span = PER_CHAR * total as u32 + LINE_EXTRA;
        if let Some(next) = self.lines.get(i + 1).and_then(|l| l.at) {
            let gap = next.saturating_sub(start);
            span = span
                .min(gap.saturating_sub(Duration::from_millis(200)))
                .max(gap.min(Duration::from_millis(500)));
        }
        let mut before = 0;
        chars
            .iter()
            .map(|&c| {
                let at = start + span.mul_f64(before as f64 / total.max(1) as f64);
                before += c;
                at
            })
            .collect()
    }

    /// The line sung at `position` and how many of its words are.
    pub fn progress(&self, position: Duration) -> Option<Progress> {
        let line = self.current_line(position)?;
        let current = &self.lines[line];
        if current.instrumental() {
            let start = current.at.unwrap_or_default();
            let end = self
                .lines
                .get(line + 1)
                .and_then(|l| l.at)
                .unwrap_or(start + Duration::from_secs(6));
            let span = end.saturating_sub(start).as_secs_f64().max(0.1);
            let lit = (position.saturating_sub(start).as_secs_f64() / span * 4.0) as usize;
            return Some(Progress {
                line,
                words: lit.min(3),
            });
        }
        let words = self
            .word_times(line)
            .iter()
            .filter(|at| **at <= position)
            .count();
        Some(Progress { line, words })
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
                words: Vec::new(),
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
                    words: Vec::new(),
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

/// [`parse_lrc`] for the window's tests.
#[cfg(all(test, feature = "gui"))]
pub(crate) fn parse_lrc_for_tests(lrc: &str) -> Vec<Line> {
    parse_lrc(lrc)
}

fn lrc_time(stamp: &str) -> Option<Duration> {
    let (min, sec) = stamp.split_once(':')?;
    Some(Duration::from_secs_f64(
        min.parse::<f64>().ok()? * 60.0 + sec.parse::<f64>().ok()?,
    ))
}

/// `[mm:ss.xx] text` lines; metadata tags (`[ar:…]`) and blanks between
/// timestamps are kept only when timed. Enhanced LRC word stamps
/// (`<mm:ss.xx>word`) become [`Line::words`].
fn parse_lrc(lrc: &str) -> Vec<Line> {
    lrc.lines()
        .filter_map(|line| {
            let rest = line.strip_prefix('[')?;
            let (stamp, body) = rest.split_once(']')?;
            let at = lrc_time(stamp)?;
            let mut words = Vec::new();
            let mut text = String::new();
            for part in body.split('<') {
                match part
                    .split_once('>')
                    .and_then(|(t, w)| Some((lrc_time(t)?, w)))
                {
                    Some((when, word)) => {
                        if !word.trim().is_empty() {
                            words.push(Word {
                                at: when,
                                text: word.to_owned(),
                            });
                        }
                        text.push_str(word);
                    }
                    None => text.push_str(part),
                }
            }
            Some(Line {
                at: Some(at),
                text: text.trim().to_owned(),
                words,
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
    fn karaoke_progress_by_word() {
        let lyrics = Lyrics {
            lines: parse_lrc("[00:10.00] One two three\n[00:20.00]\n[00:30.00] Last"),
            source: "LRCLIB".into(),
        };
        let at = |s: f64| Duration::from_secs_f64(s);
        // Spread by length: "One" at 10 s, then "two", "three" within ~1.5 s.
        let times = lyrics.word_times(0);
        assert_eq!(times.len(), 3);
        assert_eq!(times[0], at(10.0));
        assert!(times[2] > times[1] && times[2] < at(12.0), "{times:?}");
        assert_eq!(
            lyrics.progress(at(10.1)),
            Some(Progress { line: 0, words: 1 })
        );
        assert_eq!(
            lyrics.progress(at(15.0)),
            Some(Progress { line: 0, words: 3 })
        );
        // The empty line is an instrumental break: dots light up over it.
        assert!(lyrics.lines[1].instrumental());
        assert_eq!(
            lyrics.progress(at(20.5)),
            Some(Progress { line: 1, words: 0 })
        );
        assert_eq!(
            lyrics.progress(at(28.0)),
            Some(Progress { line: 1, words: 3 })
        );

        // Enhanced LRC: the lyrics' own word times.
        let enhanced = Lyrics {
            lines: parse_lrc("[00:05.00] <00:05.00>Hel<00:05.50>lo <00:07.00>world"),
            source: "LRCLIB".into(),
        };
        assert_eq!(enhanced.lines[0].text, "Hello world");
        assert_eq!(enhanced.lines[0].split_words(), ["Hello ", "world"]);
        // Two stamps inside "Hello": fall back to the estimate for that line.
        assert_eq!(enhanced.word_times(0).len(), 2);
        let exact = Lyrics {
            lines: parse_lrc("[00:05.00] <00:05.00>Hello <00:07.00>world"),
            source: "LRCLIB".into(),
        };
        assert_eq!(exact.word_times(0), [at(5.0), at(7.0)]);
        assert_eq!(
            exact.progress(at(6.0)),
            Some(Progress { line: 0, words: 1 })
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

#[cfg(test)]
mod live {
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn finds_synced_lyrics_live() {
        let http = reqwest::Client::new();
        let catalog = Catalog::new(http.clone());
        let track = Track {
            video_id: "fJ9rUzIMcZQ".into(),
            title: "Bohemian Rhapsody".into(),
            artist: "Queen".into(),
            duration_secs: Some(359),
        };
        let lyrics = find(&http, &catalog, &track, Some(Duration::from_secs(359)))
            .await
            .unwrap()
            .expect("lyrics");
        println!("{} lines from {}", lyrics.lines.len(), lyrics.source);
        assert!(lyrics.synced());
        assert!(lyrics.current_line(Duration::from_secs(60)).is_some());
    }
}
