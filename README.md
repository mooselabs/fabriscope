# Fabriscope

**View any [Scrolls](https://github.com/mooselabs/scrolls) or
[Beads](https://github.com/steveyegge/beads) store in a web browser.**

Beads tracks the work; Scrolls records what you know. Fabriscope is a small,
read-only web server that shows either store, or both together: what is ready,
what is waiting, how fast work is closing, and the knowledge graph behind it.
Pages update live while you watch.

*Fabriscope* (FAB-rih-scope) is from Latin *fabrica*, "workshop": the root of
both *fabricate* (the work Beads track) and *fabric* (the woven text of
Scrolls).

## Quick start

You need Rust 1.85+ and the tools for the stores you want to see: `bd` for
Beads, `scrolls` (and its Dolt binary) for Scrolls.

```sh
cargo install --locked --path .

# A Beads store: any directory where `bd` works. Fabriscope starts a
# private `bd serve` there and stops it when it exits.
fabriscope --beads ~/code/my-project

# A `bd serve` that is already running, here or elsewhere.
fabriscope --beads-url http://127.0.0.1:4100

# A Scrolls store.
fabriscope --scrolls ~/code/my-project/.scrolls --scrolls-dolt ~/scrolls/.tools/dolt

# Both, with a name in the header.
fabriscope --beads ~/code/my-project --scrolls ~/code/my-project/.scrolls --title "My project"
```

Open <http://127.0.0.1:8787>. Stop with Ctrl-C.

| Option | Meaning |
| --- | --- |
| `--beads DIR` | A directory where `bd` finds the Beads store. Fabriscope starts a private `bd serve` there. |
| `--beads-url URL` | A `bd serve` that is already running. |
| `--parent-key PATH` | A dot-separated metadata path that names a bead's parent by id or external reference, for stores that keep membership in metadata instead of parent-child links. |
| `--beads-token-file FILE` | A bearer token for that `bd serve`, first non-empty line, re-read on every request. |
| `--scrolls STORE` | A Scrolls store directory, usually `…/.scrolls`. |
| `--bind ADDR` | Address to listen on. Default `127.0.0.1:8787`. |
| `--title TEXT` | A name shown in the header and page titles. |
| `--bd PATH` | The `bd` program. Default: `bd` on `PATH`. |
| `--scrolls-bin PATH` | The `scrolls` program. Default: `scrolls` on `PATH`. |
| `--scrolls-dolt PATH` | The Dolt binary Scrolls should use, passed as `--dolt`. |

## What you see

- **Overview** — open, ready, in-progress and waiting counts; open beads over
  the last 28 complete UTC days; closed and added per period; what-if
  forecasts; the next ready beads grouped by parent; Scrolls kinds and the
  newest scrolls.
- **Beads** — every bead in priority order, filtered by state, type, priority
  or text. Each bead page shows its description, design, acceptance criteria
  and notes as Markdown, what it depends on, what it blocks, related beads,
  and scrolls that reference it.
- **Epics** — each epic with progress over its children: beads with a
  `parent-child` dependency on it, or, with `--parent-key`, beads whose
  metadata names it. Some stores avoid `parent-child` because Beads gives it
  scheduling meaning (a child waits on its parent) and record membership in
  metadata instead; for example `--parent-key tracker.fields.parent_epic`
  reads `{"tracker": {"fields": {"parent_epic": "EPIC-12"}}}`. A
  `parent-child` dependency always wins over the metadata.
- **Scrolls** — every record, filtered by kind, status or text. Each scroll
  page shows its body, the links from and to it grouped by relation, its
  aliases and its fields.

Links in documents connect the two stores. A relative link resolves to the
scroll at that path (or the one scroll whose path ends with it); a
`beads://STORE/ID` or `NAME:ID` reference, or a file named after a bead's id
or external reference, opens that bead.

## How it reads

Fabriscope only runs the stores' own read commands. It never writes,
claims, or edits anything.

- **Beads:** through the HTTP API of `bd serve` — `GET /v0/beads/issues`
  for every issue (closed ones included) with its dependencies, and
  `GET /v0/beads/ready` for which open issues are ready. Beads decides
  readiness, not Fabriscope.
  - With `--beads DIR`, Fabriscope starts `bd serve` on a free loopback port
    in that directory and stops it on exit (on Linux, even if Fabriscope is
    killed). `bd serve` needs the store to sit in a Git repository; when it
    refuses, Fabriscope says why and reads with `bd export` and
    `bd ready --json` instead.
  - Change detection: with a local directory, `bd vc status --json` reports
    the store's commit every two seconds and a new commit triggers a fresh
    read (plus a safety re-read every five minutes). With only
    `--beads-url`, Fabriscope watches the server's stats (and its events
    journal, when enabled) every two seconds and also re-reads every minute,
    because neither catches every edit: stats are counts, and the journal
    records only writes made through that server.
  - `bd serve` always offers write operations (claim, close, delete), and
    has no authentication on loopback unless you give it a token file.
    Fabriscope only ever sends reads.
- **Scrolls:** `scrolls render` into a private temporary directory, reading
  its `scrolls/graph.json` (every record and edge) and removing the directory
  afterwards. The store's files are checked every two seconds; a change
  triggers a fresh read, and a read also happens every ten minutes.

A failed read keeps the last good one on screen with a warning, and
`/healthz` returns 503 until a read succeeds. An unreadable store at startup
stops the server with the tool's own error message.

Reading takes as long as the tools take. On a store of 4,256 beads behind a
remote Dolt server, `bd serve` answered the full list in about 4 seconds and
the ready list in 2 (`bd export` took about 5.5); rendering 8,000 scrolls took about
16 seconds and writes that many Markdown files to the temporary directory.
Pages are served from memory and do not wait for reads.

### History and forecasts

History comes from each bead's `created_at` and `closed_at`. A bead that is
open now counts as open since it was created, even if it was closed and
reopened in between, and deleted beads are not counted. The forecasts apply
the observed net change in open beads to today's count. They are conditional
arithmetic, not confidence intervals or delivery dates; counts measure beads,
not equal-sized effort.

## Security

The server binds to loopback by default and has no authentication. For remote
access keep that binding and forward it, for example
`ssh -L 8787:127.0.0.1:8787 host`. Only bind wider on a network you trust.

It serves rendered pages and its own CSS and JavaScript, never files from
disk. Markdown is rendered without raw HTML; images become links so a
document cannot load third-party content; links leave the server only for
`http`, `https` and `mailto` destinations. Every response carries a strict
Content Security Policy.

## Health

`GET /healthz` returns JSON with, for each store shown, when it was last read,
its commit or Dolt revision, record counts and the last error.

## Development

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

`tests/live.rs` runs the real binary against fake `bd` and `scrolls` programs
and a fake Beads API. It checks the pages, reading through a private
`bd serve`, the fallback to `bd export`, attaching to a running server with a
token, a live refresh, and that a failed read keeps the last good data.

## Licence

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.
