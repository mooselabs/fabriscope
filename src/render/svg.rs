//! Server-rendered SVG charts. Colours and strokes come from CSS classes
//! (`chart`, `bar`, `tone-*`, `series-*`, `gridline`, `point`, `range`):
//! the CSP forbids inline `style=`, so nothing here writes one. Every label
//! is escaped.

use super::{escape, tone_class};
use crate::beads;
use crate::profile::Tone;
use std::fmt::Write;

/// The overview's open-beads-over-time chart.
pub(crate) fn chart(history: &beads::History) -> String {
    let days = &history.days;
    if days.is_empty() {
        return String::new();
    }
    let max = days
        .iter()
        .map(|d| d.end.max(d.start))
        .max()
        .unwrap_or(1)
        .max(1) as f64
        * 1.15;
    let y = |value: usize| 190.0 - (value as f64 / max * 156.0);
    let x = |i: usize| 58.0 + i as f64 / days.len().max(1) as f64 * 802.0;
    let mut out = "<figure><figcaption>Open beads over time <span class=\"legend\"><span class=\"total-key\">Open at the end of each UTC day</span></span></figcaption><svg class=\"chart\" viewBox=\"0 0 900 235\" role=\"img\" aria-label=\"Open bead count at UTC day boundaries\">".to_string();
    for fraction in [0.0, 0.5, 1.0] {
        let value = (max * fraction) as usize;
        write!(out, "<line class=\"gridline\" x1=\"58\" x2=\"860\" y1=\"{}\" y2=\"{}\"/><text x=\"48\" y=\"{}\" text-anchor=\"end\">{value}</text>", y(value), y(value), y(value) + 4.0).unwrap();
    }
    let mut points = format!("{},{}", x(0), y(days[0].start));
    for (i, day) in days.iter().enumerate() {
        write!(points, " {},{}", x(i + 1), y(day.end)).unwrap();
    }
    write!(out, "<polyline class=\"total-line\" points=\"{points}\"/>").unwrap();
    for (i, day) in days.iter().enumerate() {
        write!(out, "<circle class=\"point\" cx=\"{}\" cy=\"{}\" r=\"4\"><title>{}: {} open · {} added · {} closed</title></circle>", x(i + 1), y(day.end), day.date, day.end, day.added, day.closed).unwrap();
    }
    let last = &days[days.len() - 1];
    write!(out, "<text x=\"58\" y=\"220\">{}</text><text x=\"860\" y=\"220\" text-anchor=\"end\">{} · end of day</text></svg></figure>", days[0].date, last.date).unwrap();
    out
}

/// One bar of `hbars`.
pub(crate) struct Bar {
    pub label: String,
    pub value: f64,
    /// Hover text.
    pub title: String,
    pub tone: Tone,
}

/// One row of `range_bars`: the median and 90th percentile of `n` samples.
pub(crate) struct Spread {
    pub label: String,
    pub p50: f64,
    pub p90: f64,
    pub n: usize,
}

fn figure(caption: &str, class: &str, height: f64, inner: &str) -> String {
    format!(
        "<figure><figcaption>{}</figcaption><svg class=\"chart {class}\" viewBox=\"0 0 900 {height}\" role=\"img\" aria-label=\"{}\">{inner}</svg></figure>",
        escape(caption),
        escape(caption)
    )
}

fn top(values: impl Iterator<Item = f64>) -> f64 {
    values.fold(0.0_f64, f64::max).max(1e-9)
}

/// Horizontal bars, one row each: label, bar (toned), value as `format`
/// writes it.
pub(crate) fn hbars(caption: &str, bars: &[Bar], format: fn(f64) -> String) -> String {
    if bars.is_empty() {
        return String::new();
    }
    let max = top(bars.iter().map(|b| b.value));
    let mut out = String::new();
    for (i, bar) in bars.iter().enumerate() {
        let y = 10.0 + i as f64 * 28.0;
        let w = (bar.value.max(0.0) / max * 520.0).max(1.0);
        write!(out, "<text x=\"230\" y=\"{:.1}\" text-anchor=\"end\">{}</text><rect class=\"bar tone-{}\" x=\"240\" y=\"{y:.1}\" width=\"{w:.1}\" height=\"18\"><title>{}</title></rect><text x=\"{:.1}\" y=\"{:.1}\">{}</text>", y + 13.0, escape(&bar.label), tone_class(bar.tone), escape(&bar.title), 248.0 + w, y + 13.0, escape(&format(bar.value))).unwrap();
    }
    figure(caption, "hbars", 20.0 + bars.len() as f64 * 28.0, &out)
}

/// Vertical bars in groups: for each (group label, values) one bar per
/// series, coloured `series-0`, `series-1`, …, with a legend.
pub(crate) fn grouped_bars(
    caption: &str,
    series: &[&str],
    groups: &[(String, Vec<f64>)],
) -> String {
    if groups.is_empty() || series.is_empty() {
        return String::new();
    }
    let max = top(groups.iter().flat_map(|g| g.1.iter().copied())) * 1.1;
    let slot = 802.0 / groups.len() as f64;
    let width = (slot * 0.8 / series.len() as f64).max(1.0);
    let mut out = String::new();
    for (g, (label, values)) in groups.iter().enumerate() {
        let x0 = 58.0 + g as f64 * slot + slot * 0.1;
        for (k, value) in values.iter().take(series.len()).enumerate() {
            let h = value.max(0.0) / max * 156.0;
            write!(out, "<rect class=\"bar series-{k}\" x=\"{:.1}\" y=\"{:.1}\" width=\"{width:.1}\" height=\"{h:.1}\"><title>{}: {} {value}</title></rect>", x0 + k as f64 * width, 190.0 - h, escape(label), escape(series[k])).unwrap();
        }
        write!(
            out,
            "<text x=\"{:.1}\" y=\"208\" text-anchor=\"middle\">{}</text>",
            x0 + slot * 0.4,
            escape(label)
        )
        .unwrap();
    }
    for (k, name) in series.iter().enumerate() {
        let x = 58.0 + k as f64 * 160.0;
        write!(out, "<rect class=\"bar series-{k}\" x=\"{x:.1}\" y=\"222\" width=\"12\" height=\"12\"/><text x=\"{:.1}\" y=\"233\">{}</text>", x + 18.0, escape(name)).unwrap();
    }
    figure(caption, "grouped", 245.0, &out)
}

/// One row per spread: a bar from the median to the 90th percentile with a
/// dot at the median, values as `format` writes them.
pub(crate) fn range_bars(caption: &str, rows: &[Spread], format: fn(f64) -> String) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let max = top(rows.iter().map(|r| r.p90.max(r.p50)));
    let x = |v: f64| 240.0 + v.max(0.0) / max * 520.0;
    let mut out = String::new();
    for (i, row) in rows.iter().enumerate() {
        let y = 19.0 + i as f64 * 28.0;
        write!(out, "<text x=\"230\" y=\"{:.1}\" text-anchor=\"end\">{}</text><line class=\"range\" x1=\"{:.1}\" x2=\"{:.1}\" y1=\"{y:.1}\" y2=\"{y:.1}\"/><circle class=\"point\" cx=\"{:.1}\" cy=\"{y:.1}\" r=\"5\"><title>{}: median {} · p90 {} · n {}</title></circle><text x=\"{:.1}\" y=\"{:.1}\">{} – {}</text>", y + 4.0, escape(&row.label), x(row.p50), x(row.p90), x(row.p50), escape(&row.label), escape(&format(row.p50)), escape(&format(row.p90)), row.n, x(row.p90) + 10.0, y + 4.0, escape(&format(row.p50)), escape(&format(row.p90))).unwrap();
    }
    figure(caption, "ranges", 20.0 + rows.len() as f64 * 28.0, &out)
}

/// Vertical count bars, one per age bucket, left (newest) to right (oldest).
pub(crate) fn age_bars(caption: &str, buckets: &[(String, usize)]) -> String {
    if buckets.is_empty() {
        return String::new();
    }
    let max = top(buckets.iter().map(|b| b.1 as f64)) * 1.1;
    let slot = 802.0 / buckets.len() as f64;
    let mut out = String::new();
    for (i, (label, n)) in buckets.iter().enumerate() {
        let h = *n as f64 / max * 156.0;
        let x = 58.0 + i as f64 * slot + slot * 0.15;
        write!(out, "<rect class=\"bar tone-neutral\" x=\"{x:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{h:.1}\"><title>{}: {n}</title></rect><text x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"middle\">{n}</text><text x=\"{:.1}\" y=\"208\" text-anchor=\"middle\">{}</text>", 190.0 - h, slot * 0.7, escape(label), x + slot * 0.35, 186.0 - h, x + slot * 0.35, escape(label)).unwrap();
    }
    figure(caption, "ages", 220.0, &out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svg_helpers_escape_and_never_write_inline_styles() {
        let label = || "<b>&\"x\"".to_string();
        let charts = [
            hbars(
                "Cost",
                &[Bar {
                    label: label(),
                    value: 2.5,
                    title: label(),
                    tone: Tone::Bad,
                }],
                super::super::fmt::money,
            ),
            grouped_bars("Flow", &["created", "closed"], &[(label(), vec![1.0, 2.0])]),
            range_bars(
                "Cycle",
                &[Spread {
                    label: label(),
                    p50: 3600.0,
                    p90: 7200.0,
                    n: 4,
                }],
                super::super::fmt::seconds,
            ),
            age_bars("Ageing", &[(label(), 3), ("8–14d".into(), 0)]),
        ];
        for chart in &charts {
            assert!(chart.starts_with("<figure>"), "{chart}");
            assert!(!chart.contains("style="), "{chart}");
            assert!(!chart.contains("<b>"), "{chart}");
            assert!(chart.contains("&lt;b&gt;"));
        }
        assert!(charts[0].contains("tone-bad"));
        assert!(charts[2].contains("1h 0m – 2h 0m"));
        assert!(age_bars("x", &[]).is_empty());
    }
}
