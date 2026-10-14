//! Album art from YouTube thumbnails, cropped to a square like YT Music.
//!
//! Thumbnails are 16:9 frames (`mqdefault`, 320x180) or 4:3 with letterbox
//! bars (`sddefault` 640x480, `hqdefault` 480x360); the square in the middle
//! of the 16:9 picture is the album cover for music tracks. Pixels are
//! scaled down to what the UI shows, so memory stays at a few KB per row.

use anyhow::{Context, Result, bail};
use image::{RgbaImage, imageops::FilterType};
use slint::{Image, Rgb8Pixel, Rgba8Pixel, SharedPixelBuffer};

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

/// Decoded pixels. Only art with see-through parts (round artist pictures,
/// the large cover's rounded corners) needs the alpha byte; without it a
/// cached image is a quarter smaller.
pub enum Pixels {
    Rgb(SharedPixelBuffer<Rgb8Pixel>),
    Rgba(SharedPixelBuffer<Rgba8Pixel>),
}

impl Pixels {
    pub fn image(self) -> Image {
        match self {
            Self::Rgb(buffer) => Image::from_rgb8(buffer),
            Self::Rgba(buffer) => Image::from_rgba8(buffer),
        }
    }

    #[cfg(test)]
    pub fn width(&self) -> u32 {
        match self {
            Self::Rgb(buffer) => buffer.width(),
            Self::Rgba(buffer) => buffer.width(),
        }
    }

    #[cfg(test)]
    pub fn height(&self) -> u32 {
        match self {
            Self::Rgb(buffer) => buffer.height(),
            Self::Rgba(buffer) => buffer.height(),
        }
    }

    /// Bytes a decoded image takes (3 or 4 per pixel).
    #[cfg(test)]
    pub fn bytes(&self) -> usize {
        let pixels = (self.width() * self.height()) as usize;
        match self {
            Self::Rgb(_) => pixels * 3,
            Self::Rgba(_) => pixels * 4,
        }
    }

    /// The alpha of the top-left pixel (tests: corners and circles).
    #[cfg(test)]
    fn corner_alpha(&self) -> u8 {
        match self {
            Self::Rgb(_) => 255,
            Self::Rgba(buffer) => buffer.as_slice()[0].a,
        }
    }

    fn rgba_mut(&mut self) -> Option<&mut SharedPixelBuffer<Rgba8Pixel>> {
        match self {
            Self::Rgba(buffer) => Some(buffer),
            Self::Rgb(_) => None,
        }
    }
}

pub struct Art {
    pub pixels: Pixels,
    /// Average colour, for the now-playing background.
    pub tint: (u8, u8, u8),
    /// Large covers: a 12 px copy that, stretched over the now-playing
    /// view, looks like a blurred cover (no blur filter in the renderer).
    pub backdrop: Option<SharedPixelBuffer<Rgb8Pixel>>,
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
                    // Only round artist pictures and the large cover need
                    // the alpha byte (circle / rounded corners).
                    let alpha = matches!(size, ArtSize::Round | ArtSize::Large);
                    let mut art = decode_square(&bytes, size.pixels(), alpha)?;
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

const BACKDROP: u32 = 12;

/// Decodes a JPEG (or PNG), crops the centre square of its 16:9 picture area
/// and scales it to `edge` px. With `alpha` the pixels keep their alpha byte
/// (round pictures, the large cover's rounded corners); without it they are
/// a quarter smaller in the cache.
pub fn decode_square(jpeg: &[u8], edge: u32, alpha: bool) -> Result<Art> {
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
    let backdrop = (edge >= 256).then(|| {
        let small = image::imageops::resize(&out, BACKDROP, BACKDROP, FilterType::Triangle);
        let rgb: Vec<u8> = small.pixels().flat_map(|p| [p[0], p[1], p[2]]).collect();
        SharedPixelBuffer::clone_from_slice(&rgb, BACKDROP, BACKDROP)
    });
    if alpha && edge >= 256 {
        round_corners(&mut out, edge / 40);
    }
    let pixels = if alpha {
        Pixels::Rgba(SharedPixelBuffer::clone_from_slice(
            out.as_raw(),
            edge,
            edge,
        ))
    } else {
        let rgb: Vec<u8> = out.pixels().flat_map(|p| [p[0], p[1], p[2]]).collect();
        Pixels::Rgb(SharedPixelBuffer::clone_from_slice(&rgb, edge, edge))
    };
    Ok(Art {
        pixels,
        tint,
        backdrop,
    })
}

/// Makes everything outside the inscribed circle transparent
/// (anti-aliased over one pixel).
fn cut_circle(art: &mut Art) {
    let Some(pixels) = art.pixels.rgba_mut() else {
        return;
    };
    let (w, h) = (pixels.width(), pixels.height());
    let r = w.min(h) as f32 / 2.0;
    let stride = w as usize;
    for (i, p) in pixels.make_mut_slice().iter_mut().enumerate() {
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
        let art = decode_square(&jpeg(480, 360), 96, false).unwrap();
        assert_eq!((art.pixels.width(), art.pixels.height()), (96, 96));
        // No black bars made it into the crop.
        assert!(art.tint.0 > 230, "tint {:?}", art.tint);
    }

    #[test]
    fn wide_thumbnail_is_square_and_rounded_when_large() {
        let art = decode_square(&jpeg(320, 180), 480, true).unwrap();
        assert_eq!(art.pixels.width(), 480);
        assert!(
            art.pixels.corner_alpha() < 128,
            "corner should be transparent"
        );
        assert_eq!(art.pixels.bytes(), 480 * 480 * 4, "large covers keep alpha");

        // Row and card art has no see-through parts: a quarter smaller.
        let thumb = decode_square(&jpeg(640, 360), 96, false).unwrap();
        assert_eq!((thumb.pixels.width(), thumb.pixels.height()), (96, 96));
        assert_eq!(thumb.pixels.bytes(), 96 * 96 * 3);
    }
}
