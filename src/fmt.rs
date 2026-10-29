//! Human-friendly formatting helpers shared by the TUI and the JSON report.

const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];

/// Formats a byte count with binary (1024) steps, e.g. `14.2 GB`.
pub fn bytes(n: u64) -> String {
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else if value < 10.0 {
        format!("{value:.2} {}", UNITS[unit])
    } else if value < 100.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
    }
}

/// Formats a byte count compactly for tight columns, e.g. `14.2G`.
pub fn bytes_short(n: u64) -> String {
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    let suffix = &UNITS[unit][..1];
    if unit == 0 {
        format!("{n}B")
    } else if value < 10.0 {
        format!("{value:.1}{suffix}")
    } else {
        format!("{value:.0}{suffix}")
    }
}

/// Formats a throughput in bytes per second, e.g. `1.20 MB/s`.
pub fn rate(bytes_per_sec: f64) -> String {
    format!("{}/s", bytes(bytes_per_sec.max(0.0).round() as u64))
}

/// Formats a duration in seconds as `3d 4h`, `4h 12m` or `12m 3s`.
pub fn duration(secs: u64) -> String {
    let (d, h, m, s) = (
        secs / 86_400,
        secs % 86_400 / 3600,
        secs % 3600 / 60,
        secs % 60,
    );
    if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}h {m:02}m")
    } else if m > 0 {
        format!("{m}m {s:02}s")
    } else {
        format!("{s}s")
    }
}

/// Truncates `s` to at most `width` characters, ending with `…` when cut.
pub fn truncate(s: &str, width: usize) -> String {
    if s.chars().count() <= width {
        return s.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut out: String = s.chars().take(width - 1).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_scales_through_units() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(1023), "1023 B");
        assert_eq!(bytes(1024), "1.00 KB");
        assert_eq!(bytes(15 * 1024 * 1024), "15.0 MB");
        assert_eq!(bytes(24 * 1024 * 1024 * 1024), "24.0 GB");
        assert_eq!(bytes(460 * 1024 * 1024 * 1024), "460 GB");
    }

    #[test]
    fn bytes_short_is_compact() {
        assert_eq!(bytes_short(512), "512B");
        assert_eq!(bytes_short(3 * 1024 * 1024 * 1024 / 2), "1.5G");
        assert_eq!(bytes_short(24 * 1024 * 1024 * 1024), "24G");
    }

    #[test]
    fn rate_never_negative() {
        assert_eq!(rate(-5.0), "0 B/s");
        assert_eq!(rate(2048.0), "2.00 KB/s");
    }

    #[test]
    fn duration_picks_two_largest_units() {
        assert_eq!(duration(42), "42s");
        assert_eq!(duration(125), "2m 05s");
        assert_eq!(duration(3 * 3600 + 7 * 60), "3h 07m");
        assert_eq!(duration(3 * 86_400 + 4 * 3600 + 59), "3d 4h");
    }

    #[test]
    fn truncate_adds_ellipsis() {
        assert_eq!(truncate("Safari", 10), "Safari");
        assert_eq!(truncate("com.apple.WebKit.WebContent", 10), "com.apple…");
        assert_eq!(truncate("abc", 0), "");
    }
}
