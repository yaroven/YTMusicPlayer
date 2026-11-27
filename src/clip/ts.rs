//! The H.264 stream of an HLS segment (MPEG-TS): PAT → PMT → the video
//! PID's PES packets, each one access unit with its PTS. The PAT / PMT
//! parsing is the audio demuxer's (`audio::hls`).

use crate::audio::hls::{PACKET, parse_pat, parse_pmt};

/// PMT stream type of H.264.
const STREAM_H264: u8 = 0x1B;

/// One access unit (Annex B) and its presentation time (90 kHz).
pub struct Unit {
    pub pts: i64,
    pub data: Vec<u8>,
}

/// Splits whole MPEG-TS segments into the H.264 access units inside.
#[derive(Default)]
pub struct VideoDemux {
    pmt_pid: Option<u16>,
    video_pid: Option<u16>,
    /// The PES packet being gathered: its PTS and data so far.
    pes: Option<(Option<i64>, Vec<u8>)>,
}

impl VideoDemux {
    /// The units of `segment`, in decode order; the last one is emitted
    /// when the segment ends (HLS video PES packets have no length).
    pub fn units(&mut self, segment: &[u8]) -> Vec<Unit> {
        let mut out = Vec::new();
        for packet in segment.chunks_exact(PACKET) {
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
                self.video_pid = parse_pmt(payload, STREAM_H264).or(self.video_pid);
            } else if Some(pid) == self.video_pid {
                if start {
                    self.flush(&mut out);
                    self.pes = pes_start(payload);
                } else if let Some((_, data)) = &mut self.pes {
                    data.extend_from_slice(payload);
                }
            }
        }
        self.flush(&mut out);
        out
    }

    fn flush(&mut self, out: &mut Vec<Unit>) {
        if let Some((Some(pts), data)) = self.pes.take()
            && !data.is_empty()
        {
            out.push(Unit { pts, data });
        }
    }
}

/// A PES packet's PTS and the data after its header.
fn pes_start(pes: &[u8]) -> Option<(Option<i64>, Vec<u8>)> {
    if pes.get(..3)? != [0, 0, 1] {
        return None;
    }
    let flags = *pes.get(7)?;
    let header = usize::from(*pes.get(8)?);
    let pts = (flags & 0x80 != 0)
        .then(|| pes.get(9..14).map(pts_of))
        .flatten();
    Some((pts, pes.get(9 + header..)?.to_vec()))
}

/// The 33-bit timestamp in a PES header's 5 bytes.
fn pts_of(b: &[u8]) -> i64 {
    (i64::from(b[0] >> 1 & 0x07) << 30)
        | (i64::from(b[1]) << 22)
        | (i64::from(b[2] >> 1) << 15)
        | (i64::from(b[3]) << 7)
        | i64::from(b[4] >> 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A TS packet carrying `payload` on `pid` (stuffed with an adaptation
    /// field to 188 bytes).
    fn packet(pid: u16, start: bool, payload: &[u8]) -> Vec<u8> {
        let mut p = vec![
            0x47,
            (u8::from(start) << 6) | (pid >> 8) as u8,
            pid as u8,
            0x30,
        ];
        let stuffing = PACKET - 4 - 1 - payload.len();
        p.push(stuffing as u8);
        if stuffing > 0 {
            p.push(0);
            p.extend(std::iter::repeat_n(0xFF, stuffing - 1));
        }
        p.extend_from_slice(payload);
        p
    }

    fn pat() -> Vec<u8> {
        packet(
            0,
            true,
            &[
                0, 0x00, 0xB0, 0x0D, 0, 1, 0xC1, 0, 0, 0, 1, 0xF0, 0x00, 0, 0, 0, 0,
            ],
        )
    }

    fn pmt() -> Vec<u8> {
        // Program 1: AAC on 0x101, H.264 on 0x100.
        packet(
            0x1000,
            true,
            &[
                0, 0x02, 0xB0, 0x17, 0, 1, 0xC1, 0, 0, 0xE1, 0x00, 0xF0, 0x00, 0x0F, 0xE1, 0x01,
                0xF0, 0x00, 0x1B, 0xE1, 0x00, 0xF0, 0x00, 0, 0, 0, 0,
            ],
        )
    }

    /// PES header with a PTS of `pts`.
    fn pes(pts: i64, data: &[u8]) -> Vec<u8> {
        let mut p = vec![0, 0, 1, 0xE0, 0, 0, 0x80, 0x80, 5];
        p.extend([
            0x21 | ((pts >> 29) as u8 & 0x0E),
            (pts >> 22) as u8,
            ((pts >> 14) as u8 & 0xFE) | 1,
            (pts >> 7) as u8,
            ((pts << 1) as u8 & 0xFE) | 1,
        ]);
        p.extend_from_slice(data);
        p
    }

    #[test]
    fn pulls_h264_units_with_their_pts() {
        let mut ts = pat();
        ts.extend(pmt());
        ts.extend(packet(0x100, true, &pes(900_000, &[0, 0, 0, 1, 0x65, 1])));
        ts.extend(packet(0x100, false, &[2, 3]));
        ts.extend(packet(0x101, true, &pes(1, &[0xFF, 0xF1])));
        ts.extend(packet(
            0x100,
            true,
            &pes(8_589_934_000, &[0, 0, 0, 1, 0x41]),
        ));
        let units = VideoDemux::default().units(&ts);
        assert_eq!(units.len(), 2);
        assert_eq!(units[0].pts, 900_000);
        assert_eq!(units[0].data, [0, 0, 0, 1, 0x65, 1, 2, 3]);
        assert_eq!(units[1].pts, 8_589_934_000, "33-bit PTS");
        assert_eq!(units[1].data, [0, 0, 0, 1, 0x41]);
    }
}
