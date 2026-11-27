//! The clip's HLS playlists: which variant to take, and where each segment
//! starts on the clip's timeline.

use std::time::Duration;

/// An H.264 variant of the master playlist.
#[derive(Debug, Clone, PartialEq)]
pub struct Variant {
    pub height: u32,
    pub bandwidth: u64,
    pub url: String,
}

/// The H.264 variants (`avc1`) of a master playlist; VP9 ones are left
/// out, the decoders here are H.264 ones.
pub fn variants(master: &str, base: &str) -> Vec<Variant> {
    let mut found = Vec::new();
    let mut lines = master.lines();
    while let Some(line) = lines.next() {
        let Some(attrs) = line.strip_prefix("#EXT-X-STREAM-INF:") else {
            continue;
        };
        let Some(url) = lines.next().map(str::trim).filter(|l| !l.starts_with('#')) else {
            continue;
        };
        if !attribute(attrs, "CODECS").is_some_and(|c| c.contains("avc1")) {
            continue;
        }
        let height = attribute(attrs, "RESOLUTION")
            .and_then(|r| r.split_once('x'))
            .and_then(|(_, h)| h.parse().ok());
        let bandwidth = attribute(attrs, "BANDWIDTH").and_then(|b| b.parse().ok());
        if let (Some(height), Some(bandwidth)) = (height, bandwidth) {
            found.push(Variant {
                height,
                bandwidth,
                url: absolute(base, url),
            });
        }
    }
    found
}

/// The smallest variant at least `height` tall (for a sharp picture), else
/// the tallest there is; the lightest of equal heights.
pub fn pick(variants: &[Variant], height: u32) -> Option<&Variant> {
    let key = |v: &&Variant| (v.height, v.bandwidth);
    variants
        .iter()
        .filter(|v| v.height >= height)
        .min_by_key(key)
        .or_else(|| {
            let tallest = variants.iter().map(|v| v.height).max()?;
            variants
                .iter()
                .filter(|v| v.height == tallest)
                .min_by_key(key)
        })
}

/// A media segment: where it starts on the clip's timeline, and its URL.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub start: Duration,
    pub url: String,
}

/// The segments of a media playlist, from their `#EXTINF` durations.
pub fn segments(playlist: &str, base: &str) -> Vec<Segment> {
    let mut found = Vec::new();
    let (mut start, mut next) = (Duration::ZERO, None);
    for line in playlist.lines().map(str::trim) {
        if let Some(info) = line.strip_prefix("#EXTINF:") {
            next = info
                .split(',')
                .next()
                .and_then(|d| d.parse::<f64>().ok())
                .filter(|d| d.is_finite() && *d >= 0.0)
                .map(Duration::from_secs_f64);
        } else if !line.is_empty() && !line.starts_with('#') {
            found.push(Segment {
                start,
                url: absolute(base, line),
            });
            start += next.take().unwrap_or_default();
        }
    }
    found
}

/// The segment playing at `at` (the first one before the clip starts).
pub fn segment_at(segments: &[Segment], at: Duration) -> usize {
    segments
        .iter()
        .rposition(|s| s.start <= at)
        .unwrap_or_default()
}

fn attribute<'a>(attrs: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = attrs;
    while !rest.is_empty() {
        let (key, after) = rest.split_once('=')?;
        let (value, tail) = match after.strip_prefix('"') {
            Some(quoted) => {
                let (value, tail) = quoted.split_once('"')?;
                (value, tail.strip_prefix(',').unwrap_or(tail))
            }
            None => after.split_once(',').unwrap_or((after, "")),
        };
        if key.trim() == name {
            return Some(value);
        }
        rest = tail;
    }
    None
}

fn absolute(base: &str, url: &str) -> String {
    url::Url::parse(base)
        .and_then(|b| b.join(url))
        .map_or_else(|_| url.to_owned(), String::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MASTER: &str = "#EXTM3U
#EXT-X-STREAM-INF:BANDWIDTH=300000,CODECS=\"avc1.4D4015,mp4a.40.2\",RESOLUTION=426x240
https://m/240.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=1500000,CODECS=\"avc1.4D401F,mp4a.40.2\",RESOLUTION=1280x720,FRAME-RATE=25
https://m/720.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=900000,CODECS=\"vp09.00.21.08,mp4a.40.2\",RESOLUTION=1280x720
https://m/720-vp9.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=3000000,CODECS=\"avc1.640028,mp4a.40.2\",RESOLUTION=1920x1080
https://m/1080.m3u8
";

    #[test]
    fn picks_an_h264_variant_for_the_window() {
        let all = variants(MASTER, "https://m/master.m3u8");
        assert_eq!(all.len(), 3, "VP9 left out");
        assert_eq!(pick(&all, 500).unwrap().url, "https://m/720.m3u8");
        assert_eq!(pick(&all, 200).unwrap().height, 240);
        assert_eq!(
            pick(&all, 2000).unwrap().height,
            1080,
            "the tallest there is"
        );
        assert!(pick(&[], 720).is_none());
    }

    #[test]
    fn places_segments_on_the_timeline() {
        let playlist = "#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXTINF:5.005,\nseg0.ts\n#EXTINF:5.005,\nseg1.ts\n#EXTINF:3.2,\nhttps://other/seg2.ts\n#EXT-X-ENDLIST\n";
        let segs = segments(playlist, "https://m/v/index.m3u8");
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0].url, "https://m/v/seg0.ts");
        assert_eq!(segs[2].url, "https://other/seg2.ts");
        assert_eq!(segs[1].start, Duration::from_secs_f64(5.005));
        assert_eq!(segment_at(&segs, Duration::from_secs(7)), 1);
        assert_eq!(segment_at(&segs, Duration::from_secs(60)), 2);
        assert_eq!(segment_at(&segs, Duration::ZERO), 0);
    }
}
