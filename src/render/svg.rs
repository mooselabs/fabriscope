//! Server-rendered SVG charts.

use crate::beads;
use std::fmt::Write;

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
