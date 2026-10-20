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

- **Overview** — open, ready, in-progress and waiting counts; open beads over
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

The Scrolls HTTP reader was added after Fabriscope v0.1.0. That tagged binary
still uses the prior render reader; build from source until a later tagged
release includes this integration. Scrolls must advertise the `graph`
capability; a version number alone does not establish support.

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
`http`, `https` and `mailto` destinations. Every response carries a strict
Content Security Policy.

## Health

`GET /healthz` returns JSON with the running version, any newer version
installed by the updater, and, for each store shown, when it was last read,
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
token, a live refresh, and that a failed read keeps the last good data. It
also runs copies of the binary against a fake GitHub release. These check
`fabriscope update`, the background update, the opt-out, and that a bad
checksum or a wrong program is never installed.

`shellcheck scripts/install.sh` lints the installer.

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
