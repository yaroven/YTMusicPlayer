//! Memory of this process and its helper children (yt-dlp), for display.
//!
//! macOS: `phys_footprint` (what Activity Monitor shows as "Memory").
//! Linux: proportional set size (`Pss` from `smaps_rollup`).
//! Children are found one and two levels down (yt-dlp's PyInstaller build
//! runs Python in a grandchild process).

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    /// This process, bytes.
    pub own: u64,
    /// yt-dlp and other children (and their children), bytes.
    pub helpers: u64,
}

impl Usage {
    /// "RAM 15 MB" or "RAM 15 MB + yt-dlp 93 MB".
    pub fn label(&self) -> String {
        if self.own == 0 {
            return String::new();
        }
        let mb = |b: u64| (b + (1 << 19)) >> 20;
        if self.helpers > 0 {
            format!("RAM {} MB + yt-dlp {} MB", mb(self.own), mb(self.helpers))
        } else {
            format!("RAM {} MB", mb(self.own))
        }
    }
}

pub fn sample() -> Usage {
    let pid = std::process::id();
    let mut helpers = 0;
    for child in children(pid) {
        helpers += memory(child);
        helpers += children(child).into_iter().map(memory).sum::<u64>();
    }
    Usage {
        own: memory(pid),
        helpers,
    }
}

#[cfg(target_os = "macos")]
fn memory(pid: u32) -> u64 {
    let mut info: libc::rusage_info_v2 = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        libc::proc_pid_rusage(
            pid as libc::c_int,
            libc::RUSAGE_INFO_V2,
            (&mut info as *mut libc::rusage_info_v2).cast(),
        )
    } == 0;
    if ok { info.ri_phys_footprint } else { 0 }
}

#[cfg(target_os = "macos")]
fn children(pid: u32) -> Vec<u32> {
    let mut pids = [0 as libc::pid_t; 32];
    let bytes = unsafe {
        libc::proc_listchildpids(
            pid as libc::pid_t,
            pids.as_mut_ptr().cast(),
            std::mem::size_of_val(&pids) as libc::c_int,
        )
    };
    // Returns a count on recent macOS (bytes on some older releases).
    let n = (bytes.max(0) as usize).min(pids.len());
    pids[..n]
        .iter()
        .filter(|&&p| p > 0)
        .map(|&p| p as u32)
        .collect()
}

#[cfg(target_os = "linux")]
fn memory(pid: u32) -> u64 {
    std::fs::read_to_string(format!("/proc/{pid}/smaps_rollup"))
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Pss:"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok())
        })
        .map_or(0, |kb| kb * 1024)
}

#[cfg(target_os = "linux")]
fn children(pid: u32) -> Vec<u32> {
    std::fs::read_to_string(format!("/proc/{pid}/task/{pid}/children"))
        .map(|s| {
            s.split_whitespace()
                .filter_map(|p| p.parse().ok())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn memory(_pid: u32) -> u64 {
    0
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn children(_pid: u32) -> Vec<u32> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn measures_this_process() {
        let usage = sample();
        assert!(
            usage.own > 1 << 20,
            "own memory should be > 1 MB: {usage:?}"
        );
    }

    #[test]
    fn labels() {
        let mb = 1 << 20;
        assert_eq!(
            Usage {
                own: 15 * mb,
                helpers: 0
            }
            .label(),
            "RAM 15 MB"
        );
        assert_eq!(
            Usage {
                own: 15 * mb,
                helpers: 93 * mb
            }
            .label(),
            "RAM 15 MB + yt-dlp 93 MB"
        );
        assert_eq!(Usage::default().label(), "");
    }
}
