//! Markdown documents, and the links inside them.

use super::url;
use crate::model::View;
use crate::{beads, scrolls};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd, html};
use std::collections::HashMap;

/// Turns a link in a document into a safe destination: external web links
/// pass through, references to a bead or scroll on show become local links,
/// and everything else goes nowhere.
pub struct Links<'a> {
    beads: Option<&'a beads::Snapshot>,
    scrolls: Option<&'a scrolls::Snapshot>,
}

impl<'a> Links<'a> {
    pub fn new(view: &'a View) -> Self {
        Self {
            beads: view.beads.as_ref().map(|f| &*f.data),
            scrolls: view.scrolls.as_ref().map(|f| &*f.data),
        }
    }

    fn bead(&self, key: &str) -> Option<String> {
        let beads = self.beads?;
        let bead = beads.find(key).or_else(|| {
            // A file named after an external reference, e.g. TICKET-12-fix-login.md.
            beads.beads.iter().find(|b| {
                !b.external_ref.is_empty() && key.starts_with(&format!("{}-", b.external_ref))
            })
        })?;
        Some(format!("/bead/{}", url(&bead.id)))
    }

    pub fn resolve(&self, base: Option<&str>, raw: &str) -> String {
        let lower = raw.trim().to_ascii_lowercase();
        if lower.starts_with("https://")
            || lower.starts_with("http://")
            || lower.starts_with("mailto:")
            || raw.starts_with('#')
        {
            return raw.to_string();
        }
        if raw.starts_with("beads://") || (raw.contains(':') && !raw.contains('/')) {
            return scrolls::bead_reference(raw)
                .and_then(|id| self.bead(id))
                .unwrap_or_else(|| "#".into());
        }
        if raw.contains(':') || raw.starts_with('/') || raw.contains('\\') {
            return "#".into();
        }
        let (path, anchor) = raw.split_once('#').unwrap_or((raw, ""));
        let anchor = if anchor.is_empty() {
            String::new()
        } else {
            format!("#{}", url(anchor))
        };
        let decoded = percent_encoding::percent_decode_str(path)
            .decode_utf8_lossy()
            .into_owned();
        if let Some(scrolls) = self.scrolls {
            if let Some(joined) = base.and_then(|b| join(b, &decoded)) {
                if let Some(target) = scrolls.by_path(&joined) {
                    return format!("/scroll/{}{anchor}", url(&target.id));
                }
            }
            let stripped = decoded.trim_start_matches("./").trim_start_matches("../");
            let stripped = stripped
                .split('/')
                .skip_while(|p| *p == ".." || *p == ".")
                .collect::<Vec<_>>()
                .join("/");
            if let Some(target) = scrolls.by_path_suffix(&stripped) {
                return format!("/scroll/{}{anchor}", url(&target.id));
            }
        }
        let stem = decoded
            .rsplit('/')
            .next()
            .unwrap_or("")
            .trim_end_matches(".md");
        if !stem.is_empty() {
            if let Some(local) = self.bead(stem) {
                return local;
            }
        }
        "#".into()
    }
}

/// `relative` resolved against the directory of `base`, or `None` if it
/// climbs above the root.
fn join(base: &str, relative: &str) -> Option<String> {
    let mut parts: Vec<&str> = base.split('/').collect();
    parts.pop();
    for part in relative.split('/') {
        match part {
            ".." => {
                parts.pop()?;
            }
            "." | "" => {}
            part => parts.push(part),
        }
    }
    Some(parts.join("/"))
}

pub fn markdown(text: &str, base: Option<&str>, links: &Links) -> String {
    let mut parsed: Vec<_> = Parser::new_ext(
        text,
        Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS | Options::ENABLE_STRIKETHROUGH,
    )
    .collect();
    let mut heading_counts: HashMap<String, usize> = HashMap::new();
    for i in 0..parsed.len() {
        if matches!(parsed[i], Event::Start(Tag::Heading { .. })) {
            let text: String = parsed[i + 1..]
                .iter()
                .take_while(|event| !matches!(event, Event::End(TagEnd::Heading(_))))
                .filter_map(|event| match event {
                    Event::Text(t) | Event::Code(t) => Some(t.as_ref()),
                    _ => None,
                })
                .collect();
            let slug: String = text
                .to_lowercase()
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_' || c.is_whitespace())
                .map(|c| if c.is_whitespace() { '-' } else { c })
                .collect();
            let count = heading_counts.entry(slug.clone()).or_insert(0);
            let slug = if *count == 0 {
                slug
            } else {
                format!("{slug}-{count}")
            };
            *count += 1;
            if let Event::Start(Tag::Heading { id, .. }) = &mut parsed[i] {
                *id = Some(slug.into());
            }
        }
    }
    let events = parsed.into_iter().map(|event| match event {
        // Documents are content, never executable HTML. Images become links so
        // a document cannot start background requests to third parties.
        Event::Html(text) | Event::InlineHtml(text) => Event::Text(text),
        Event::Start(Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        })
        | Event::Start(Tag::Image {
            link_type,
            dest_url,
            title,
            id,
        }) => Event::Start(Tag::Link {
            link_type,
            dest_url: links.resolve(base, &dest_url).into(),
            title,
            id,
        }),
        Event::End(TagEnd::Image) => Event::End(TagEnd::Link),
        other => other,
    });
    let mut output = String::new();
    html::push_html(&mut output, events);
    output
}

#[cfg(test)]
mod tests {
    use super::{Links, markdown};
    use crate::model::View;
    use crate::render::tests::view;

    #[test]
    fn markdown_never_executes_html_or_follows_unsafe_links() {
        let view = view();
        let links = Links::new(&view);
        let html = markdown(
            "<script>alert(1)</script>\n\n[x](javascript:alert) ![remote](https://example.org/i.png) [secret](../../../.secret) [abs](/etc/passwd)",
            None,
            &links,
        );
        assert!(!html.contains("<script>"));
        assert!(!html.contains("href=\"javascript:"));
        assert!(!html.contains("<img"));
        assert!(!html.contains(".secret"));
        assert!(!html.contains("/etc/passwd"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("href=\"https://example.org/i.png\""));
    }

    #[test]
    fn links_reach_scrolls_and_beads_across_stores() {
        let view = view();
        let links = Links::new(&view);
        assert_eq!(
            links.resolve(Some("decisions/a.md"), "../notes/b.md#top"),
            "/scroll/sc%2Db#top"
        );
        assert_eq!(links.resolve(None, "../../notes/b.md"), "/scroll/sc%2Db");
        assert_eq!(
            links.resolve(None, "../items/TICKET-12-fix-login.md"),
            "/bead/fx%2D1"
        );
        assert_eq!(links.resolve(None, "fx-1.md"), "/bead/fx%2D1");
        assert_eq!(links.resolve(None, "beads://store/fx-1"), "/bead/fx%2D1");
        assert_eq!(links.resolve(None, "work:fx-1"), "/bead/fx%2D1");
        assert_eq!(links.resolve(None, "missing.md"), "#");
        assert_eq!(links.resolve(None, "ftp://example.org/x"), "#");
    }

    #[test]
    fn headings_get_distinct_anchors() {
        let view = View::default();
        let html = markdown(
            "## Acceptance\n\nOne\n\n## Acceptance\n\nTwo",
            None,
            &Links::new(&view),
        );
        assert!(html.contains("id=\"acceptance\""));
        assert!(html.contains("id=\"acceptance-1\""));
    }
}
