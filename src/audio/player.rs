//! Playback engine on a dedicated OS thread (rodio `OutputStream` is !Send).
//! Receives `PlayerCommand` (Play, Pause, Skip, Seek, Volume) over a channel
//! and publishes `PlayerEvent` (position, track ended, error) back to the app.
