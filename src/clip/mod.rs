//! The song's clip, played by us instead of YouTube's page: the window
//! shows the picture over the cover with its own controls, and the sound
//! stays the studio song's (the clip runs at song time + its intro offset).
//!
//! Pipeline: InnerTube's HLS master playlist for the clip (VISIONOS, H.264 +
//! AAC up to 1080p) → a variant fitting the window → its MPEG-TS segments,
//! fetched one ahead → our TS demuxer keeps the H.264 access units with
//! their PTS → the platform's H.264 decoder ([`VideoDecoder`]) → RGB
//! pictures scaled to the window, a few queued → the window's timer shows
//! the one due at the song's position.
//!
//! No pure-Rust decoder handles YouTube's Main / High profiles (openh264
//! decoded 9 of 158 frames of a 720p segment, 2026-11-27), so decoding is
//! the system's. Linux: FFmpeg's libavcodec + libswscale, loaded at run
//! time (no link-time dependency; without them the clip opens in the
//! browser as before). macOS / Windows: VideoToolbox / Media Foundation
//! backends are still to come; until then those keep the web view.

mod hls;
mod player;
mod ts;

#[cfg(target_os = "linux")]
mod ffmpeg;

pub use player::{ClipPlayer, Frame};

use slint::{Rgb8Pixel, SharedPixelBuffer};

/// One decoded picture: its presentation time (90 kHz, as in the stream)
/// and its pixels, already scaled to the size asked for.
pub struct Picture {
    pub pts: i64,
    pub pixels: SharedPixelBuffer<Rgb8Pixel>,
}

/// An H.264 decoder: Annex B access units in decode order, pictures out in
/// display order.
pub trait VideoDecoder: Send {
    /// Feeds one access unit with its PTS; appends the pictures that came
    /// out to `out`. A broken unit is dropped (the decoder recovers at the
    /// next key frame).
    fn decode(&mut self, unit: &[u8], pts: i64, out: &mut Vec<Picture>);

    /// The largest picture to hand out (physical pixels); the aspect ratio
    /// is kept and pictures are never scaled up.
    fn set_size(&mut self, width: u32, height: u32);

    /// Forgets the pictures in flight (a seek).
    fn reset(&mut self);
}

/// This system's decoder, when it has one.
pub fn decoder() -> Option<Box<dyn VideoDecoder>> {
    #[cfg(target_os = "linux")]
    {
        ffmpeg::Ffmpeg::new().map(|d| Box::new(d) as Box<dyn VideoDecoder>)
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// Whether clips can play in the window here.
pub fn available() -> bool {
    #[cfg(target_os = "linux")]
    {
        ffmpeg::loaded()
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}
