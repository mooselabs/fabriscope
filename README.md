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

## Install

On Linux (x86_64, arm64) or macOS (Intel, Apple Silicon):

```sh
curl -fsSL https://raw.githubusercontent.com/mooselabs/fabriscope/main/scripts/install.sh | bash
```

This puts `fabriscope` in `~/.local/bin`, after checking the download
against the release's `SHA256SUMS`. It installs nothing if the check fails.
Settings, as environment variables for `bash`:

| Variable | Meaning |
| --- | --- |
| `FABRISCOPE_VERSION` | A release tag to install, such as `v0.1.0`. Default: the latest. |
| `FABRISCOPE_INSTALL_DIR` | Where to put the program. Default: `~/.local/bin`. |
| `FABRISCOPE_RELEASES_URL` | Where releases are served. Default: this repository's GitHub releases. |

For example, `curl -fsSL …/install.sh | FABRISCOPE_VERSION=v0.1.0 bash`.
Each release's archives carry GitHub build attestations, which
`gh attestation verify fabriscope-<target>.tar.gz -R mooselabs/fabriscope`
checks.

From source, with Rust 1.85+: `cargo install --locked --git https://github.com/mooselabs/fabriscope`
(or `--path .` in a checkout). A source build does not update itself.

## Quick start

You need the tools for the stores you want to see: `bd` for Beads,
`scrolls` (and its Dolt binary) for Scrolls.

```sh
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
| `--profile FILE` | A `fabriscope.profile.v1` file saying how the store uses its metadata: row classes, kinds, states, groups, people, panels, history events, receipts and links. See [docs/profile.md](docs/profile.md). |
| `--runs URL\|DIR` | A run feed (`runfeed.v1`, see [docs/runfeed-v1.md](docs/runfeed-v1.md)) to show beside the work: a directory, or a URL. A URL must be plain `http://`; `https://` is refused, so serve the feed on loopback or a private network. Fabriscope polls `feed.json` every 30 s. |
| `--runs-token-file FILE` | A bearer token for a `--runs` URL, first non-empty line, re-read on every request. |
| `--beads-token-file FILE` | A bearer token for that `bd serve`, first non-empty line, re-read on every request. |
| `--scrolls STORE` | A Scrolls store directory, usually `…/.scrolls`. |
| `--bind ADDR` | Address to listen on. Default `127.0.0.1:8787`. |
| `--title TEXT` | A name shown in the header and page titles. |
| `--bd PATH` | The `bd` program. Default: `bd` on `PATH`. |
| `--scrolls-bin PATH` | The `scrolls` program. Default: `scrolls` on `PATH`. |
| `--scrolls-dolt PATH` | The Dolt binary Scrolls should use, passed as `--dolt`. |
| `--no-update` | Do not check for a new release while running. |
| `--version` | Print the version and the release target, then exit. |

`fabriscope update` installs a newer release now, if there is one, and exits.
`fabriscope profile check FILE` validates a profile and prints it resolved.

A profile and a run feed are both optional. Without them Fabriscope shows
what Beads stores, and the tabs that need them stay hidden; with them it
adds the pages under **Ledger views** below. The environment variable
`FABRISCOPE_RUNS_POLL_MS` shortens the feed poll for the tests; it is not a
setting to use in production.

## Updates

A Fabriscope installed from a release keeps itself current, the way Claude
Code does. Shortly after it starts, and then once a day, it asks GitHub for
the latest release. If that release is newer, Fabriscope downloads it for
the same platform, checks it against `SHA256SUMS` and checks that it runs,
and then replaces its own program file in one step. The server keeps
running the version it started with. It prints
`fabriscope: updated to vX.Y.Z; restart to use it`, and `/healthz` shows
`"update": {"installed": "X.Y.Z"}`. The next start runs the new version.

- It never installs an older version or a pre-release.
- A failed check prints one line and tries again the next day. It never
  stops the server.
- If it cannot write next to its own program (for example, an install in a
  folder owned by root), it says so once and stops checking for that run.
- To turn it off, pass `--no-update` or set `FABRISCOPE_NO_UPDATE=1`.
- A source build (`cargo install`) never updates itself.
  `FABRISCOPE_TARGET=<target>` at run time makes one follow releases.

## What you see

- **Overview** — open, ready, in-progress and waiting counts; where each
  bead in progress is up to (see **Progress** below); open beads over
  the last 28 complete UTC days; closed and added per period; what-if
  forecasts; the next ready beads grouped by parent; Scrolls kinds and the
  newest scrolls.
- **Beads** — every bead in priority order, filtered by state, readiness,
  type, priority or text. A bead's state is its status exactly as Beads
  stores it (`open`, `in_progress`, `closed`, or any custom status), and the
  State and Type filters list the values the store holds, with counts.
  Readiness is a separate filter, because it is `bd ready`'s answer about
  dependencies, not a status. Each bead page shows its description, design,
  acceptance criteria and notes as Markdown, what it depends on, what it
  blocks, related beads, and scrolls that reference it.
- **Progress** — on each bead page, the stages its provenance log shows it
  has reached, as a line of steps: **Created**, **Claimed** (`claim`),
  **Started** (`cut`), **Committed** (`commit`), **Landed** (`land`) and
  **Closed**, with **Used** (`used`) beside them. Each step shows when it
  happened and what it points at: the claiming actor, the run, the commit
  SHA and branch, the pull request. The next step is marked with how long it
  has been due.
  - Each `claim` starts an attempt; the line shows the latest. A `suspend`
    marks the step the attempt stopped before, with the cause the event
    records, and holds it there until a `resume`. After a `resume` or a
    `handoff` the bead waits for a new claim, and the stopped attempt moves
    to **Earlier attempts**.
  - A step with no event although a later step has one shows as not
    recorded, rather than guessed.
  - The full log is under **Provenance log**. Beads in progress also show a
    compact line of dots in lists and on the overview.
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

### Ledger views

With a profile ([docs/profile.md](docs/profile.md)) and a run feed
([docs/runfeed-v1.md](docs/runfeed-v1.md)), every page links to the
others: a bead to its group, lane, people, runs, attempts and receipts; a
run back to its bead.

- **Groups** (`/by/{group}`) — beads by a profile group, such as a
  milestone, and each value's lanes (`/by/{group}/{value}`).
- **People** and **Activity** — who holds, created or claimed what, with
  factory seats classed apart from people; one timeline of history events,
  comments, provenance and run starts and ends, filtered by the same facets
  as the lists and a date range.
- **Flow** — items created and claimed per period, cycle time, the age of
  open work and decisions waiting for an answer, each split by a group,
  type, kind, priority, claimant or creator (`?by=`).
- **Held** — work a profile state holds back, why it stopped and since when,
  with its run and attempt; stop history over a date range.
- **Costs** — spend by attempt, run, group and model from provenance `used`
  events and the run feed. A missing amount shows as unknown, never as $0.
- **Receipts** — receipt rows by kind, joined to the bead they are about.
- **Runs** — every run in the feed. A run page shows its attempts, steps,
  facts and costs; its documents (logs and transcripts) open as plain text,
  a page at a time.
- **Boards** — whatever JSON the feed publishes as a board, shown as facts,
  tables and lists, with the raw JSON beside it.
- **Search** — finds beads, people, group values and runs; an exact
  reference opens its bead.

Links in documents connect the two stores. A relative link resolves to the
scroll at that path (or the one scroll whose path ends with it); a
`beads://STORE/ID` or `NAME:ID` reference, or a file named after a bead's id
or external reference, opens that bead.

## Appearance

Use **Theme** in the header to choose **System**, **Light**, or **Dark**.
System is the default and follows your device's appearance, including changes
while a page is open. An explicit choice overrides the device setting.
Fabriscope remembers the choice in this browser for the same server address,
across page navigation and restarts, and keeps open tabs in sync. If browser
storage is disabled, the choice still works for the current page. With
JavaScript disabled, pages follow the device setting.

## How it reads

The Scrolls HTTP reader is included from Fabriscope v0.2.0. Scrolls must
advertise the `graph` capability; a version number alone does not establish support.

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
- **Provenance:** with `bd provenance log --json -- ID`, one bead at a time,
  because `bd serve` does not serve provenance and `bd` has no bulk read. So
  it needs a local `bd`: with `--beads-url` alone there is no **Progress**.
  - Reads run in the background, at most four at once. A page never waits
    for one: it shows the last read (or "Reading the provenance log…") and
    updates itself when a read changes what it shows.
  - Fabriscope reads a bead's log when its page is opened, and reads every
    bead in progress ahead of time, after each Beads read. A log is read
    again when the bead's `updated_at` changes, or after two minutes (thirty
    for a closed bead, one after a failed read).
  - Against a Dolt server across a network each `bd` call can take seconds
    (about ten over Tailscale in one measurement), which is why none of
    this is on the request path.
- **Scrolls:** through `GET /v0/graph` on a loopback `scrolls serve`. The
  complete snapshot includes document bodies, fields, paths, aliases and all
  relationships, including Beads backlinks. Normal viewing creates no render
  directory and does not scan the store filesystem.
  - `--scrolls STORE` starts `scrolls --store STORE --json serve --bind
    127.0.0.1:0`, retaining `--scrolls-bin` and `--scrolls-dolt`. Fabriscope
    drains its output and terminates the private server on startup failure or
    shutdown (on Linux, also when Fabriscope is killed).
  - `--scrolls-url http://127.0.0.1:PORT` attaches to an existing server and
    leaves it running on exit. It is mutually exclusive with `--scrolls`.
    URLs must use HTTP and loopback, without credentials, paths or queries.
    `localhost` resolves locally to IPv4 loopback; use `[::1]` for IPv6.
    Configure shared connections and proposal selection on that server:

    ```sh
    scrolls --connection /absolute/connection.json --proposal proposal/NAME serve --bind 127.0.0.1:8080 --json
    fabriscope --scrolls-url http://127.0.0.1:8080
    ```

  - Discovery must advertise API `v0`, envelope version 2, read-only operation
    and `graph`. Older Scrolls builds fail with an upgrade instruction; there
    is no render fallback. Build/install a Scrolls revision that includes
    `/v0/graph`, then restart the server. No storage migration is needed.
  - Every 30 seconds after the previous request completes, Fabriscope sends
    `If-None-Match` with its last validated ETag. Only one refresh is in flight.
    An unchanged response updates the successful-check time without reloading
    pages. Conditional requests still read the backend.
  - Startup/discovery of a private server is bounded to 60 seconds. HTTP
    requests are bounded to 120 seconds and graph responses to 256 MiB.
    Malformed or incomplete snapshots never replace the current view. A
    changed store identity or branch requires restarting Fabriscope to select it.

A failed refresh keeps the last good documents and relationships on screen with
an error warning. `/healthz` returns 503 until recovery and reports the source
URL, store ID, branch, revision, schema revision and `last_successful_check`.
An unreadable store at startup stops Fabriscope. Attached servers remain owned
by their caller. Pages are served from memory and do not wait for backend reads.


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
`http`, `https` and `mailto` destinations. Profile panels and run feed
links become external links only for `https://`. Run documents, including
transcripts and logs, are shown as escaped plain text, never as HTML or
Markdown, and feed files are read only inside the `--runs` directory (or
under the feed URL, each path segment percent-encoded). Every response
carries a strict Content Security Policy.

## Health

`GET /healthz` returns JSON with the running version, any newer version
installed by the updater, and, for each store shown, when it was last read,
its commit or Dolt revision, record counts and the last error. With a local
`bd`, `provenance` counts the logs read, the reads under way and the reads
that failed; `history` does the same for comment threads. With `--runs`,
`runs` gives the feed's revision, run count, gaps and last error, and how
many run docs and document bytes the cache holds. A failed feed read answers
503 until the next good poll.

## Development

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

`tests/live.rs` runs the real binary against fake `bd` and `scrolls` programs
and a fake Beads API. It checks the pages, reading through a private
`bd serve`, the fallback to `bd export`, attaching to a running server with a
token, a live refresh, and that a failed read keeps the last good data. It
also runs copies of the binary against a fake GitHub release. These check
`fabriscope update`, the background update, the opt-out, and that a bad
checksum or a wrong program is never installed.

The other `tests/live_*.rs` files run the binary against the work fixture
(`tests/fixtures/work/`), its profile and the golden run feed
(`tests/fixtures/runfeed/v1/`): bead pages, lists, flow, history, people,
held work, costs, receipts and runs. `tests/live_crawl.rs` follows every
internal link from the overview, with and without a profile and feed, and
fails on any 404, 500, unfilled template or inline style.
`python3 tests/runfeed_schema.py` checks the golden feed against
[docs/runfeed-v1.md](docs/runfeed-v1.md).

`shellcheck scripts/install.sh` lints the installer.

The theme browser checks run against the real binary with disposable Beads and
Scrolls fixtures. They cover system preferences, explicit overrides, persistence,
other tabs, live refresh, keyboard access, contrast, mobile widths and disabled
storage/JavaScript. CI runs them; locally, after `cargo build --locked`:

```sh
python3 -m pip install playwright==1.64.0
playwright install chromium
python3 tests/theme.py --binary target/debug/fabriscope
```

`tests/theme.py` also runs every `tests/browser_*.py` check: the ledger
pages in light and dark, at desktop width and at 320 px.

## Releasing

1. Set the new `version` in `Cargo.toml`, run `cargo check` so
   `Cargo.lock` follows, and merge that to `main`.
2. Tag the merged commit and push the tag:
   `git tag v0.2.0 && git push origin v0.2.0`.

The [release workflow](.github/workflows/release.yml) checks that the tag
matches `Cargo.toml` and runs the full checks. It builds static Linux (musl)
and macOS binaries, then publishes the release with
`fabriscope-<target>.tar.gz` archives, `SHA256SUMS` and build attestations.
Installed copies pick it up within a day. A pull request that changes the
release path runs the same checks and builds without publishing.

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

### Qualify the real Scrolls integration

The CI integration tests use disposable HTTP/process fixtures. For a composed
readback with both real binaries and local/shared Dolt, run:

```sh
python3 scripts/verify-scrolls.py --fabriscope target/debug/fabriscope --scrolls /absolute/scrolls --dolt /absolute/dolt --out /fresh/qualification-directory
```

This creates and cleans up its own servers, checks all graph records and edges
against a render oracle, exercises Beads backlinks and a live refresh, and
reports startup, conditional read and refresh timing plus Linux RSS for 8,000
documents. The render oracle is test-only; the viewer's temporary directory
must remain empty. No existing store or installed service is changed.
