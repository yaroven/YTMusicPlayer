//! Age-restricted songs, signed in: YouTube gives no audio-only HTTPS
//! format without a PO token, but its HLS variants (itag 93+, MPEG-TS
//! segments with the full AAC 128 kbps audio of itag 140 inside) play.
//! This fetches the segments, keeps only the audio (ADTS frames) and
//! writes it to a file the player decodes and seeks like a download. No
//! ffmpeg: a minimal MPEG-TS demuxer (PAT → PMT → the AAC stream's PES).

use std::{path::Path, time::Duration};

use anyhow::{Context, Result, bail};
use futures_util::{StreamExt, TryStreamExt};

/// Segments fetched at once (~5 s each, ~150 KB at 240 kbps).
const PARALLEL: usize = 4;
/// Longest pause between retries of a segment that isn't ready yet.
const MAX_WAIT: Duration = Duration::from_secs(4);

/// Downloads the HLS media playlist's audio into `dest` (ADTS AAC).
pub async fn download_audio(
    http: &reqwest::Client,
    playlist: &str,
    headers: &[(String, String)],
    dest: &Path,
) -> Result<u64> {
    let get = |url: String| {
        let mut request = http.get(url);
        for (name, value) in headers {
            request = request.header(name, value);
        }
        request
    };
    let mut text = get(playlist.to_owned())
        .send()
        .await?
        .error_for_status()
        .context("HLS playlist")?
        .text()
        .await?;
    // A master playlist: take the first variant with AAC-LC audio.
    if text.contains("#EXT-X-STREAM-INF") {
        let variant = master_variant(&text).context("no AAC variant in the HLS playlist")?;
        text = get(variant)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
    }
    let segments = segment_urls(&text);
    if segments.is_empty() {
        bail!("empty HLS playlist");
    }
    let tmp = dest.with_extension("part");
    let mut out = tokio::io::BufWriter::new(tokio::fs::File::create(&tmp).await?);
    let mut demux = Demux::default();
    let mut audio = Vec::new();
    let mut written = 0;
    let mut bodies = futures_util::stream::iter(segments)
        .map(|url| {
            let get = &get;
            async move {
                // A fresh playlist's segments answer 403 for the first few
                // seconds (measured 2026-10-13: 403 at once, 200 after 5 s).
                let mut wait = Duration::from_millis(500);
                loop {
                    let response = get(url.clone()).send().await?;
                    if response.status() == reqwest::StatusCode::FORBIDDEN && wait <= MAX_WAIT {
                        tokio::time::sleep(wait).await;
                        wait *= 2;
                        continue;
                    }
                    return response.error_for_status()?.bytes().await;
                }
            }
        })
        .buffered(PARALLEL);
    use tokio::io::AsyncWriteExt;
    while let Some(body) = bodies.try_next().await.context("HLS segment")? {
        audio.clear();
        demux.feed(&body, &mut audio);
        out.write_all(&audio).await?;
        written += audio.len() as u64;
    }
    out.flush().await?;
    drop(out);
    if written == 0 {
        bail!("no AAC audio in the HLS segments");
    }
    tokio::fs::rename(&tmp, dest).await?;
    Ok(written)
}

fn segment_urls(playlist: &str) -> Vec<String> {
    playlist
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

/// The first variant whose codecs include AAC-LC (`mp4a.40.2`).
fn master_variant(master: &str) -> Option<String> {
    let mut lines = master.lines();
    while let Some(line) = lines.next() {
        if line.starts_with("#EXT-X-STREAM-INF") && line.contains("mp4a.40.2") {
            return lines.next().map(|l| l.trim().to_owned());
        }
    }
    None
}

/// Just enough MPEG-TS to pull out an ADTS AAC elementary stream.
#[derive(Default)]
struct Demux {
    pmt_pid: Option<u16>,
    audio_pid: Option<u16>,
}

const PACKET: usize = 188;
/// PMT stream type of ADTS AAC.
const STREAM_AAC: u8 = 0x0F;

impl Demux {
    /// Appends the audio of `ts` (whole packets) to `out`.
    fn feed(&mut self, ts: &[u8], out: &mut Vec<u8>) {
        for packet in ts.chunks_exact(PACKET) {
            if packet[0] != 0x47 {
                continue;
            }
            let start = packet[1] & 0x40 != 0;
            let pid = (u16::from(packet[1] & 0x1F) << 8) | u16::from(packet[2]);
            let adaptation = (packet[3] >> 4) & 0x3;
            let mut at = 4;
            if adaptation & 0x2 != 0 {
                at += 1 + usize::from(packet[4]);
            }
            if adaptation & 0x1 == 0 || at >= PACKET {
                continue;
            }
            let payload = &packet[at..];
            if pid == 0 && start {
                self.pmt_pid = parse_pat(payload);
            } else if Some(pid) == self.pmt_pid && start {
                self.audio_pid = parse_pmt(payload).or(self.audio_pid);
            } else if Some(pid) == self.audio_pid {
                if start {
                    out.extend_from_slice(pes_payload(payload).unwrap_or_default());
                } else {
                    out.extend_from_slice(payload);
                }
            }
        }
    }
}

/// PAT: the first program's PMT PID.
fn parse_pat(payload: &[u8]) -> Option<u16> {
    let section = payload.get(1 + usize::from(*payload.first()?)..)?;
    // table_id, section length (2), ts id (2), version, section/last (2), then programs.
    let program = section.get(8..12)?;
    let number = u16::from_be_bytes([program[0], program[1]]);
    (number != 0).then(|| (u16::from(program[2] & 0x1F) << 8) | u16::from(program[3]))
}

/// PMT: the PID of the ADTS AAC stream.
fn parse_pmt(payload: &[u8]) -> Option<u16> {
    let section = payload.get(1 + usize::from(*payload.first()?)..)?;
    let length = usize::from(u16::from_be_bytes([section[1], section[2]]) & 0x0FFF);
    let end = (3 + length).saturating_sub(4).min(section.len()); // minus CRC
    let info_len = usize::from(u16::from_be_bytes([*section.get(10)?, *section.get(11)?]) & 0x0FFF);
    let mut at = 12 + info_len;
    while at + 5 <= end {
        let kind = section[at];
        let pid = (u16::from(section[at + 1] & 0x1F) << 8) | u16::from(section[at + 2]);
        let es_info = usize::from(u16::from_be_bytes([section[at + 3], section[at + 4]]) & 0x0FFF);
        if kind == STREAM_AAC {
            return Some(pid);
        }
        at += 5 + es_info;
    }
    None
}

/// A PES packet's data after its header.
fn pes_payload(pes: &[u8]) -> Option<&[u8]> {
    if pes.get(..3)? != [0, 0, 1] {
        return None;
    }
    let header = usize::from(*pes.get(8)?);
    pes.get(9 + header..)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(pid: u16, start: bool, payload: &[u8]) -> Vec<u8> {
        let mut p = vec![
            0x47,
            (if start { 0x40 } else { 0 }) | (pid >> 8) as u8,
            pid as u8,
            0x10,
        ];
        p.extend_from_slice(payload);
        p.resize(PACKET, 0xFF);
        p
    }

    #[test]
    fn pulls_the_aac_stream_out_of_mpeg_ts() {
        // PAT → program 1 at PID 0x1000; PMT → video 0x100 (H.264), audio 0x101 (AAC).
        let pat = [
            0, 0x00, 0xB0, 0x0D, 0, 1, 0xC1, 0, 0, 0, 1, 0xF0, 0x00, 0, 0, 0, 0,
        ];
        let pmt = [
            0, 0x02, 0xB0, 0x17, 0, 1, 0xC1, 0, 0, 0xE1, 0x00, 0xF0, 0x00, 0x1B, 0xE1, 0x00, 0xF0,
            0x00, 0x0F, 0xE1, 0x01, 0xF0, 0x00, 0, 0, 0, 0,
        ];
        let adts = [0xFF, 0xF1, 0x50, 0x80, 0x01, 0x7F, 0xFC, 1, 2, 3];
        let mut pes = vec![0, 0, 1, 0xC0, 0, 0, 0x80, 0x80, 5, 0x21, 0, 1, 0, 1];
        pes.extend_from_slice(&adts[..5]);
        let mut ts = packet(0, true, &pat);
        ts.extend(packet(0x1000, true, &pmt));
        ts.extend(packet(0x100, true, &[0, 0, 1, 0xE0, 9, 9, 9]));
        ts.extend(packet(0x101, true, &pes));
        // The rest of the frame comes in a continuation packet with an
        // adaptation field filling the gap (no stuffing in the payload).
        let rest = &adts[5..];
        let mut cont = vec![0x47, 0x01, 0x01, 0x30, (PACKET - 5 - rest.len()) as u8];
        cont.resize(PACKET - rest.len(), 0xFF);
        cont[5] = 0;
        cont.extend_from_slice(rest);
        ts.extend(cont);

        let mut demux = Demux::default();
        let mut out = Vec::new();
        demux.feed(&ts, &mut out);
        assert_eq!(demux.audio_pid, Some(0x101));
        // The first packet's payload is padded with 0xFF by `packet`: what
        // matters is that the frame starts the stream and video is skipped.
        assert_eq!(&out[..5], &adts[..5]);
        assert!(out.ends_with(&adts[5..]));
        assert!(!out.windows(3).any(|w| w == [9, 9, 9]), "video skipped");
    }

    #[test]
    fn picks_an_aac_variant_and_lists_segments() {
        let master = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=75350,CODECS=\"mp4a.40.5,avc1.4D400B\"\nhttps://a/91\n\
            #EXT-X-STREAM-INF:BANDWIDTH=237549,CODECS=\"mp4a.40.2,avc1.4D4015\"\nhttps://a/93\n";
        assert_eq!(master_variant(master).as_deref(), Some("https://a/93"));
        let media =
            "#EXTM3U\n#EXTINF:5.12,\nhttps://s/1\n#EXTINF:5.12,\nhttps://s/2\n#EXT-X-ENDLIST\n";
        assert_eq!(segment_urls(media), ["https://s/1", "https://s/2"]);
    }

    /// Network: a real HLS variant, demuxed and decoded.
    #[tokio::test]
    #[ignore]
    async fn live_hls_audio() {
        let Ok(url) = std::env::var("YTM_HLS_URL") else {
            return;
        };
        let dest = std::env::temp_dir().join("ytm-hls-test.aac");
        let bytes = download_audio(&reqwest::Client::new(), &url, &[], &dest)
            .await
            .unwrap();
        println!("{bytes} bytes of ADTS in {}", dest.display());
    }
}
