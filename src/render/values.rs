//! One field value as HTML, by the kind a profile panel or receipt field
//! gives it. Every string is escaped; only `https://` values ever become
//! external links; identifiers link inside the server when they resolve.

use super::fmt::{count, money, seconds, when};
use super::markdown::{Links, markdown};
use super::{bead_link, escape, person_link, scroll_link, tone_badge, words};
use crate::model::View;
use crate::profile::{FieldKind, IdTarget, Tone, text_of};
use serde_json::Value;

/// What a value needs besides itself: the view (to resolve ids and
/// people), a unit for counts and durations, a `[links.repos]` name for
/// commits, and a base path for relative Markdown links.
pub(crate) struct Ctx<'a> {
    pub view: &'a View,
    pub unit: Option<&'a str>,
    pub repo: Option<&'a str>,
    pub base: Option<&'a str>,
}

impl<'a> Ctx<'a> {
    /// A context with no unit, repo or base.
    pub(crate) fn new(view: &'a View) -> Self {
        Self {
            view,
            unit: None,
            repo: None,
            base: None,
        }
    }
}

/// A value as HTML for its kind. A null or empty value gives `""` (the
/// caller decides whether to show a dash); a value the kind cannot read
/// falls back to escaped text.
pub(crate) fn value(kind: FieldKind, v: &Value, ctx: &Ctx) -> String {
    let Some(text) = text_of(v) else {
        return String::new();
    };
    let unit = ctx
        .unit
        .map(|u| format!(" {}", escape(u)))
        .unwrap_or_default();
    match kind {
        FieldKind::Text => escape(&text),
        FieldKind::List => match v {
            Value::Array(items) => {
                let lis: String = items
                    .iter()
                    .filter_map(text_of)
                    .map(|t| format!("<li>{}</li>", escape(&t)))
                    .collect();
                format!("<ul class=\"values\">{lis}</ul>")
            }
            _ => escape(&text),
        },
        FieldKind::Markdown => markdown(&text, ctx.base, &Links::new(ctx.view)),
        FieldKind::Duration => match v.as_f64() {
            Some(s) => format!("{}{unit}", escape(&seconds(s))),
            None => match parse_duration(&text) {
                Some(d) => escape(&d.display()),
                None => escape(&text),
            },
        },
        FieldKind::Money => match v
            .as_f64()
            .or_else(|| text.trim_start_matches('$').parse().ok())
        {
            Some(usd) => escape(&money(usd)),
            None => escape(&text),
        },
        FieldKind::Link => list(v, link),
        FieldKind::IdRef => list(v, |t| id_ref(t, ctx)),
        FieldKind::EnumBadge => tone_badge(&words(&text), Tone::Neutral),
        FieldKind::Json => format!(
            "<pre class=\"json\">{}</pre>",
            escape(&serde_json::to_string_pretty(v).unwrap_or_default())
        ),
        FieldKind::Timestamp => when(&text),
        FieldKind::Code => code(&text, ctx),
        FieldKind::Count => match v.as_u64().or_else(|| text.parse().ok()) {
            Some(n) => format!("{}{unit}", count(n)),
            None => escape(&text),
        },
        FieldKind::Bool => match text.as_str() {
            "true" => "yes".into(),
            "false" => "no".into(),
            other => escape(other),
        },
        FieldKind::Person => list(v, |t| person_link(&ctx.view.profile.person(t).name)),
    }
}

/// Each element of a list through `one`, joined; a scalar through `one`.
fn list(v: &Value, one: impl Fn(&str) -> String) -> String {
    match v {
        Value::Array(items) => items
            .iter()
            .filter_map(text_of)
            .map(|t| one(&t))
            .collect::<Vec<_>>()
            .join(", "),
        other => text_of(other).map(|t| one(&t)).unwrap_or_default(),
    }
}

/// A link to `href` when it is `https://`; otherwise `text` alone.
pub(crate) fn external(href: &str, text: &str) -> String {
    if href.to_ascii_lowercase().starts_with("https://") {
        format!(
            "<a href=\"{}\" rel=\"noreferrer\">{}</a>",
            escape(href),
            escape(text)
        )
    } else {
        escape(text)
    }
}

/// An `https://` value as a link; anything else as text.
fn link(text: &str) -> String {
    external(text, text.trim_start_matches("https://"))
}

/// An identifier: a bead it names (by id or external ref), else the first
/// `[links.ids]` rule whose prefix it has, else text.
fn id_ref(text: &str, ctx: &Ctx) -> String {
    if let Some(bead) = ctx.view.snapshot().and_then(|s| s.find(text)) {
        return bead_link(&bead.id, text);
    }
    for rule in &ctx.view.profile.links.ids {
        if !text.starts_with(rule.prefix.as_str()) {
            continue;
        }
        match rule.target {
            IdTarget::Bead => {}
            IdTarget::ScrollAlias => {
                if let Some(scroll) = ctx.view.scrolls.as_ref().and_then(|f| {
                    f.data
                        .scrolls
                        .iter()
                        .find(|s| s.aliases.iter().any(|a| a == text))
                }) {
                    return scroll_link(&scroll.id, text);
                }
            }
            IdTarget::Template => {
                if let Some(template) = &rule.template {
                    return external(&template.replace("{value}", text), text);
                }
            }
        }
    }
    escape(text)
}

/// A code value; with a repo and a commit template, a 40-hex value links to
/// the commit.
fn code(text: &str, ctx: &Ctx) -> String {
    let links = &ctx.view.profile.links;
    let base = ctx.repo.and_then(|r| links.repos.get(r));
    let sha = text.len() == 40 && text.bytes().all(|b| b.is_ascii_hexdigit());
    match (base, &links.commit) {
        (Some(base), Some(template)) if sha => {
            let href = template.replace("{repo}", base).replace("{value}", text);
            if href.starts_with("https://") {
                return format!(
                    "<a href=\"{}\" rel=\"noreferrer\"><code title=\"{}\">{}</code></a>",
                    escape(&href),
                    escape(text),
                    escape(&text[..7])
                );
            }
            format!("<code>{}</code>", escape(text))
        }
        _ => format!("<code>{}</code>", escape(text)),
    }
}

/// A duration a person wrote: `2h`, `90m`, `1d`, `2-3h`, `4–6h active
/// work`. `min` and `max` are seconds (equal for a single value); `rest` is
/// any trailing words.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Written {
    pub min: f64,
    pub max: f64,
    pub rest: String,
}

impl Written {
    /// `2–3 h active work`, `2 h`, `90 m`.
    pub(crate) fn display(&self) -> String {
        let (scale, unit) = if self.max >= 86_400.0 && self.min % 86_400.0 == 0.0 {
            (86_400.0, "d")
        } else if self.max >= 3_600.0 {
            (3_600.0, "h")
        } else {
            (60.0, "m")
        };
        let n = |s: f64| {
            let v = s / scale;
            if v.fract() == 0.0 {
                format!("{v:.0}")
            } else {
                format!("{v:.1}")
            }
        };
        let amount = if self.min == self.max {
            n(self.min)
        } else {
            format!("{}–{}", n(self.min), n(self.max))
        };
        let rest = if self.rest.is_empty() {
            String::new()
        } else {
            format!(" {}", self.rest)
        };
        format!("{amount} {unit}{rest}")
    }
}

/// Parses a written duration, or `None` when the text is not one.
pub(crate) fn parse_duration(text: &str) -> Option<Written> {
    let text = text.trim();
    let end = text
        .find(|c: char| {
            !(c.is_ascii_digit() || c == '.' || c == '-' || c == '–' || c.is_whitespace())
        })
        .unwrap_or(text.len());
    let (numbers, tail) = text.split_at(end);
    let unit_len = tail
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(tail.len());
    let (unit, rest) = tail.split_at(unit_len);
    let scale = match unit.to_ascii_lowercase().as_str() {
        "s" | "sec" | "secs" | "second" | "seconds" => 1.0,
        "m" | "min" | "mins" | "minute" | "minutes" => 60.0,
        "h" | "hr" | "hrs" | "hour" | "hours" => 3_600.0,
        "d" | "day" | "days" => 86_400.0,
        _ => return None,
    };
    let parts: Vec<f64> = numbers
        .split(['-', '–'])
        .map(|p| p.trim().parse::<f64>())
        .collect::<Result<_, _>>()
        .ok()?;
    let (min, max) = match parts.as_slice() {
        [one] => (*one, *one),
        [a, b] if a <= b => (*a, *b),
        _ => return None,
    };
    Some(Written {
        min: min * scale,
        max: max * scale,
        rest: rest.trim().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn durations_people_write_are_read_and_others_stay_text() {
        let d = parse_duration("2-3h").unwrap();
        assert_eq!((d.min, d.max), (7200.0, 10800.0));
        assert_eq!(d.display(), "2–3 h");
        assert_eq!(parse_duration("2h").unwrap().display(), "2 h");
        let active = parse_duration("4-6h active work").unwrap();
        assert_eq!(active.rest, "active work");
        assert_eq!(active.display(), "4–6 h active work");
        assert_eq!(parse_duration("90m").unwrap().max, 5400.0);
        assert_eq!(parse_duration("1.5 hours").unwrap().display(), "1.5 h");
        assert_eq!(parse_duration("2 days").unwrap().display(), "2 d");
        for text in ["soon", "3-2h", "h", "2x", ""] {
            assert!(parse_duration(text).is_none(), "{text}");
        }
    }

    #[test]
    fn every_field_kind_escapes_and_links_only_https() {
        let view = crate::testing::work_view();
        let ctx = Ctx::new(&view);
        let evil = json!("<script>x</script>");
        for kind in [
            FieldKind::Text,
            FieldKind::List,
            FieldKind::Markdown,
            FieldKind::Duration,
            FieldKind::Money,
            FieldKind::Link,
            FieldKind::IdRef,
            FieldKind::EnumBadge,
            FieldKind::Json,
            FieldKind::Timestamp,
            FieldKind::Code,
            FieldKind::Count,
            FieldKind::Bool,
            FieldKind::Person,
        ] {
            let html = value(kind, &evil, &ctx);
            assert!(!html.contains("<script>"), "{kind:?}: {html}");
            assert_eq!(value(kind, &Value::Null, &ctx), "", "{kind:?}");
        }
        assert_eq!(value(FieldKind::Duration, &json!(4374), &ctx), "1h 12m");
        assert_eq!(value(FieldKind::Duration, &json!("2-3h"), &ctx), "2–3 h");
        assert_eq!(value(FieldKind::Money, &json!(4.5), &ctx), "$4.50");
        assert_eq!(value(FieldKind::Bool, &json!("false"), &ctx), "no");
        assert_eq!(value(FieldKind::Bool, &json!(true), &ctx), "yes");
        assert_eq!(value(FieldKind::Count, &json!(1234), &ctx), "1,234");
        assert!(
            value(FieldKind::Link, &json!("javascript:alert(1)"), &ctx).starts_with("javascript")
        );
        assert!(!value(FieldKind::Link, &json!("http://x.example"), &ctx).contains("href"));
        assert!(
            value(FieldKind::Link, &json!(["https://a.example/1"]), &ctx)
                .contains("href=\"https://a.example/1\"")
        );
        let ids = value(
            FieldKind::IdRef,
            &json!(["WORK-0002", "ADR-0007", "NOPE-1"]),
            &ctx,
        );
        assert!(ids.contains("href=\"/bead/wk%2D2\">WORK-0002</a>"), "{ids}");
        assert!(
            ids.contains("href=\"https://example.com/adr/ADR-0007\""),
            "{ids}"
        );
        assert!(ids.ends_with("NOPE-1"));
        assert!(
            value(FieldKind::Person, &json!("factory/hostA/rigA"), &ctx)
                .contains("/person/factory%2FhostA%2FrigA")
        );
        let commit = Ctx {
            repo: Some("widgets"),
            ..Ctx::new(&view)
        };
        assert!(
            value(
                FieldKind::Code,
                &json!("9eedcb590af984bdf4520a8939840bd61d4979fe"),
                &commit
            )
            .contains("href=\"https://example.com/acme/widgets/commit/9eedcb5")
        );
        assert!(value(FieldKind::List, &json!(["a", "b"]), &ctx).contains("<li>b</li>"));
    }
}
