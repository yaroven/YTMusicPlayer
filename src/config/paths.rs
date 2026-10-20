//! Per-OS app directories.
//!
//! | OS      | data_dir                                              |
//! |---------|-------------------------------------------------------|
//! | macOS   | `~/Library/Application Support/dev.yarek.yarek-music` |
//! | Linux   | `$XDG_DATA_HOME/yarek-music` (`~/.local/share/yarek-music`) |
//! | Windows | `%APPDATA%\yarek\yarek-music\data`                |

use std::path::{Path, PathBuf};

use directories::ProjectDirs;

#[derive(Debug, Clone)]
pub struct AppPaths {
    dirs: ProjectDirs,
}

impl AppPaths {
    /// `None` only when no home directory can be determined.
    pub fn new() -> Option<Self> {
        ProjectDirs::from("dev", "yarek", "yarek-music").map(|dirs| Self { dirs })
    }

    /// Up to 1.0.0 the app was "ytm-player": its folders (settings, library,
    /// downloads, yt-dlp) move to the new ones once, so nothing is lost. The
    /// library stores downloads by absolute path; those follow the move.
    pub fn migrate_old(&self) {
        let Some(old) = ProjectDirs::from("dev", "ytm-player", "ytm-player") else {
            return;
        };
        let old_data = old.data_dir().to_path_buf();
        let mut moved_data = false;
        for (from, to) in [
            (old.config_dir(), self.dirs.config_dir()),
            (old.data_dir(), self.dirs.data_dir()),
        ] {
            if from == to || !from.exists() || to.exists() {
                continue;
            }
            if let Some(parent) = to.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if std::fs::rename(from, to).is_ok() && to == self.dirs.data_dir() {
                moved_data = true;
            }
        }
        if moved_data && let Ok(conn) = rusqlite::Connection::open(self.database()) {
            let _ = conn.execute(
                "UPDATE downloads SET path = ?2 || substr(path, length(?1) + 1) WHERE path LIKE ?1 || '%'",
                (
                    old_data.to_string_lossy().as_ref(),
                    self.data_dir().to_string_lossy().as_ref(),
                ),
            );
        }
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
        let name = format!("yarek-music-{}.sock", unsafe { libc::getuid() });
        #[cfg(not(unix))]
        let name = "yarek-music.sock".to_owned();
        dir.join(name)
    }

    pub fn log_dir(&self) -> PathBuf {
        self.dirs.cache_dir().join("logs")
    }
}
