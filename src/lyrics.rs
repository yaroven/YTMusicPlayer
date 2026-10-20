//! Lyrics for a track: time-synced from LRCLIB (lrclib.net, free and open)
//! when it knows the song, else YouTube Music's plain lyrics.
//!
//! YouTube titles are noisy ("ABBA - Dancing Queen (Official Video)"), so
//! the query is cleaned up first and the match is checked by duration.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::{api::models::Track, catalog::Catalog};

const LRCLIB: &str = "https://lrclib.net/api/search";
const LRCLIB_GET: &str = "https://lrclib.net/api/get";
const LRCLIB_ATTEMPTS: u32 = 3;
/// A lyrics entry this far off the track's length is another recording.
const DURATION_SLACK: f64 = 4.0;

/// One line; `at` is when it starts (synced lyrics only).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Line {
    pub at: Option<Duration>,
    pub text: String,
    /// Word timings when the lyrics have them (Enhanced LRC); without them
    /// the whole line lights up at once, see [`Lyrics::progress`].
    pub words: Vec<Word>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// Characters of the line lit so far: every word that has started,
    /// whole (a word lights up at once, not letter by letter — the user's
    /// choice, 2026-10-18); the whole line when only lines are timed.
    pub chars: usize,
}

/// A gap this long before the first line gets an instrumental line.
pub const INTRO_GAP: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lyrics {
    pub lines: Vec<Line>,
    /// "LRCLIB" or YouTube Music's credit ("Source: Musixmatch").
    pub source: String,
}

impl Lyrics {
    pub fn synced(&self) -> bool {
        self.lines.first().is_some_and(|l| l.at.is_some())
    }

    /// When each word of line `i` starts, when the lyrics time words
    /// (Enhanced LRC); empty otherwise — guessing them from the words'
    /// lengths lit words out of step with the singing (the user's call,
    /// 2026-10-20: lines then, not words).
    pub fn word_times(&self, i: usize) -> Vec<Duration> {
        let Some(line) = self.lines.get(i) else {
            return Vec::new();
        };
        let words = line.split_words();
        if line.words.len() == words.len() && !words.is_empty() {
            return line.words.iter().map(|w| w.at).collect();
        }
        Vec::new()
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
                chars: lit.min(3),
            });
        }
        let times = self.word_times(line);
        let words = current.split_words();
        // Only lines are timed: the line lights up whole.
        if times.is_empty() {
            return Some(Progress {
                line,
                words: words.len(),
                chars: current.text.chars().count(),
            });
        }
        let sung = times.iter().filter(|at| **at <= position).count();
        let chars: usize = words[..sung.min(words.len())]
            .iter()
            .map(|w| w.chars().count())
            .sum();
        Some(Progress {
            line,
            words: sung,
            chars,
        })
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

/// Lyrics kept on disk (`<data_dir>/lyrics/<video id>.json`): every
/// downloaded song's, so they show offline, and the last [`Cache::RECENT`]
/// others, so a song played again doesn't ask LRCLIB again.
#[derive(Clone, Default)]
pub struct Cache {
    dir: Option<PathBuf>,
}

impl Cache {
    pub const RECENT: usize = 50;

    pub fn new(dir: PathBuf) -> Self {
        Self { dir: Some(dir) }
    }

    fn file(&self, video_id: &str) -> Option<PathBuf> {
        let dir = self.dir.as_ref()?;
        // Video ids are 11 URL-safe characters; nothing else names a file.
        let valid = video_id.len() == 11
            && video_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        valid.then(|| dir.join(format!("{video_id}.json")))
    }

    pub fn get(&self, video_id: &str) -> Option<Lyrics> {
        let data = std::fs::read(self.file(video_id)?).ok()?;
        serde_json::from_slice(&data).ok()
    }

    pub fn contains(&self, video_id: &str) -> bool {
        self.file(video_id).is_some_and(|f| f.exists())
    }

    /// Saves `lyrics`, then drops the oldest that `keep` (the downloaded
    /// songs) doesn't name, beyond [`Cache::RECENT`].
    pub fn put(&self, video_id: &str, lyrics: &Lyrics, keep: impl Fn(&str) -> bool) {
        let (Some(dir), Some(file)) = (self.dir.as_ref(), self.file(video_id)) else {
            return;
        };
        let Ok(data) = serde_json::to_vec(lyrics) else {
            return;
        };
        if std::fs::create_dir_all(dir).is_err() || std::fs::write(&file, data).is_err() {
            return;
        }
        prune(dir, &keep);
    }

    pub fn remove(&self, video_id: &str) {
        if let Some(file) = self.file(video_id) {
            let _ = std::fs::remove_file(file);
        }
    }
}

fn prune(dir: &Path, keep: &impl Fn(&str) -> bool) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut others: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let id = path.file_stem()?.to_str()?.to_owned();
            if keep(&id) {
                return None;
            }
            Some((e.metadata().ok()?.modified().ok()?, path))
        })
        .collect();
    if others.len() <= Cache::RECENT {
        return;
    }
    others.sort_by_key(|o| std::cmp::Reverse(o.0));
    for (_, path) in others.into_iter().skip(Cache::RECENT) {
        let _ = std::fs::remove_file(path);
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

/// One LRCLIB request; `None` when it has nothing (404). It answers 503
/// now and then: retried a few times, so a busy moment doesn't leave the
/// song with untimed lyrics.
async fn lrclib_get<T: serde::de::DeserializeOwned>(
    http: &reqwest::Client,
    url: &str,
    query: &[(&str, &str)],
) -> Result<Option<T>> {
    let mut wait = Duration::from_millis(400);
    for attempt in 1..=LRCLIB_ATTEMPTS {
        let result = http
            .get(url)
            .query(query)
            .timeout(Duration::from_secs(10))
            .send()
            .await;
        match result {
            Ok(r) if r.status() == reqwest::StatusCode::NOT_FOUND => return Ok(None),
            Ok(r) if r.status().is_success() => return Ok(Some(r.json().await?)),
            Ok(r)
                if attempt < LRCLIB_ATTEMPTS
                    && (r.status().is_server_error() || r.status() == 429) => {}
            Err(err)
                if attempt < LRCLIB_ATTEMPTS
                    && (err.is_timeout() || err.is_connect() || err.is_request()) => {}
            Ok(r) => return Err(r.error_for_status().unwrap_err().into()),
            Err(err) => return Err(err.into()),
        }
        tokio::time::sleep(wait).await;
        wait *= 2;
    }
    Ok(None)
}

async fn lrclib(
    http: &reqwest::Client,
    track: &Track,
    duration: Option<Duration>,
) -> Result<Option<Lyrics>> {
    let (artist, title) = clean_title(&track.artist, &track.title);
    let query = [
        ("track_name", title.as_str()),
        ("artist_name", artist.as_str()),
    ];
    // The exact match first (LRCLIB caches it), then a search.
    let mut entries = Vec::new();
    if let Some(d) = duration {
        let secs = d.as_secs().to_string();
        let mut exact = query.to_vec();
        exact.push(("duration", secs.as_str()));
        match lrclib_get::<LrclibEntry>(http, LRCLIB_GET, &exact).await {
            Ok(Some(entry)) => entries.push(entry),
            Ok(None) => {}
            // Only a shortcut (LRCLIB caches it): when it fails — 503s are
            // common — the search below still has a chance.
            Err(err) => tracing::debug!(%err, "LRCLIB exact lookup"),
        }
    }
    if !entries
        .iter()
        .any(|e| e.synced_lyrics.as_deref().is_some_and(|s| !s.is_empty()))
    {
        entries.extend(
            lrclib_get::<Vec<LrclibEntry>>(http, LRCLIB, &query)
                .await?
                .unwrap_or_default(),
        );
    }
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
pub(crate) fn clean_title(artist: &str, title: &str) -> (String, String) {
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
    fn karaoke_progress_by_line_or_word() {
        let lyrics = Lyrics {
            lines: parse_lrc("[00:10.00] One two three\n[00:20.00]\n[00:30.00] Last"),
            source: "LRCLIB".into(),
        };
        let at = |s: f64| Duration::from_secs_f64(s);
        // Only lines are timed: no guessed word times, the line lights up whole.
        assert!(lyrics.word_times(0).is_empty());
        let p = lyrics.progress(at(10.1)).unwrap();
        assert_eq!((p.line, p.words, p.chars), (0, 3, 13), "{p:?}");
        // The empty line is an instrumental break: dots light up over it.
        assert!(lyrics.lines[1].instrumental());
        let p = lyrics.progress(at(20.5)).unwrap();
        assert_eq!((p.line, p.words), (1, 0));
        let p = lyrics.progress(at(28.0)).unwrap();
        assert_eq!((p.line, p.words), (1, 3));

        // Enhanced LRC: the lyrics' own word times.
        let enhanced = Lyrics {
            lines: parse_lrc("[00:05.00] <00:05.00>Hel<00:05.50>lo <00:07.00>world"),
            source: "LRCLIB".into(),
        };
        assert_eq!(enhanced.lines[0].text, "Hello world");
        assert_eq!(enhanced.lines[0].split_words(), ["Hello ", "world"]);
        // Two stamps inside "Hello": not one per word, so the line lights up whole.
        assert!(enhanced.word_times(0).is_empty());
        assert_eq!(enhanced.progress(at(5.1)).unwrap().chars, 11);
        let exact = Lyrics {
            lines: parse_lrc("[00:05.00] <00:05.00>Hello <00:07.00>world"),
            source: "LRCLIB".into(),
        };
        assert_eq!(exact.word_times(0), [at(5.0), at(7.0)]);
        let p = exact.progress(at(6.0)).unwrap();
        assert_eq!((p.words, p.chars), (1, 6), "all of \"Hello \" at once");
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
            explicit: false,
        };
        let lyrics = find(&http, &catalog, &track, Some(Duration::from_secs(359)))
            .await
            .unwrap()
            .expect("lyrics");
        println!("{} lines from {}", lyrics.lines.len(), lyrics.source);
        assert!(lyrics.synced());
        assert!(lyrics.current_line(Duration::from_secs(60)).is_some());

        // Songs a user played whose lyrics came out untimed.
        for (title, artist, secs) in [
            ("Still Swingin'", "Papa Roach", 204),
            ("Movie Blood", "Powerman 5000", 194),
            (
                "Aviators - Sweet Dreams (Five Nights At Freddy's 4 Song)",
                "Aviators",
                322,
            ),
        ] {
            let track = Track {
                video_id: "dQw4w9WgXcQ".into(),
                title: title.into(),
                artist: artist.into(),
                duration_secs: Some(secs),
                explicit: false,
            };
            let lyrics = lrclib(&http, &track, Some(Duration::from_secs(secs.into())))
                .await
                .unwrap()
                .expect(title);
            assert!(lyrics.synced(), "{title}");
        }
    }

    #[test]
    fn cache_keeps_downloads_and_the_last_ones() {
        let dir = std::env::temp_dir().join(format!("yarek-lyrics-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cache = Cache::new(dir.clone());
        let lyrics = Lyrics {
            lines: parse_lrc("[00:01.00] <00:01.00>Hello <00:02.00>world"),
            source: "LRCLIB".into(),
        };
        let downloaded = "DLDLDLDLDLD";
        cache.put(downloaded, &lyrics, |_| true);
        assert_eq!(
            cache.get(downloaded),
            Some(lyrics.clone()),
            "round trip, word times too"
        );
        assert!(cache.get("../../etc/xx").is_none() && !cache.contains("bad"));
        for i in 0..Cache::RECENT + 5 {
            let id = format!("v{i:010}");
            cache.put(&id, &lyrics, |id| id == downloaded);
            // Distinct modification times for the pruning order.
            std::thread::sleep(Duration::from_millis(2));
        }
        let files = std::fs::read_dir(&dir).unwrap().count();
        assert_eq!(
            files,
            Cache::RECENT + 1,
            "the last {} and the download",
            Cache::RECENT
        );
        assert!(cache.contains(downloaded));
        assert!(
            !cache.contains("v0000000000")
                && cache.contains(&format!("v{:010}", Cache::RECENT + 4))
        );
        cache.remove(downloaded);
        assert!(!cache.contains(downloaded));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
