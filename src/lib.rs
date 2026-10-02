//! ytm-player core. `main.rs` is a thin binary over this library so every
//! module is unit-testable and usable from integration tests.

pub mod account;
pub mod api;
pub mod app;
pub mod audio;
pub mod bootstrap;
pub mod cast;
pub mod catalog;
pub mod config;
pub mod discord;
pub mod fmt;
#[cfg(feature = "gui")]
pub mod gui;
pub mod instance;
pub mod lastfm;
pub mod library_view;
pub mod lyrics;
pub mod media;
#[cfg(feature = "gui")]
pub mod notify;
pub mod session;
pub mod storage;
pub mod sync;
pub mod sysmem;
pub mod ui;
pub mod update;
