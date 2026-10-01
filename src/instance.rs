//! Single instance: the first player (TUI or GUI) listens on a Unix socket;
//! a second launch connects, asks it to show its window, and exits.
//!
//! Protocol: client writes `show\n`, server replies with its kind (`gui` or
//! `tui`) and, for the GUI, raises its window. CLI subcommands (`sync`,
//! `status`, …) don't take part and may run alongside.

use std::path::{Path, PathBuf};

pub enum Acquired {
    /// We are the only player; keep this alive until exit.
    Primary(Instance),
    /// Another player is running; its kind (`gui` / `tui`).
    Running(String),
}

pub struct Instance {
    #[cfg_attr(not(unix), allow(dead_code))]
    path: PathBuf,
}

impl Instance {
    /// No socket (single-instance check unavailable); does nothing on drop.
    pub fn none() -> Self {
        Self {
            path: PathBuf::new(),
        }
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        #[cfg(unix)]
        if !self.path.as_os_str().is_empty() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// `kind` is reported to later launches; `on_show` runs (on a listener
/// thread) when one of them asks to bring this player to the front.
#[cfg(unix)]
pub fn acquire(
    path: &Path,
    kind: &'static str,
    on_show: impl Fn() + Send + 'static,
) -> std::io::Result<Acquired> {
    use std::{
        io::{BufRead, BufReader, Write},
        os::unix::net::{UnixListener, UnixStream},
        time::Duration,
    };

    if let Ok(mut stream) = UnixStream::connect(path) {
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        stream.write_all(b"show\n")?;
        let mut reply = String::new();
        BufReader::new(stream).read_line(&mut reply)?;
        return Ok(Acquired::Running(reply.trim().to_owned()));
    }

    // Nobody answered: a leftover socket from a crash, or none at all.
    let _ = std::fs::remove_file(path);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let listener = UnixListener::bind(path)?;
    std::thread::Builder::new()
        .name("instance".into())
        .stack_size(64 * 1024)
        .spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let mut line = String::new();
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                if BufReader::new(&stream).read_line(&mut line).is_ok() && line.trim() == "show" {
                    on_show();
                }
                let _ = writeln!(stream, "{kind}");
            }
        })?;
    Ok(Acquired::Primary(Instance {
        path: path.to_owned(),
    }))
}

#[cfg(not(unix))]
pub fn acquire(
    path: &Path,
    _kind: &'static str,
    _on_show: impl Fn() + Send + 'static,
) -> std::io::Result<Acquired> {
    Ok(Acquired::Primary(Instance {
        path: path.to_owned(),
    }))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn second_launch_finds_the_first() {
        let path = std::env::temp_dir().join(format!("ytm-inst-{}.sock", std::process::id()));
        let shown = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = shown.clone();
        let first = acquire(&path, "gui", move || {
            flag.store(true, std::sync::atomic::Ordering::SeqCst)
        })
        .unwrap();
        assert!(matches!(first, Acquired::Primary(_)));
        match acquire(&path, "tui", || {}).unwrap() {
            Acquired::Running(kind) => assert_eq!(kind, "gui"),
            Acquired::Primary(_) => panic!("second launch became primary"),
        }
        assert!(shown.load(std::sync::atomic::Ordering::SeqCst));
        drop(first);
        assert!(!path.exists(), "socket removed on exit");
    }
}
