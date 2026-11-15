//! Formatting helpers shared by every page: words, short ids and times.

use super::escape;

pub(crate) fn words(value: &str) -> String {
    value.replace('_', " ")
}

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
