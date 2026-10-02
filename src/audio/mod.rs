//! Audio pipeline: the Track source (yt-dlp URL resolution + bounded HTTP
//! streaming), rodio playback and the play queue.

pub mod extractor;
pub mod install;
pub mod js_runtime;
pub mod player;
pub mod queue;
mod resolver;
pub mod source;
pub mod stream;

pub use resolver::JsPolicy;
pub use source::{Extractor, Opened, SourceOptions, TrackSource};
