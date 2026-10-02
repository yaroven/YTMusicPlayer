//! ytm-player core. `main.rs` is a thin binary over this library so every
//! module is unit-testable and usable from integration tests.

pub mod account;
pub mod api;
pub mod app;
pub mod audio;
pub mod bootstrap;
pub mod config;
pub mod fmt;
#[cfg(feature = "gui")]
pub mod gui;
pub mod instance;
pub mod library_view;
pub mod media;
pub mod session;
pub mod storage;
pub mod sync;
pub mod sysmem;
pub mod ui;
