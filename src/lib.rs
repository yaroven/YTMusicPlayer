//! ytm-player core. `main.rs` is a thin binary over this library so every
//! module is unit-testable and usable from integration tests.

pub mod api;
pub mod app;
pub mod audio;
pub mod config;
pub mod storage;
pub mod sync;
pub mod ui;
