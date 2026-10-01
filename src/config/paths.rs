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

    /// Single-instance socket (see `instance`): the per-user runtime dir on
    /// Linux, else the temp dir (per-user on macOS). Unix socket paths must
    /// stay under ~100 bytes, so not the (long) data dir.
    pub fn instance_socket(&self) -> PathBuf {
        let dir = self
            .dirs
            .runtime_dir()
            .map(Path::to_path_buf)
            .unwrap_or_else(std::env::temp_dir);
        #[cfg(unix)]
        let name = format!("ytm-player-{}.sock", unsafe { libc::getuid() });
        #[cfg(not(unix))]
        let name = "ytm-player.sock".to_owned();
        dir.join(name)
    }

    pub fn log_dir(&self) -> PathBuf {
        self.dirs.cache_dir().join("logs")
    }
}
