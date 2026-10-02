//! Display formatting shared by both frontends.

use std::time::Duration;

/// `m:ss` or `h:mm:ss`.
pub fn fmt_time(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_format() {
        assert_eq!(fmt_time(Duration::from_secs(5)), "0:05");
        assert_eq!(fmt_time(Duration::from_secs(213)), "3:33");
        assert_eq!(fmt_time(Duration::from_secs(3723)), "1:02:03");
    }
}
