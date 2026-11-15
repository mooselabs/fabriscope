//! Formatting helpers shared by every page: words, short ids, times,
//! durations, money, tokens, counts and percentages. Each returns plain text
//! except `when`, which returns a `<time>` element; escape the text ones.

use super::escape;

/// Underscores as spaces: `in_progress` → `in progress`.
pub(crate) fn words(value: &str) -> String {
    value.replace('_', " ")
}

/// The first nine characters, for commits and revisions.
pub(crate) fn short(value: &str) -> &str {
    value.get(..9).unwrap_or(value)
}

/// An RFC 3339 instant as a short UTC time, with the full value on hover.
pub(crate) fn when(at: &str) -> String {
    let short = chrono::DateTime::parse_from_rfc3339(at)
        .map(|t| {
            t.with_timezone(&chrono::Utc)
                .format("%-d %b %H:%M")
                .to_string()
        })
        .unwrap_or_else(|_| at.to_string());
    format!(
        "<time datetime=\"{}\" title=\"{}\">{}</time>",
        escape(at),
        escape(at),
        escape(&short)
    )
}

/// A whole number with thousands separators: `15883933` → `15,883,933`.
pub(crate) fn count(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Dollars to the cent: `3.2328` → `$3.23`.
pub(crate) fn money(usd: f64) -> String {
    if usd < 0.0 {
        format!("-${:.2}", -usd)
    } else {
        format!("${usd:.2}")
    }
}

/// A token count, short: `950` → `950`, `15883933` → `15.9M`, `48200` →
/// `48.2k`.
pub(crate) fn tokens(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.1}k", n as f64 / 1e3),
        1_000_000..1_000_000_000 => format!("{:.1}M", n as f64 / 1e6),
        _ => format!("{:.1}B", n as f64 / 1e9),
    }
}

/// Seconds as the two largest units: `45` → `45s`, `4374` → `1h 12m`,
/// `200000` → `2d 7h`.
pub(crate) fn seconds(total: f64) -> String {
    let s = total.max(0.0).round() as u64;
    let (d, h, m, sec) = (s / 86_400, s / 3_600 % 24, s / 60 % 60, s % 60);
    if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else if m > 0 {
        format!("{m}m {sec}s")
    } else {
        format!("{sec}s")
    }
}

/// Hours with one decimal: `5.24` → `5.2 h`.
pub(crate) fn hours(h: f64) -> String {
    format!("{h:.1} h")
}

/// A fraction as a whole percentage: `0.423` → `42%`.
pub(crate) fn percent(fraction: f64) -> String {
    format!("{:.0}%", fraction * 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_read_the_same_everywhere() {
        assert_eq!(count(15_883_933), "15,883,933");
        assert_eq!(count(7), "7");
        assert_eq!(money(3.2328282), "$3.23");
        assert_eq!(money(-1.0), "-$1.00");
        assert_eq!(tokens(950), "950");
        assert_eq!(tokens(48_200), "48.2k");
        assert_eq!(tokens(15_883_933), "15.9M");
        assert_eq!(seconds(45.0), "45s");
        assert_eq!(seconds(4374.0), "1h 12m");
        assert_eq!(seconds(200_000.0), "2d 7h");
        assert_eq!(seconds(125.0), "2m 5s");
        assert_eq!(hours(5.24), "5.2 h");
        assert_eq!(percent(0.423), "42%");
    }
}
