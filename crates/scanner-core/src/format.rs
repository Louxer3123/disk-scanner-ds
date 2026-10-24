/// Formats a byte count with a binary unit and three significant digits:
/// `512 B`, `1.50 KB`, `15.0 MB`, `150 GB`. No thousand separators, as in the original Scanner.
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["KB", "MB", "GB", "TB", "PB", "EB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    // Switch units before rounding could print "1024 KB".
    while value >= 999.5 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    let decimals = if value < 9.995 {
        2
    } else if value < 99.95 {
        1
    } else {
        0
    };
    format!("{value:.decimals$} {}", UNITS[unit])
}

#[cfg(test)]
mod tests {
    use super::format_size;

    #[test]
    fn picks_unit_and_precision() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1024), "1.00 KB");
        assert_eq!(format_size(1536), "1.50 KB");
        assert_eq!(format_size(15 * 1024 * 1024), "15.0 MB");
        assert_eq!(format_size(150 * 1024 * 1024 * 1024), "150 GB");
        assert_eq!(format_size(3 * (1u64 << 40)), "3.00 TB");
        assert_eq!(format_size(u64::MAX), "16.0 EB");
    }

    #[test]
    fn never_prints_four_digit_values() {
        assert_eq!(format_size(1023 * 1024), "1.00 MB");
        assert_eq!(format_size(999 * 1024), "999 KB");
    }
}
