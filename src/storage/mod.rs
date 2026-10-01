//! Local SQLite cache (rusqlite, bundled) for playlists, tracks, ETags and
//! quota usage. Accessed through `spawn_blocking` from async code.

pub mod db;
pub mod migrations;
pub mod repo;
