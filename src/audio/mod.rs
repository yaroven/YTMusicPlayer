//! Audio pipeline: the Track source (media URLs from InnerTube, yt-dlp as
//! the fallback, + bounded HTTP streaming), rodio playback and the play queue.

pub mod extractor;
pub mod hls;
pub mod innertube;
pub mod install;
pub mod js_runtime;
pub mod media_url;
pub mod player;
pub mod queue;
mod resolver;
pub mod source;
pub mod stream;

pub use resolver::JsPolicy;
pub use source::{Extractor, Opened, SourceOptions, TrackSource};
