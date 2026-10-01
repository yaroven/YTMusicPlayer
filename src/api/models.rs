//! Domain types shared by API, storage, audio queue and UI.

/// Pseudo playlist id for the "Liked" list.
pub const LIKED_PLAYLIST_ID: &str = "__liked__";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Playlist {
    pub id: String,
    pub title: String,
    pub item_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    pub video_id: String,
    pub title: String,
    pub artist: String,
    pub duration_secs: Option<u32>,
}

/// "Artist - Topic" is how YouTube names auto-generated music channels.
pub fn clean_artist(channel: &str) -> String {
    channel
        .strip_suffix(" - Topic")
        .unwrap_or(channel)
        .to_owned()
}

/// Parses ISO 8601 durations as returned by the API (`PT1H2M3S`, `P1DT2S`).
pub fn parse_iso_duration(s: &str) -> Option<u32> {
    let rest = s.strip_prefix('P')?;
    let (mut total, mut num, mut in_time) = (0u32, 0u32, false);
    for c in rest.chars() {
        match c {
            'T' => in_time = true,
            '0'..='9' => num = num.checked_mul(10)?.checked_add(c.to_digit(10)?)?,
            'D' => (total, num) = (total + num * 86_400, 0),
            'H' if in_time => (total, num) = (total + num * 3600, 0),
            'M' if in_time => (total, num) = (total + num * 60, 0),
            'S' if in_time => (total, num) = (total + num, 0),
            _ => return None,
        }
    }
    Some(total)
}

/// Accepts a bare 11-char id or a youtube.com / music.youtube.com / youtu.be URL.
pub fn video_id_from_input(input: &str) -> Option<String> {
    let input = input.trim();
    let is_id = |s: &str| {
        s.len() == 11
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    };
    if is_id(input) {
        return Some(input.to_owned());
    }
    let url = url::Url::parse(input).ok()?;
    let id = match url.host_str()? {
        "youtu.be" => url.path().trim_start_matches('/').to_owned(),
        _ => url
            .query_pairs()
            .find(|(k, _)| k == "v")
            .map(|(_, v)| v.into_owned())?,
    };
    is_id(&id).then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_durations() {
        assert_eq!(parse_iso_duration("PT3M33S"), Some(213));
        assert_eq!(parse_iso_duration("PT1H"), Some(3600));
        assert_eq!(parse_iso_duration("P1DT1S"), Some(86_401));
        assert_eq!(parse_iso_duration("PT0S"), Some(0));
        assert_eq!(parse_iso_duration("3M"), None);
    }

    #[test]
    fn video_ids_from_urls() {
        let id = Some("dQw4w9WgXcQ".to_owned());
        assert_eq!(video_id_from_input("dQw4w9WgXcQ"), id);
        assert_eq!(
            video_id_from_input("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=1"),
            id
        );
        assert_eq!(
            video_id_from_input("https://music.youtube.com/watch?v=dQw4w9WgXcQ"),
            id
        );
        assert_eq!(video_id_from_input("https://youtu.be/dQw4w9WgXcQ"), id);
        assert_eq!(video_id_from_input("https://example.com/"), None);
    }

    #[test]
    fn topic_suffix_removed() {
        assert_eq!(clean_artist("Daft Punk - Topic"), "Daft Punk");
        assert_eq!(clean_artist("Rick Astley"), "Rick Astley");
    }
}
