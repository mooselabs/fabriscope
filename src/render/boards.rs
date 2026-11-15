// Owned by WP5 — Runs. Frozen signatures: `boards(&View) -> String`,
// `board(&View, &str) -> Option<String>`.
//! `/boards` and `/boards/{name}`: the run feed's boards. A board's body is
//! any JSON, rendered generically: objects as nested facts, arrays of flat
//! objects as tables, lists of scalars as lists, with the raw JSON under a
//! collapsed section. There is no board-specific code.

use super::fmt::{count, when};
use super::layout::{not_found, page};
use super::{escape, heading, url};
use crate::model::View;
use crate::runs::cache::Fetch;
use serde_json::{Map, Value};
use std::fmt::Write;

/// Rows shown of one array.
const ROWS: usize = 200;

/// How deep nested values are drawn before falling back to JSON.
const DEPTH: usize = 6;

/// `/boards`.
pub(crate) fn boards(view: &View) -> String {
    let Some(index) = view.run_index() else {
        return not_found(view);
    };
    let mut body = heading(
        "Factory",
        "Boards",
        "Read models the factory publishes beside its runs, shown as they come.",
    );
    if index.feed.boards.is_empty() {
        body.push_str("<p class=\"empty\">The run feed publishes no boards.</p>");
    } else {
        body.push_str("<ul class=\"board-list\">");
        for name in index.feed.boards.keys() {
            let detail = match view.run_cache.get_board(name) {
                Fetch::Ready(board) => {
                    let title = board
                        .title
                        .as_deref()
                        .map(|t| format!("<strong>{}</strong> ", escape(t)))
                        .unwrap_or_default();
                    let at = if board.generated_at.is_empty() {
                        String::new()
                    } else {
                        format!("generated {}", when(&board.generated_at))
                    };
                    format!("{title}<small class=\"muted\">{at}</small>")
                }
                Fetch::Pending => "<small class=\"muted\">not read yet</small>".into(),
                Fetch::Failed(error) => {
                    format!("<small class=\"tone-bad\">{}</small>", escape(&error))
                }
            };
            write!(
                body,
                "<li><a href=\"/boards/{}\"><code>{}</code></a> {detail}</li>",
                url(name),
                escape(name)
            )
            .unwrap();
        }
        body.push_str("</ul>");
    }
    page(view, "Boards", "/boards", &body)
}

/// `/boards/{name}`; `None` for a board the feed does not name.
pub(crate) fn board(view: &View, name: &str) -> Option<String> {
    let index = view.run_index()?;
    index.feed.boards.get(name)?;
    let mut body = String::new();
    match view.run_cache.get_board(name) {
        Fetch::Pending => {
            body.push_str(&heading("Board", name, ""));
            body.push_str("<p class=\"fetching\" role=\"status\">Fetching board… this page fills in when it arrives.</p>");
        }
        Fetch::Failed(error) => {
            body.push_str(&heading("Board", name, ""));
            write!(
                body,
                "<div class=\"banner bad\" role=\"alert\"><strong>This board could not be read.</strong> {}</div>",
                escape(&error)
            )
            .unwrap();
        }
        Fetch::Ready(board) => {
            body.push_str(&heading(
                "Board",
                board.title.as_deref().unwrap_or(name),
                "",
            ));
            body.push_str("<dl class=\"facts\">");
            write!(body, "<dt>Name</dt><dd><code>{}</code></dd>", escape(name)).unwrap();
            if !board.generated_at.is_empty() {
                write!(
                    body,
                    "<dt>Generated</dt><dd>{}</dd>",
                    when(&board.generated_at)
                )
                .unwrap();
            }
            if let Some(schema) = &board.source_schema {
                write!(
                    body,
                    "<dt>Format</dt><dd><code>{}</code></dd>",
                    escape(schema)
                )
                .unwrap();
            }
            body.push_str("</dl><section class=\"board-body\">");
            body.push_str(&value(&board.body, 0));
            write!(
                body,
                "</section><details class=\"raw\" id=\"board-raw\"><summary>Raw JSON</summary><pre class=\"json\">{}</pre></details>",
                escape(&serde_json::to_string_pretty(&board.body).unwrap_or_default())
            )
            .unwrap();
        }
    }
    body.push_str("<p><a href=\"/boards\">← All boards</a></p>");
    Some(page(view, name, "/boards", &body))
}

/// A scalar as text: numbers as written (whole ones grouped), booleans as
/// yes/no, null as a dash.
fn scalar(v: &Value) -> String {
    match v {
        Value::Null => "<span class=\"muted\">—</span>".into(),
        Value::Bool(true) => "yes".into(),
        Value::Bool(false) => "no".into(),
        Value::Number(n) => match n.as_u64() {
            Some(whole) => count(whole),
            None => escape(&n.to_string()),
        },
        Value::String(s) => escape(s),
        other => format!(
            "<pre class=\"json\">{}</pre>",
            escape(&serde_json::to_string_pretty(other).unwrap_or_default())
        ),
    }
}

fn is_scalar(v: &Value) -> bool {
    !matches!(v, Value::Object(_) | Value::Array(_))
}

/// Whether an array is a table: every element an object of scalars.
fn flat_rows(items: &[Value]) -> Option<Vec<&Map<String, Value>>> {
    items
        .iter()
        .map(|item| item.as_object().filter(|o| o.values().all(is_scalar)))
        .collect()
}

fn value(v: &Value, depth: usize) -> String {
    if depth > DEPTH {
        return format!(
            "<pre class=\"json\">{}</pre>",
            escape(&serde_json::to_string_pretty(v).unwrap_or_default())
        );
    }
    match v {
        Value::Object(map) if map.is_empty() => "<span class=\"muted\">(empty)</span>".into(),
        Value::Object(map) => {
            let mut out = "<dl class=\"facts\">".to_string();
            for (key, child) in map {
                write!(
                    out,
                    "<dt>{}</dt><dd>{}</dd>",
                    escape(key),
                    value(child, depth + 1)
                )
                .unwrap();
            }
            out.push_str("</dl>");
            out
        }
        Value::Array(items) if items.is_empty() => "<span class=\"muted\">(none)</span>".into(),
        Value::Array(items) => {
            let more = if items.len() > ROWS {
                format!(
                    "<p class=\"muted\">The first {ROWS} of {} rows.</p>",
                    count(items.len() as u64)
                )
            } else {
                String::new()
            };
            let shown = &items[..items.len().min(ROWS)];
            if let Some(rows) = flat_rows(shown) {
                let mut columns: Vec<&str> = Vec::new();
                for row in &rows {
                    for key in row.keys() {
                        if !columns.contains(&key.as_str()) {
                            columns.push(key);
                        }
                    }
                }
                let mut out =
                    "<div class=\"table-wrap\"><table class=\"data\"><thead><tr>".to_string();
                for column in &columns {
                    write!(out, "<th>{}</th>", escape(column)).unwrap();
                }
                out.push_str("</tr></thead><tbody>");
                for row in rows {
                    out.push_str("<tr>");
                    for column in &columns {
                        let cell = row.get(*column).map(scalar).unwrap_or_default();
                        let class = if row.get(*column).is_some_and(Value::is_number) {
                            " class=\"num\""
                        } else {
                            ""
                        };
                        write!(out, "<td{class}>{cell}</td>").unwrap();
                    }
                    out.push_str("</tr>");
                }
                out.push_str("</tbody></table></div>");
                return format!("{out}{more}");
            }
            let tag = if shown.iter().all(is_scalar) {
                "ul"
            } else {
                "ol"
            };
            let mut out = format!("<{tag} class=\"values\">");
            for item in shown {
                write!(out, "<li>{}</li>", value(item, depth + 1)).unwrap();
            }
            write!(out, "</{tag}>{more}").unwrap();
            out
        }
        scalar_value => scalar(scalar_value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runs::Board;
    use serde_json::json;

    #[test]
    fn boards_render_any_json_generically() {
        let view = crate::testing::work_view();
        let list = boards(&view);
        assert!(list.contains("href=\"/boards/stop%2Dratio\""));
        assert!(list.contains("not read yet"));
        assert!(board(&view, "nope").is_none());
        assert!(
            board(&view, "stop-ratio")
                .unwrap()
                .contains("Fetching board…")
        );
        let stop: Board = serde_json::from_str(include_str!(
            "../../tests/fixtures/runfeed/v1/boards/stop-ratio.json"
        ))
        .unwrap();
        view.run_cache.put_board("stop-ratio", Ok(stop));
        let page = board(&view, "stop-ratio").unwrap();
        assert!(page.contains("Stops per merge"));
        assert!(page.contains("<code>acme.stop_ratio.v2</code>"));
        assert!(page.contains("<dt>stops_per_merge</dt><dd>0.25</dd>"));
        assert!(page.contains("<dt>contract</dt><dd>2</dd>"));
        assert!(page.contains("id=\"board-raw\""));
        let rows: Vec<Value> = (0..250)
            .map(|i| json!({"name": format!("<r{i}>"), "n": i}))
            .collect();
        let table = value(&json!({"rows": rows, "tags": ["a", "b"], "none": null}), 0);
        assert!(table.contains("<th>n</th><th>name</th>"));
        assert!(table.contains("&lt;r199&gt;") && !table.contains("&lt;r200&gt;"));
        assert!(table.contains("The first 200 of 250 rows."));
        assert!(table.contains("<ul class=\"values\"><li>a</li>"));
        assert!(!table.contains("<r1>"));
    }
}
