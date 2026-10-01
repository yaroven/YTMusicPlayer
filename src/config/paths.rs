//! Per-OS app directories.
//!
//! | OS      | data_dir                                              |
//! |---------|-------------------------------------------------------|
//! | macOS   | `~/Library/Application Support/dev.ytm-player.ytm-player` |
//! | Linux   | `$XDG_DATA_HOME/ytm-player` (`~/.local/share/ytm-player`) |
//! | Windows | `%APPDATA%\ytm-player\ytm-player\data`                |

use std::path::{Path, PathBuf};

use directories::ProjectDirs;

#[derive(Debug, Clone)]
pub struct AppPaths {
    dirs: ProjectDirs,
}

impl AppPaths {
    /// `None` only when no home directory can be determined.
    pub fn new() -> Option<Self> {
        ProjectDirs::from("dev", "ytm-player", "ytm-player").map(|dirs| Self { dirs })
    }

    pub fn config_file(&self) -> PathBuf {
        self.dirs.config_dir().join("config.toml")
    }

    pub fn data_dir(&self) -> &Path {
        self.dirs.data_dir()
    }

    pub fn database(&self) -> PathBuf {
        self.data_dir().join("library.sqlite3")
    }

    /// App-managed yt-dlp lives here.
    pub fn bin_dir(&self) -> PathBuf {
        self.data_dir().join("bin")
    }

    pub fn log_dir(&self) -> PathBuf {
        self.dirs.cache_dir().join("logs")
    }
}
