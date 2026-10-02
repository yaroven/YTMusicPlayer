//! Album art from YouTube thumbnails, cropped to a square like YT Music.
//!
//! Thumbnails are 16:9 frames (`mqdefault`, 320x180) or 4:3 with letterbox
//! bars (`sddefault` 640x480, `hqdefault` 480x360); the square in the middle
//! of the 16:9 picture is the album cover for music tracks. Pixels are
//! scaled down to what the UI shows, so memory stays at a few KB per row.

use anyhow::{Context, Result, bail};
use image::{RgbaImage, imageops::FilterType};
use slint::{Rgba8Pixel, SharedPixelBuffer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArtSize {
    /// List rows and the player bar (rendered at 40-48 px).
    Thumb,
    /// The full-screen "now playing" view.
    Large,
    /// Album / playlist cards and page headers, by URL.
    Card,
    /// Artist pictures: like `Card`, cut to a circle (the software
    /// renderer doesn't clip images to rounded shapes).
    Round,
}

impl ArtSize {
    /// Output edge in pixels (2x for HiDPI screens).
    fn pixels(self) -> u32 {
        match self {
            Self::Thumb => 96,
            Self::Large => 480,
            Self::Card | Self::Round => 160,
        }
    }

    fn sources(self) -> &'static [&'static str] {
        match self {
            Self::Thumb => &["mqdefault"],
            // sddefault is missing for some videos; hqdefault always exists.
            Self::Large => &["sddefault", "hqdefault"],
            Self::Card | Self::Round => &[],
        }
    }
}

pub struct Art {
    pub pixels: SharedPixelBuffer<Rgba8Pixel>,
    /// Average colour, for the now-playing background.
    pub tint: (u8, u8, u8),
}

/// Art for `key`: a video id, or for [`ArtSize::Card`] the image URL.
pub async fn fetch(http: &reqwest::Client, key: &str, size: ArtSize) -> Result<Art> {
    let urls: Vec<String> = match size {
        ArtSize::Card | ArtSize::Round => vec![card_url(key, size.pixels())],
        _ => size
            .sources()
            .iter()
            .map(|name| format!("https://i.ytimg.com/vi/{key}/{name}.jpg"))
            .collect(),
    };
    let mut last_err = None;
    for url in urls {
        match http
            .get(&url)
            .send()
            .await
            .and_then(|r| r.error_for_status())
        {
            Ok(response) => {
                let bytes = response.bytes().await?;
                return tokio::task::spawn_blocking(move || {
                    let mut art = decode_square(&bytes, size.pixels())?;
                    if size == ArtSize::Round {
                        cut_circle(&mut art);
                    }
                    Ok(art)
                })
                .await?;
            }
            Err(err) => last_err = Some(err),
        }
    }
    match last_err {
        Some(err) => Err(err).context("downloading thumbnail"),
        None => bail!("no thumbnail source"),
    }
}

/// Google image URLs take a size suffix (`=s576`, `=w226-h226-l90-rj`);
/// asking for the card's size as JPEG (`-rj`) keeps downloads small and
/// decodable (they'd be WebP or PNG otherwise).
fn card_url(url: &str, edge: u32) -> String {
    match url.rsplit_once('=') {
        Some((base, _)) if url.contains("googleusercontent.com/") => {
            format!("{base}=w{edge}-h{edge}-l90-rj")
        }
        _ => url.to_owned(),
    }
}

/// Decodes a JPEG (or PNG), crops the centre square of its 16:9 picture area and
/// scales it to `edge` px. Large covers get softly rounded corners.
pub fn decode_square(jpeg: &[u8], edge: u32) -> Result<Art> {
    let img = image::load_from_memory(jpeg)
        .context("decoding thumbnail")?
        .to_rgba8();
    let (w, h) = img.dimensions();
    // 4:3 thumbnails letterbox a 16:9 picture: skip the bars.
    let picture_h = if w * 3 == h * 4 { w * 9 / 16 } else { h };
    let side = picture_h.min(w);
    let x = (w - side) / 2;
    let y = (h - side) / 2;
    let square = image::imageops::crop_imm(&img, x, y, side, side).to_image();
    drop(img);
    let mut out = image::imageops::resize(&square, edge, edge, FilterType::Triangle);
    let tint = average(&out);
    if edge >= 256 {
        round_corners(&mut out, edge / 40);
    }
    Ok(Art {
        pixels: SharedPixelBuffer::clone_from_slice(out.as_raw(), edge, edge),
        tint,
    })
}

/// Makes everything outside the inscribed circle transparent
/// (anti-aliased over one pixel).
fn cut_circle(art: &mut Art) {
    let (w, h) = (art.pixels.width(), art.pixels.height());
    let r = w.min(h) as f32 / 2.0;
    let stride = w as usize;
    for (i, p) in art.pixels.make_mut_slice().iter_mut().enumerate() {
        let (x, y) = ((i % stride) as f32 + 0.5, (i / stride) as f32 + 0.5);
        let d = ((x - r).powi(2) + (y - r).powi(2)).sqrt();
        let coverage = (r - d + 0.5).clamp(0.0, 1.0);
        if coverage < 1.0 {
            p.a = (f32::from(p.a) * coverage) as u8;
        }
    }
}

fn average(img: &RgbaImage) -> (u8, u8, u8) {
    let (mut r, mut g, mut b) = (0u64, 0u64, 0u64);
    for p in img.pixels() {
        r += u64::from(p[0]);
        g += u64::from(p[1]);
        b += u64::from(p[2]);
    }
    let n = u64::from(img.width() * img.height()).max(1);
    ((r / n) as u8, (g / n) as u8, (b / n) as u8)
}

/// Makes corner pixels transparent (anti-aliased by coverage).
fn round_corners(img: &mut RgbaImage, radius: u32) {
    if radius == 0 {
        return;
    }
    let (w, h) = img.dimensions();
    let r = radius as f32;
    for cy in 0..radius {
        for cx in 0..radius {
            // Distance from the corner circle's centre to this pixel centre.
            let dx = r - (cx as f32 + 0.5);
            let dy = r - (cy as f32 + 0.5);
            let coverage = (r - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0.0, 1.0);
            if coverage >= 1.0 {
                continue;
            }
            for (x, y) in [
                (cx, cy),
                (w - 1 - cx, cy),
                (cx, h - 1 - cy),
                (w - 1 - cx, h - 1 - cy),
            ] {
                let p = img.get_pixel_mut(x, y);
                p[3] = (f32::from(p[3]) * coverage) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg(w: u32, h: u32) -> Vec<u8> {
        // Black bars top/bottom, white picture in the middle 16:9 area.
        let mut img = image::RgbImage::new(w, h);
        let picture_h = w * 9 / 16;
        let top = (h - picture_h) / 2;
        for y in top..top + picture_h {
            for x in 0..w {
                img.put_pixel(x, y, image::Rgb([255, 255, 255]));
            }
        }
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Jpeg).unwrap();
        out.into_inner()
    }

    #[test]
    fn google_image_urls_ask_for_jpeg_at_card_size() {
        assert_eq!(
            card_url("https://yt3.googleusercontent.com/abc=s576", 160),
            "https://yt3.googleusercontent.com/abc=w160-h160-l90-rj"
        );
        let ytimg = "https://i.ytimg.com/vi/x/hqdefault.jpg";
        assert_eq!(card_url(ytimg, 160), ytimg);
    }

    #[test]
    fn letterboxed_thumbnail_crops_to_picture() {
        let art = decode_square(&jpeg(480, 360), 96).unwrap();
        assert_eq!((art.pixels.width(), art.pixels.height()), (96, 96));
        // No black bars made it into the crop.
        assert!(art.tint.0 > 230, "tint {:?}", art.tint);
    }

    #[test]
    fn wide_thumbnail_is_square_and_rounded_when_large() {
        let art = decode_square(&jpeg(320, 180), 480).unwrap();
        assert_eq!(art.pixels.width(), 480);
        let corner = art.pixels.as_slice()[0];
        assert!(corner.a < 128, "corner should be transparent");
    }
}
