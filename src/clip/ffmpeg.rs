//! H.264 with the system's FFmpeg (libavcodec, libavutil, libswscale),
//! loaded with `dlopen` on first use: the binary doesn't link them, and
//! without them clips open in the browser as before.
//!
//! Only the leading fields of `AVPacket` (pts, data, size) and `AVFrame`
//! (data, linesize, width, height, format) are touched; they have kept
//! their places from FFmpeg 4 to 8 (libavcodec 58–62; verified on 7.1),
//! other majors aren't loaded, and a picture of an implausible size is
//! dropped rather than read. Pictures come out in display order without their PTS
//! being read back (`AVFrame.pts` moved between versions): each unit's
//! PTS goes into a min-heap and every picture takes the smallest.

use std::{
    cmp::Reverse,
    collections::BinaryHeap,
    ffi::{CString, c_char, c_int, c_void},
    ptr,
    sync::OnceLock,
};

use slint::{Rgb8Pixel, SharedPixelBuffer};

use super::{Picture, VideoDecoder};

const AV_CODEC_ID_H264: c_int = 27;
const AV_PIX_FMT_RGB24: c_int = 2;
const SWS_BILINEAR: c_int = 2;
/// `AVERROR(EAGAIN)` on Linux.
const EAGAIN: c_int = -11;
/// One decoding thread: 720p takes ~13 % of a core with it, and every
/// frame thread keeps a set of pictures of its own (2026-11-27: two
/// threads cost ~20 MB more at 720p).
const THREADS: i64 = 1;

// Field offsets (x86_64 / aarch64, FFmpeg 4–7).
const PACKET_PTS: usize = 8;
const PACKET_DATA: usize = 24;
const PACKET_SIZE: usize = 32;
const FRAME_LINESIZE: usize = 64;
const FRAME_WIDTH: usize = 104;
const FRAME_HEIGHT: usize = 108;
const FRAME_FORMAT: usize = 116;

type Ptr = *mut c_void;

struct Api {
    find_decoder: unsafe extern "C" fn(c_int) -> *const c_void,
    alloc_context: unsafe extern "C" fn(*const c_void) -> Ptr,
    open: unsafe extern "C" fn(Ptr, *const c_void, *mut Ptr) -> c_int,
    free_context: unsafe extern "C" fn(*mut Ptr),
    flush: unsafe extern "C" fn(Ptr),
    send_packet: unsafe extern "C" fn(Ptr, *const c_void) -> c_int,
    receive_frame: unsafe extern "C" fn(Ptr, Ptr) -> c_int,
    packet_alloc: unsafe extern "C" fn() -> Ptr,
    packet_free: unsafe extern "C" fn(*mut Ptr),
    frame_alloc: unsafe extern "C" fn() -> Ptr,
    frame_free: unsafe extern "C" fn(*mut Ptr),
    frame_unref: unsafe extern "C" fn(Ptr),
    opt_set_int: unsafe extern "C" fn(Ptr, *const c_char, i64, c_int) -> c_int,
    sws_get_context: unsafe extern "C" fn(
        c_int,
        c_int,
        c_int,
        c_int,
        c_int,
        c_int,
        c_int,
        Ptr,
        Ptr,
        *const f64,
    ) -> Ptr,
    sws_scale: unsafe extern "C" fn(
        Ptr,
        *const *const u8,
        *const c_int,
        c_int,
        c_int,
        *const *mut u8,
        *const c_int,
    ) -> c_int,
    sws_free: unsafe extern "C" fn(Ptr),
}

// Function pointers into libraries that stay loaded for the whole run.
unsafe impl Send for Api {}
unsafe impl Sync for Api {}

/// The first of `names` that loads.
fn open_library(names: &[&str]) -> Option<Ptr> {
    names.iter().find_map(|name| {
        let name = CString::new(*name).ok()?;
        // SAFETY: a plain dlopen; the handle is never closed.
        let handle = unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        (!handle.is_null()).then_some(handle)
    })
}

fn symbol(handle: Ptr, name: &str) -> Option<*mut c_void> {
    let name = CString::new(name).ok()?;
    // SAFETY: looks a symbol up in a loaded library.
    let found = unsafe { libc::dlsym(handle, name.as_ptr()) };
    (!found.is_null()).then_some(found)
}

// Each symbol's type is the `Api` field it is stored in.
#[allow(clippy::missing_transmute_annotations)]
fn load() -> Option<Api> {
    let codec = open_library(&[
        "libavcodec.so.62",
        "libavcodec.so.61",
        "libavcodec.so.60",
        "libavcodec.so.59",
        "libavcodec.so.58",
    ])?;
    let util = open_library(&[
        "libavutil.so.60",
        "libavutil.so.59",
        "libavutil.so.58",
        "libavutil.so.57",
        "libavutil.so.56",
    ])?;
    let scale = open_library(&[
        "libswscale.so.9",
        "libswscale.so.8",
        "libswscale.so.7",
        "libswscale.so.6",
        "libswscale.so.5",
    ])?;
    // SAFETY: each symbol is cast to its C signature (FFmpeg's public API).
    unsafe {
        let version: unsafe extern "C" fn() -> u32 =
            std::mem::transmute(symbol(codec, "avcodec_version")?);
        let major = version() >> 16;
        if !(58..=62).contains(&major) {
            tracing::info!(
                major,
                "libavcodec with an unknown layout; clips use the browser"
            );
            return None;
        }
        macro_rules! get {
            ($lib:expr, $name:literal) => {
                std::mem::transmute(symbol($lib, $name)?)
            };
        }
        Some(Api {
            find_decoder: get!(codec, "avcodec_find_decoder"),
            alloc_context: get!(codec, "avcodec_alloc_context3"),
            open: get!(codec, "avcodec_open2"),
            free_context: get!(codec, "avcodec_free_context"),
            flush: get!(codec, "avcodec_flush_buffers"),
            send_packet: get!(codec, "avcodec_send_packet"),
            receive_frame: get!(codec, "avcodec_receive_frame"),
            packet_alloc: get!(codec, "av_packet_alloc"),
            packet_free: get!(codec, "av_packet_free"),
            frame_alloc: get!(util, "av_frame_alloc"),
            frame_free: get!(util, "av_frame_free"),
            frame_unref: get!(util, "av_frame_unref"),
            opt_set_int: get!(util, "av_opt_set_int"),
            sws_get_context: get!(scale, "sws_getContext"),
            sws_scale: get!(scale, "sws_scale"),
            sws_free: get!(scale, "sws_freeContext"),
        })
    }
}

fn api() -> Option<&'static Api> {
    static API: OnceLock<Option<Api>> = OnceLock::new();
    API.get_or_init(|| {
        let api = load();
        if api.is_none() {
            tracing::info!("no usable FFmpeg libraries: clips open in the browser");
        }
        api
    })
    .as_ref()
}

pub fn loaded() -> bool {
    api().is_some()
}

pub struct Ffmpeg {
    api: &'static Api,
    context: Ptr,
    packet: Ptr,
    frame: Ptr,
    /// Scaler and what it was made for: source size and format, output size.
    scaler: Ptr,
    scaler_for: (c_int, c_int, c_int, u32, u32),
    /// PTS of the units in flight, smallest first.
    pending: BinaryHeap<Reverse<i64>>,
    size: (u32, u32),
}

// The FFmpeg objects belong to this decoder alone and move with it.
unsafe impl Send for Ffmpeg {}

impl Ffmpeg {
    pub fn new() -> Option<Self> {
        let api = api()?;
        // SAFETY: FFmpeg's documented allocation sequence; every pointer is
        // checked before use and freed in `drop`.
        unsafe {
            let codec = (api.find_decoder)(AV_CODEC_ID_H264);
            if codec.is_null() {
                return None;
            }
            let mut context = (api.alloc_context)(codec);
            if context.is_null() {
                return None;
            }
            (api.opt_set_int)(context, c"threads".as_ptr(), THREADS, 0);
            if (api.open)(context, codec, ptr::null_mut()) < 0 {
                (api.free_context)(&mut context);
                return None;
            }
            Some(Self {
                api,
                context,
                packet: (api.packet_alloc)(),
                frame: (api.frame_alloc)(),
                scaler: ptr::null_mut(),
                scaler_for: (0, 0, 0, 0, 0),
                pending: BinaryHeap::new(),
                size: (1280, 720),
            })
        }
    }

    /// The decoded frame, scaled to fit `self.size`, as RGB.
    fn picture(&mut self) -> Option<SharedPixelBuffer<Rgb8Pixel>> {
        let api = self.api;
        // SAFETY: reads the stable leading fields of a frame the decoder
        // just filled; the scaler writes into a buffer of the size it was
        // made for, with a matching stride.
        unsafe {
            let frame = self.frame.cast::<u8>();
            let width = frame.add(FRAME_WIDTH).cast::<c_int>().read();
            let height = frame.add(FRAME_HEIGHT).cast::<c_int>().read();
            let format = frame.add(FRAME_FORMAT).cast::<c_int>().read();
            if !(1..=8192).contains(&width) || !(1..=8192).contains(&height) {
                return None;
            }
            let (out_w, out_h) = fit((width as u32, height as u32), self.size);
            let wanted = (width, height, format, out_w, out_h);
            if self.scaler.is_null() || self.scaler_for != wanted {
                if !self.scaler.is_null() {
                    (api.sws_free)(self.scaler);
                }
                self.scaler = (api.sws_get_context)(
                    width,
                    height,
                    format,
                    out_w as c_int,
                    out_h as c_int,
                    AV_PIX_FMT_RGB24,
                    SWS_BILINEAR,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null(),
                );
                self.scaler_for = wanted;
                if self.scaler.is_null() {
                    return None;
                }
            }
            let mut pixels = SharedPixelBuffer::<Rgb8Pixel>::new(out_w, out_h);
            let dst = [
                pixels.make_mut_bytes().as_mut_ptr(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            ];
            let dst_stride = [(out_w * 3) as c_int, 0, 0, 0];
            (api.sws_scale)(
                self.scaler,
                self.frame.cast::<*const u8>(),
                frame.add(FRAME_LINESIZE).cast::<c_int>(),
                0,
                height,
                dst.as_ptr(),
                dst_stride.as_ptr(),
            );
            Some(pixels)
        }
    }
}

/// `source` scaled down to fit `bounds` (aspect kept, even sizes, never up).
fn fit(source: (u32, u32), bounds: (u32, u32)) -> (u32, u32) {
    let (w, h) = source;
    let scale = (bounds.0 as f64 / w as f64)
        .min(bounds.1 as f64 / h as f64)
        .min(1.0);
    let even = |v: f64| ((v as u32) & !1).max(2);
    (even(w as f64 * scale), even(h as f64 * scale))
}

impl VideoDecoder for Ffmpeg {
    fn decode(&mut self, unit: &[u8], pts: i64, out: &mut Vec<Picture>) {
        let api = self.api;
        // SAFETY: the packet points at `unit` only for `send_packet`, which
        // copies data of a packet without a buffer; it's cleared after.
        let sent = unsafe {
            let packet = self.packet.cast::<u8>();
            packet.add(PACKET_PTS).cast::<i64>().write(pts);
            packet
                .add(PACKET_DATA)
                .cast::<*const u8>()
                .write(unit.as_ptr());
            packet
                .add(PACKET_SIZE)
                .cast::<c_int>()
                .write(unit.len() as c_int);
            let sent = (api.send_packet)(self.context, self.packet);
            packet
                .add(PACKET_DATA)
                .cast::<*const u8>()
                .write(ptr::null());
            packet.add(PACKET_SIZE).cast::<c_int>().write(0);
            sent
        };
        if sent < 0 && sent != EAGAIN {
            tracing::debug!(sent, "H.264 unit rejected");
            return;
        }
        self.pending.push(Reverse(pts));
        loop {
            // SAFETY: the decoder fills our frame or says why not.
            let got = unsafe { (api.receive_frame)(self.context, self.frame) };
            if got < 0 {
                break;
            }
            let pts = self.pending.pop().map_or(pts, |Reverse(p)| p);
            if let Some(pixels) = self.picture() {
                out.push(Picture { pts, pixels });
            }
            // SAFETY: releases the picture's buffers back to the decoder.
            unsafe { (api.frame_unref)(self.frame) };
        }
        // Units that never come out (broken ones) mustn't pile up.
        while self.pending.len() > 32 {
            self.pending.pop();
        }
    }

    fn set_size(&mut self, width: u32, height: u32) {
        self.size = (width.max(2), height.max(2));
    }

    fn reset(&mut self) {
        // SAFETY: drops the pictures in flight; the context stays usable.
        unsafe { (self.api.flush)(self.context) };
        self.pending.clear();
    }
}

impl Drop for Ffmpeg {
    fn drop(&mut self) {
        let api = self.api;
        // SAFETY: frees what `new` and `picture` allocated, once.
        unsafe {
            if !self.scaler.is_null() {
                (api.sws_free)(self.scaler);
            }
            (api.frame_free)(&mut self.frame);
            (api.packet_free)(&mut self.packet);
            (api.free_context)(&mut self.context);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_fit_without_growing() {
        assert_eq!(fit((1280, 720), (640, 640)), (640, 360));
        assert_eq!(fit((1280, 720), (4000, 4000)), (1280, 720));
        assert_eq!(fit((1920, 1080), (1001, 563)), (1000, 562));
    }
}
