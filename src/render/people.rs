// Owned by WP3 — History, activity, people. Frozen signatures:
// `people(&View, &Facets) -> String`,
// `person(&View, &str, &Facets) -> Option<String>`.
//! `/people` and `/person/{name}`: who did what, factory seats classed apart
//! from people (`Profile::person`).
//!
//! A person's roles come from the profile: bead-field roles from the
//! snapshot (`RowFacts::people`) and event roles from history threads
//! (`View::person_in_role`). "Holding now" is the assignee of an item not
//! closed. Activity counts history events, comments and provenance events
//! the person made, in the page's date range (the last seven days unless the
//! query names one).

use super::activity::{self, filter_form, range, range_links, range_words};
use super::fmt::{when, words};
use super::layout::page;
use super::table::bead_table;
use super::{escape, group_link, heading, person_link};
use crate::beads::{Bead, Snapshot};
use crate::model::View;
use crate::query::{Facets, href};
use chrono::{DateTime, NaiveDate, Utc};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write;

/// Rows shown per role on a person's page.
const ROLE_ROWS: usize = 50;
/// Entries of recent activity on a person's page.
const RECENT: usize = 50;

/// The facets that select items: every state unless one is named, and no
/// person (pages here filter by person themselves).
fn item_facets(view: &View, facets: &Facets) -> Facets {
    let mut items = facets.clone();
    items.person.clear();
    items.page.clear();
    if view.profile.group(activity::EVENT_KEY).is_none() {
        items.groups.remove(activity::EVENT_KEY);
    }
    if items.state.is_empty() {
        items.state = "all".into();
    }
    items
}

/// What one person did, as `/people` counts it.
#[derive(Default)]
struct Tally {
    class: String,
    holding: usize,
    /// Per profile role, in profile order.
    roles: Vec<usize>,
    closed: usize,
    active: usize,
    last: Option<(DateTime<Utc>, String)>,
}

fn in_bounds(day: NaiveDate, bounds: (Option<NaiveDate>, Option<NaiveDate>)) -> bool {
    bounds.0.is_none_or(|f| day >= f) && bounds.1.is_none_or(|t| day <= t)
}

fn time(at: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(at)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// Every actor time in memory: history events, comments and provenance
/// events, as `(raw name, time text)`, items only.
fn actions<'a>(view: &'a View, s: &'a Snapshot) -> impl Iterator<Item = (&'a str, &'a str)> {
    let item = move |id: &str| s.position(id).is_some_and(|i| s.derived.is_item(i));
    let threads = view
        .history
        .iter()
        .flat_map(|h| h.threads.iter())
        .filter(move |(id, _)| item(id))
        .flat_map(|(_, entry)| {
            let events = entry
                .thread
                .events
                .iter()
                .filter_map(|e| Some((e.actor.as_str(), e.at.as_deref()?)));
            let comments = entry
                .thread
                .comments
                .iter()
                .filter_map(|c| Some((c.author.as_deref()?, c.created_at.as_deref()?)));
            events.chain(comments)
        });
    let trails = view
        .provenance
        .iter()
        .flat_map(|p| p.trails.iter())
        .filter(move |(id, _)| item(id))
        .flat_map(|(_, trail)| {
            trail.events.iter().map(|e| {
                let at = if e.occurred_at.is_empty() {
                    &e.created_at
                } else {
                    &e.occurred_at
                };
                (e.actor.as_str(), at.as_str())
            })
        });
    threads.chain(trails).filter(|(actor, _)| !actor.is_empty())
}

/// Names a person by: the profile's classing of a raw name, cached.
struct Names<'a> {
    view: &'a View,
    cache: HashMap<&'a str, String>,
}

impl<'a> Names<'a> {
    fn new(view: &'a View) -> Self {
        Names {
            view,
            cache: HashMap::new(),
        }
    }

    fn of(&mut self, raw: &'a str) -> &str {
        self.cache
            .entry(raw)
            .or_insert_with(|| self.view.profile.person(raw).name)
    }
}

fn tallies(view: &View, s: &Snapshot, facets: &Facets) -> BTreeMap<String, Tally> {
    let roles = &view.profile.people.roles;
    let items = item_facets(view, facets);
    let bounds = range(facets).bounds(Utc::now());
    let mut out: BTreeMap<String, Tally> = BTreeMap::new();
    let entry = |out: &mut BTreeMap<String, Tally>, name: &str| {
        if !out.contains_key(name) {
            out.insert(
                name.to_string(),
                Tally {
                    class: view.profile.person(name).class,
                    roles: vec![0; roles.len()],
                    ..Default::default()
                },
            );
        }
    };
    let mut names = Names::new(view);
    for (i, bead) in s.beads.iter().enumerate() {
        if !items.matches(view, i) {
            continue;
        }
        let mut involved: BTreeSet<String> = BTreeSet::new();
        if !bead.is_closed() && !bead.assignee.is_empty() {
            let name = names.of(&bead.assignee).to_string();
            entry(&mut out, &name);
            out.get_mut(&name).unwrap().holding += 1;
            involved.insert(name);
        }
        for (r, role) in roles.iter().enumerate() {
            if let Some(name) = view.person_in_role(&bead.id, &role.key) {
                entry(&mut out, &name);
                out.get_mut(&name).unwrap().roles[r] += 1;
                involved.insert(name);
            }
        }
        let closed_in_range = bead.is_closed()
            && bead
                .closed_at
                .as_deref()
                .and_then(time)
                .is_some_and(|t| in_bounds(t.date_naive(), bounds));
        if closed_in_range {
            for name in &involved {
                entry(&mut out, name);
                out.get_mut(name).unwrap().closed += 1;
            }
        }
    }
    let narrowed = item_narrowed(facets);
    for (raw, at_text) in actions(view, s) {
        let Some(at) = time(at_text) else { continue };
        let name = names.of(raw).to_string();
        if narrowed && !out.contains_key(&name) {
            continue;
        }
        entry(&mut out, &name);
        let t = out.get_mut(&name).unwrap();
        if t.last.as_ref().is_none_or(|(last, _)| at > *last) {
            t.last = Some((at, at_text.to_string()));
        }
        if in_bounds(at.date_naive(), bounds) {
            t.active += 1;
        }
    }
    out
}

/// Whether the query narrows the items (then only people tied to those
/// items are listed).
fn item_narrowed(facets: &Facets) -> bool {
    [
        &facets.q,
        &facets.issue_type,
        &facets.kind,
        &facets.priority,
        &facets.state,
        &facets.dstate,
        &facets.ready,
        &facets.epic,
    ]
    .iter()
    .any(|v| !v.is_empty())
        || facets.groups.keys().any(|k| k != activity::EVENT_KEY)
}

/// A class as a table heading: `person` → People, `seat` → Seats.
fn class_heading(class: &str) -> String {
    if class == "person" {
        return "People".into();
    }
    let mut chars = words(class).chars().collect::<Vec<_>>();
    if let Some(first) = chars.first_mut() {
        *first = first.to_ascii_uppercase();
    }
    let mut out: String = chars.into_iter().collect();
    if !out.ends_with('s') {
        out.push('s');
    }
    out
}

fn count_cell(n: usize) -> String {
    if n == 0 {
        "<td class=\"muted\">0</td>".into()
    } else {
        format!("<td>{n}</td>")
    }
}

/// `/people`.
pub(crate) fn people(view: &View, facets: &Facets) -> String {
    let Some(s) = view.snapshot() else {
        return super::layout::not_found(view);
    };
    let now = Utc::now();
    let all = tallies(view, s, facets);
    let mut body = heading(
        "Work",
        "People",
        "Everyone the items name or the history records, with what they hold and did. Factory seats are listed apart from people.",
    );
    body.push_str(&filter_form(view, facets, "/people", None, None));
    body.push_str(&range_links("/people", facets));
    write!(
        body,
        "<p class=\"muted\">{} names · closed and activity counts: {}{}</p>",
        all.len(),
        escape(&range_words(&range(facets), now)),
        activity::coverage(view, s)
    )
    .unwrap();
    if all.is_empty() {
        body.push_str("<p class=\"empty\">No one is named by these items.</p>");
        return page(view, "People", "/people", &body);
    }
    let mut classes: BTreeMap<(bool, &str), Vec<(&String, &Tally)>> = BTreeMap::new();
    for (name, tally) in &all {
        classes
            .entry((tally.class != "person", tally.class.as_str()))
            .or_default()
            .push((name, tally));
    }
    let roles = &view.profile.people.roles;
    for ((_, class), mut members) in classes {
        members.sort_by(|(an, a), (bn, b)| {
            b.last
                .as_ref()
                .map(|l| l.0)
                .cmp(&a.last.as_ref().map(|l| l.0))
                .then_with(|| an.cmp(bn))
        });
        write!(
            body,
            "<section class=\"people-class\" id=\"class-{}\"><div class=\"section-heading\"><h2>{}</h2><span class=\"muted\">{}</span></div><div class=\"table-wrap\"><table class=\"people-table\"><thead><tr><th>Name</th><th>Holding now</th>",
            escape(class),
            escape(&class_heading(class)),
            members.len()
        )
        .unwrap();
        for role in roles {
            write!(body, "<th>{}</th>", escape(&role.label)).unwrap();
        }
        body.push_str("<th>Closed in range</th><th>Activity in range</th><th>Last activity</th></tr></thead><tbody>");
        for (name, t) in members {
            write!(
                body,
                "<tr><td>{}</td>{}",
                person_link(name),
                count_cell(t.holding)
            )
            .unwrap();
            for n in &t.roles {
                body.push_str(&count_cell(*n));
            }
            write!(
                body,
                "{}{}<td>{}</td></tr>",
                count_cell(t.closed),
                count_cell(t.active),
                t.last
                    .as_ref()
                    .map(|(_, text)| when(text))
                    .unwrap_or_else(|| "<span class=\"muted\">unknown</span>".into())
            )
            .unwrap();
        }
        body.push_str("</tbody></table></div></section>");
    }
    page(view, "People", "/people", &body)
}

/// Whether anything in memory names this person.
fn mentioned(view: &View, s: &Snapshot, name: &str) -> bool {
    if s.derived.people.contains_key(name) {
        return true;
    }
    let mut names = Names::new(view);
    s.beads
        .iter()
        .any(|b| !b.assignee.is_empty() && names.of(&b.assignee) == name)
        || view.profile.people.roles.iter().any(|r| {
            s.beads
                .iter()
                .any(|b| view.person_in_role(&b.id, &r.key).as_deref() == Some(name))
        })
        || actions(view, s).any(|(raw, _)| names.of(raw) == name)
}

/// Open items first in display order, then closed ones, newest closed first.
fn ordered(s: &Snapshot, mut items: Vec<usize>) -> Vec<&Bead> {
    let rank: HashMap<usize, usize> = s.order.iter().enumerate().map(|(n, &i)| (i, n)).collect();
    items.sort_by(|&a, &b| {
        let (x, y) = (&s.beads[a], &s.beads[b]);
        x.is_closed().cmp(&y.is_closed()).then_with(|| {
            if x.is_closed() {
                y.closed_at.cmp(&x.closed_at)
            } else {
                rank[&a].cmp(&rank[&b])
            }
        })
    });
    items.into_iter().map(|i| &s.beads[i]).collect()
}

/// `/person/{name}`; `None` for a name nothing mentions.
pub(crate) fn person(view: &View, name: &str, facets: &Facets) -> Option<String> {
    let s = view.snapshot()?;
    if name.is_empty() || !mentioned(view, s, name) {
        return None;
    }
    let class = view.profile.person(name).class;
    let items = item_facets(view, facets);
    let roles = &view.profile.people.roles;
    // Holding now, then each profile role.
    let mut lists: Vec<(String, Vec<usize>)> = vec![("Holding now".into(), Vec::new())];
    lists.extend(roles.iter().map(|r| (r.label.clone(), Vec::new())));
    let mut names = Names::new(view);
    let mut theirs: BTreeSet<usize> = BTreeSet::new();
    for (i, bead) in s.beads.iter().enumerate() {
        if !items.matches(view, i) {
            continue;
        }
        if !bead.is_closed() && !bead.assignee.is_empty() && names.of(&bead.assignee) == name {
            lists[0].1.push(i);
            theirs.insert(i);
        }
        for (r, role) in roles.iter().enumerate() {
            if view.person_in_role(&bead.id, &role.key).as_deref() == Some(name) {
                lists[r + 1].1.push(i);
                theirs.insert(i);
            }
        }
    }
    let eyebrow = if class == "person" {
        "Person".to_string()
    } else {
        format!("Factory · {}", words(&class))
    };
    let mut body = heading(&eyebrow, name, "");
    body.push_str(&filter_form(
        view,
        facets,
        &format!("/person/{}", super::url(name)),
        None,
        None,
    ));
    if class != "person" {
        // The group values this seat's items fall in.
        let mut groups: BTreeMap<&str, BTreeMap<&str, usize>> = BTreeMap::new();
        for &i in &theirs {
            if let Some(row) = s.derived.rows.get(i) {
                for (key, values) in &row.groups {
                    for value in values {
                        *groups.entry(key).or_default().entry(value).or_default() += 1;
                    }
                }
            }
        }
        body.push_str("<section class=\"person-groups\" id=\"person-groups\"><h2>Maps to</h2>");
        if groups.is_empty() {
            body.push_str("<p class=\"empty\">None of these items is in a group.</p>");
        } else {
            body.push_str("<dl class=\"group-map\">");
            for (key, values) in groups {
                let label = view.profile.group(key).map_or(key, |g| g.label.as_str());
                write!(body, "<dt>{}</dt><dd>", escape(label)).unwrap();
                for (value, n) in values {
                    write!(
                        body,
                        "{} <span class=\"muted\">{n}</span> ",
                        group_link(key, value, value)
                    )
                    .unwrap();
                }
                body.push_str("</dd>");
            }
            body.push_str("</dl>");
        }
        body.push_str("</section>");
    }
    for (n, (label, list)) in lists.into_iter().enumerate() {
        let total = list.len();
        write!(
            body,
            "<section class=\"person-role\" id=\"role-{n}\"><div class=\"section-heading\"><h2>{}</h2><span class=\"muted\">{total}</span></div>",
            escape(&label)
        )
        .unwrap();
        let beads = ordered(s, list);
        let rows: Vec<(usize, &Bead)> = beads
            .into_iter()
            .take(ROLE_ROWS)
            .enumerate()
            .map(|(k, b)| (k + 1, b))
            .collect();
        body.push_str(&bead_table(s, view.provenance.as_ref(), &rows));
        if total > ROLE_ROWS {
            write!(
                body,
                "<p class=\"muted\">Showing {ROLE_ROWS} of {total}: open first, then the most recently closed.</p>"
            )
            .unwrap();
        }
        body.push_str("</section>");
    }
    let mut mine = facets.clone();
    mine.person = name.to_string();
    let (recent, total) = activity::recent(view, &mine, RECENT);
    write!(
        body,
        "<section class=\"person-activity\" id=\"person-activity\"><div class=\"section-heading\"><h2>Recent activity</h2><a href=\"{}\">All {total}, {} →</a></div>",
        escape(&href("/activity", &mine, &[])),
        escape(&range_words(&range(facets), Utc::now()))
    )
    .unwrap();
    body.push_str(&range_links(
        &format!("/person/{}", super::url(name)),
        facets,
    ));
    body.push_str(&recent);
    body.push_str("</section>");
    Some(page(view, name, "/people", &body))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facets(query: &str) -> Facets {
        let uri: axum::http::Uri = format!("/x?{query}").parse().unwrap();
        axum::extract::Query::<Facets>::try_from_uri(&uri)
            .unwrap()
            .0
    }

    #[test]
    fn people_are_classed_apart_from_factory_seats() {
        let view = crate::testing::work_view();
        let s = view.snapshot().unwrap();
        let all = tallies(&view, s, &facets("from=2026-10-01"));
        assert_eq!(all["alice"].class, "person");
        assert_eq!(all["factory/hostA/rigA"].class, "seat");
        // Roles: claimant, creator, asked (profile order).
        assert_eq!(all["bob"].roles[2], 1, "asked of bob");
        assert!(
            all["alice"].roles[1] >= 2,
            "alice created wk-1 and wk-2 (events)"
        );
        assert_eq!(all["factory/hostA/rigA"].roles[0], 1, "claims wk-1");
        assert!(all["alice"].last.is_some());
        let html = people(&view, &facets("from=2026-10-01"));
        let seats = html.find("id=\"class-seat\"").unwrap();
        let humans = html.find("id=\"class-person\"").unwrap();
        assert!(humans < seats, "people first");
        let alice = html.find("href=\"/person/alice\"").unwrap();
        let seat = html
            .find("href=\"/person/factory%2FhostA%2FrigA\"")
            .unwrap();
        assert!(alice < seats && seat > seats);
        assert!(html.contains("<th>Asked of</th>"));
        assert!(!html.contains(" style="));
    }

    #[test]
    fn a_person_page_lists_roles_activity_and_seat_groups() {
        let view = crate::testing::work_view();
        let seat = person(&view, "factory/hostA/rigA", &facets("from=2026-10-01")).unwrap();
        assert!(seat.contains("Maps to"));
        assert!(seat.contains("href=\"/by/rig/rigA\""), "{seat}");
        assert!(seat.contains("href=\"/bead/wk%2D1\""));
        assert!(seat.contains("Recent activity"));
        assert!(seat.contains("/activity?person=factory%2FhostA%2FrigA"));
        let bob = person(&view, "bob", &facets("from=2026-10-01")).unwrap();
        assert!(!bob.contains("Maps to"));
        assert!(bob.contains("href=\"/bead/wk%2Dq1\""));
        assert!(person(&view, "nobody", &Facets::default()).is_none());
        assert!(person(&view, "", &Facets::default()).is_none());
    }
}
