//! Conditions: the `when = { … }` tables a profile uses to pick rows, kinds,
//! states, panels and receipts.
//!
//! Every key narrows the match and keys AND together; a key given a list
//! matches any one of the values. An empty `Match` matches every bead.

use super::Source;
use crate::beads::Bead;
use chrono::{DateTime, NaiveDate, Utc};
use serde::de::{self, Deserializer, IntoDeserializer, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::marker::PhantomData;

/// A condition on one bead. See docs/profile.md for each key.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Match {
    /// The stored status is one of these.
    #[serde(deserialize_with = "one_or_many")]
    pub status: Vec<String>,
    /// The issue type is one of these.
    #[serde(rename = "type", deserialize_with = "one_or_many")]
    pub issue_type: Vec<String>,
    /// Every one of these sources has a value.
    #[serde(deserialize_with = "one_or_many")]
    pub has: Vec<Source>,
    /// None of these sources has a value.
    #[serde(deserialize_with = "one_or_many")]
    pub lacks: Vec<Source>,
    /// Each source has one of the listed values, compared as text: a
    /// metadata `true` equals `"true"`, and a list matches when any element
    /// does.
    pub eq: BTreeMap<Source, OneOrMany>,
    /// The bead carries one of these labels exactly.
    #[serde(deserialize_with = "one_or_many")]
    pub label: Vec<String>,
    /// The bead carries a label starting with one of these.
    #[serde(deserialize_with = "one_or_many")]
    pub label_prefix: Vec<String>,
    /// The bead id starts with one of these.
    #[serde(deserialize_with = "one_or_many")]
    pub id_prefix: Vec<String>,
    /// Whether `bd ready` listed the bead.
    pub ready: Option<bool>,
    /// Each source holds a timestamp before now, or a date before today (UTC).
    #[serde(deserialize_with = "one_or_many")]
    pub past: Vec<Source>,
    /// At least one of these matches (ignored when empty).
    pub any: Vec<Match>,
    /// This does not match.
    pub not: Option<Box<Match>>,
}

/// A value or a list of values, as text. `eq = { "a.b" = "x" }` and
/// `eq = { "a.b" = ["x", "y"] }` are both allowed; booleans and numbers are
/// read as their text.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct OneOrMany(pub Vec<String>);

impl<'de> Deserialize<'de> for OneOrMany {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        one_or_many(d).map(OneOrMany)
    }
}

impl Match {
    /// Whether the bead meets every key of this condition at `now`.
    pub fn matches(&self, bead: &Bead, now: DateTime<Utc>) -> bool {
        let one_of =
            |list: &[String], value: &str| list.is_empty() || list.iter().any(|v| v == value);
        one_of(&self.status, &bead.status)
            && one_of(&self.issue_type, &bead.issue_type)
            && self.has.iter().all(|s| s.value(bead).is_some())
            && !self.lacks.iter().any(|s| s.value(bead).is_some())
            && self.eq.iter().all(|(source, wanted)| {
                source
                    .texts(bead)
                    .iter()
                    .any(|text| wanted.0.iter().any(|w| w == text))
            })
            && (self.label.is_empty() || bead.labels.iter().any(|l| self.label.contains(l)))
            && (self.label_prefix.is_empty()
                || bead
                    .labels
                    .iter()
                    .any(|l| self.label_prefix.iter().any(|p| l.starts_with(p.as_str()))))
            && (self.id_prefix.is_empty()
                || self
                    .id_prefix
                    .iter()
                    .any(|p| bead.id.starts_with(p.as_str())))
            && self.ready.is_none_or(|r| r == bead.ready)
            && self
                .past
                .iter()
                .all(|s| s.text(bead).is_some_and(|t| is_past(&t, now)))
            && (self.any.is_empty() || self.any.iter().any(|m| m.matches(bead, now)))
            && self.not.as_ref().is_none_or(|m| !m.matches(bead, now))
    }

    /// Every source this condition reads, for validation.
    pub(crate) fn sources(&self) -> Vec<&Source> {
        let mut out: Vec<&Source> = self
            .has
            .iter()
            .chain(&self.lacks)
            .chain(self.eq.keys())
            .chain(&self.past)
            .collect();
        for m in &self.any {
            out.extend(m.sources());
        }
        if let Some(m) = &self.not {
            out.extend(m.sources());
        }
        out
    }
}

/// A timestamp before `now`, or a `YYYY-MM-DD` date before today (UTC).
pub fn is_past(text: &str, now: DateTime<Utc>) -> bool {
    if let Ok(t) = DateTime::parse_from_rfc3339(text) {
        return t.with_timezone(&Utc) < now;
    }
    NaiveDate::parse_from_str(text.get(..10).unwrap_or(text), "%Y-%m-%d")
        .is_ok_and(|d| d < now.date_naive())
}

/// Deserializes either one value or a list of values into a `Vec`, keeping
/// the inner error (so a bad source names itself). Booleans and numbers are
/// passed on as text.
pub(crate) fn one_or_many<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct V<T>(PhantomData<T>);
    impl<'de, T: Deserialize<'de>> Visitor<'de> for V<T> {
        type Value = Vec<T>;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a value or a list of values")
        }
        fn visit_str<E: de::Error>(self, v: &str) -> Result<Vec<T>, E> {
            T::deserialize(v.to_string().into_deserializer()).map(|t| vec![t])
        }
        fn visit_bool<E: de::Error>(self, v: bool) -> Result<Vec<T>, E> {
            self.visit_str(&v.to_string())
        }
        fn visit_i64<E: de::Error>(self, v: i64) -> Result<Vec<T>, E> {
            self.visit_str(&v.to_string())
        }
        fn visit_u64<E: de::Error>(self, v: u64) -> Result<Vec<T>, E> {
            self.visit_str(&v.to_string())
        }
        fn visit_f64<E: de::Error>(self, v: f64) -> Result<Vec<T>, E> {
            self.visit_str(&v.to_string())
        }
        fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Vec<T>, A::Error> {
            Vec::<T>::deserialize(de::value::SeqAccessDeserializer::new(seq))
        }
    }
    d.deserialize_any(V(PhantomData))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse_when(text: &str) -> Result<Match, String> {
        #[derive(Deserialize)]
        struct W {
            when: Match,
        }
        toml::from_str::<W>(text)
            .map(|w| w.when)
            .map_err(|e| e.to_string())
    }

    fn when(text: &str) -> Match {
        parse_when(text).unwrap()
    }

    fn bead() -> Bead {
        Bead {
            id: "wk-1".into(),
            status: "open".into(),
            issue_type: "task".into(),
            labels: vec!["area:billing".into(), "docs".into()],
            ready: true,
            metadata: json!({
                "work": {"fields": {"gate": "true", "due": "2026-11-01", "tags": ["a", "b"], "done": true}}
            }),
            ..Default::default()
        }
    }

    #[test]
    fn match_grammar_ands_keys_and_lists_mean_one_of() {
        let now = DateTime::parse_from_rfc3339("2026-11-15T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let b = bead();
        for (text, expected) in [
            ("when = {}", true),
            ("when = { status = \"open\" }", true),
            (
                "when = { status = [\"closed\", \"open\"], type = \"task\" }",
                true,
            ),
            ("when = { status = \"open\", type = \"epic\" }", false),
            ("when = { has = \"work.fields.gate\" }", true),
            ("when = { lacks = \"work.suspended\" }", true),
            (
                "when = { has = [\"work.fields.gate\", \"work.nope\"] }",
                false,
            ),
            ("when = { eq = { \"work.fields.gate\" = \"true\" } }", true),
            ("when = { eq = { \"work.fields.done\" = true } }", true),
            (
                "when = { eq = { \"work.fields.tags\" = [\"z\", \"b\"] } }",
                true,
            ),
            ("when = { label = \"docs\" }", true),
            ("when = { label_prefix = \"area:\" }", true),
            ("when = { id_prefix = [\"x-\", \"wk-\"] }", true),
            ("when = { ready = false }", false),
            ("when = { past = \"work.fields.due\" }", true),
            (
                "when = { any = [{ type = \"epic\" }, { label = \"docs\" }] }",
                true,
            ),
            ("when = { not = { status = \"open\" } }", false),
            ("when = { label = \"field:status\" }", false),
        ] {
            assert_eq!(when(text).matches(&b, now), expected, "{text}");
        }
        assert!(is_past("2026-11-15T11:59:59Z", now));
        assert!(!is_past("2026-11-15", now), "today is not past");
        assert!(!is_past("soon", now));
        let error = parse_when("when = { stauts = \"open\" }").unwrap_err();
        assert!(error.contains("stauts"), "{error}");
    }
}
