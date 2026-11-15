# Run feed v1 (`runfeed.v1`)

A run feed is a directory of static JSON files that describes the runs a
factory made: an automated system that takes work items (subjects) and runs
attempts at them, step by step, until each run passes, fails, stops for a
person, or is superseded. A producer writes the feed beside the factory;
a reader such as Fabriscope shows it. Nothing else is shared between them.

The feed is a **view**. It carries no authority: no gate, merge, dispatch,
billing or any other decision may branch on its contents. The systems that
make those decisions read their own records; the feed only reports them.

Machine-readable schema: [`schema/runfeed-v1.json`](../schema/runfeed-v1.json)
(JSON Schema draft 2020-12, one `$defs` entry per file and object kind).
Golden example: [`tests/fixtures/runfeed/v1/`](../tests/fixtures/runfeed/v1/),
checked by `python3 tests/runfeed_schema.py` (standard library only; pass
`--feed DIR --allow-orphans` to check a live feed).

The key words MUST, MUST NOT, SHOULD and MAY are used as in RFC 2119.

## Layout

```
feed.json                  entry point; written last in every publish
index.json                 open runs and recent ones
index/YYYY-MM.json         older index shards, same shape as index.json
subjects/<key>.json        every run for one subject
runs/<run>.json            one run doc per run
docs/<run>/<doc>/<page>    document pages (transcripts, logs, diagnostics)
boards/<name>.json         producer read models wrapped for display
changes.jsonl              one line per run change, oldest first
```

Only `feed.json` has a fixed name. Every other file is found through an href
(below), except subject files, which a reader finds by name so that an item
page can look up its runs in one request.

## Conventions

**Hrefs.** Every href that names a feed file is a path relative to the feed
root: file names separated by `/`, each made of `A-Z a-z 0-9 - _ . ~ %`, none
empty and none starting with `.`. An href therefore cannot be absolute and
cannot escape the root, and a reader MUST refuse one that does. An href is a
path of file names, not a URL: a reader that fetches the feed over HTTP
percent-encodes each segment, so a file named `a%2Fb.json` is requested as
`a%252Fb.json`. Link hrefs (the `link` object) are the one exception: they are
absolute external URLs, and a reader makes only `https://` ones clickable.

**File names from identifiers.** A subject ref or run id becomes a file name
by percent-encoding its UTF-8 bytes: every byte outside `A-Z a-z 0-9 - _ . ~`
becomes `%XX` (upper-case hex), and a leading `.` becomes `%2E`. So
`WORK-0001` stays `WORK-0001` and `a/b c` becomes `a%2Fb%20c`. Producers on a
case-insensitive file system MUST NOT publish two subjects whose keys differ
only in case.

**Timestamps** are RFC 3339 in UTC with a `Z` suffix, optionally with
fractional seconds: `2026-11-15T12:00:00Z`.

**Unknown fields and values.** A reader MUST ignore fields it does not know,
and MUST render an enum value it does not know neutrally (as plain text, with
no state colour) rather than reject the file. The schema is strict
(`additionalProperties: false`, closed enums) because it checks producers;
readers are lenient. Producer-specific data belongs in the free maps (`refs`,
`group`, `roles`, `by_model`, `by_role`) and in facts, never in new top-level
fields.

**Facts** are the open-ended part of the format: `{key, value, type?, label?,
tone?, ref?}`. A key is opaque to readers; producers MAY namespace keys with
dots (`acme.gate_seconds`). A reader shows `label`, else a label it was
configured with for that key, else the raw key. `type` tells a reader how to
format the value (`seconds` and `hours` are durations, `usd` dollars,
`tokens` and `count` whole numbers, `percent` a percentage where `42.5`
means 42.5%, `sha` and `tree` hashes, `timestamp` an RFC 3339 instant); a
value that does not fit its type is shown as text; `tone` (`good`, `warn`, `bad`) how to colour it; `ref`
makes it a link to another item, run, attempt, commit or tree.

## Files

### `feed.json`

```json
{
  "schema": "runfeed.v1",
  "producer": {"name": "demo-exporter", "version": "1.0.0", "commit": "…"},
  "factory": "demo",
  "revision": 42,
  "generated_at": "2026-11-15T12:00:00Z",
  "index": "index.json",
  "shards": ["index/2026-10.json"],
  "source_health": {"at": "2026-11-15T11:59:30Z", "cursor": "events:18342", "gaps": []},
  "boards": {"stop-ratio": "boards/stop-ratio.json"}
}
```

- `revision` is the feed's change cursor: a non-negative integer that grows
  with every publish. A reader polls `feed.json` and rereads only when the
  revision has moved. A producer MAY also serve an HTTP `ETag`; readers MUST
  NOT require one.
- `shards` is optional; absent means there are no shards.
- `source_health` says how fresh the producer's inputs were: when it last
  read them (`at`), an opaque position in them (`cursor`), and `gaps`, the
  inputs it could not read in this publish. An empty `gaps` is a healthy feed.
- `boards` maps each board name (`^[a-z][a-z0-9_.-]*$`) to its href.

### `index.json` and shards

```json
{"schema": "runfeed.index.v1", "revision": 42, "runs": [RunSummary, …]}
```

`index.json` lists every open run and the recently finished ones (the
producer chooses the window; 14 days is a good default). Older runs move to
monthly shards, `index/YYYY-MM.json` by the month the run started, which have
the same shape and are listed in `feed.json` `shards`. A run is in exactly one
listing. Readers MUST NOT assume any order within `runs`.

A shard's `revision` is the revision at which it was last written. The
`index.json` revision is at least the `feed.json` revision; it can be greater
while a publish is in flight, and a reader accepts that.

**RunSummary** mirrors the run doc it names, so an overview needs no run docs:

| Field | Type | |
| --- | --- | --- |
| `run` | string | Run id. Unique within the feed. |
| `subject` | Subject | `{ref?, id?, title?}`, at least one of `ref` and `id`. |
| `group` | map of string | Grouping dimensions with free keys, such as `rig` or `repository`. |
| `state` | run state | See **States**. |
| `started_at`, `ended_at` | timestamp, optional | |
| `updated_at` | timestamp | When the run doc last changed. |
| `digest` | sha256 hex | Equal to the run doc's `digest`. |
| `href` | href | The run doc. |
| `cost_usd` | number, optional | Equal to the run doc's `costs.usd`. |
| `attempts` | integer | Number of attempts in the run doc. |
| `stop_code` | string, optional | Equal to the run doc's `stop.code`. |

`run`, `subject`, `group`, `state`, the three timestamps and `digest` MUST
equal the run doc's values.

### `subjects/<key>.json`

```json
{
  "schema": "runfeed.subject.v1",
  "subject": {"ref": "WORK-0002", "title": "Fix rounding in invoice totals"},
  "runs": [{"run": "run-0002", "attempt": "att-0002-2", "state": "passed",
            "started_at": "2026-11-14T08:00:00Z", "href": "runs/run-0002.json"}]
}
```

`<key>` is the subject `ref`, encoded as under **File names from
identifiers**. There is one subject file for every ref that any listed run
has, and it lists every listed run for that ref, in any order. Runs whose
subject has no `ref` have no subject file. `subject` carries the ref and the
most recent title. Each entry's `state`, `started_at` and `href` match the
run; `attempt` is the run's current attempt.

### `runs/<run>.json` — the run doc

| Field | Type | |
| --- | --- | --- |
| `schema` | `"runfeed.run.v1"` | |
| `run` | string | Run id. |
| `refs` | map of string | Identifiers the run worked from and produced. Conventional keys: `attempt` (the current attempt's id), `contract`, `branch`, `commit`, `tree`; others are free. |
| `subject`, `group`, `state` | | As in RunSummary. |
| `current_step` | string, optional | Id of the step in progress. |
| `started_at`, `ended_at` | timestamp, optional | |
| `updated_at` | timestamp | |
| `attempts` | Attempt[] | Numbered 1..n in order. |
| `costs` | Cost, optional | The whole run. |
| `roles` | map of `{model?, provider?}`, optional | Which model played each role. |
| `timing` | optional | `{wall_hours?, active_hours?, timebox?, timebox_hours_max?, over_timebox?}`; `timebox` is the producer's size label. |
| `sessions` | `[{id, provider?, model?, started_at?}]`, optional | Agent sessions; steps name them by id. |
| `outcome` | `{facts: Fact[]}`, optional | What the run produced, as facts. |
| `stop` | Stop, optional | Present when the run is stopped. |
| `links` | Link[] | |
| `documents` | DocRef[] | Every document of the run; steps name them by id. |
| `facts` | Fact[] | |
| `gaps` | string[] | What the producer knows is missing from this doc, e.g. `steps-not-retained`. |
| `digest` | sha256 hex | See **Digest**. |

**Attempt** — `{id, number, state, started_at?, ended_at?, steps: Step[],
facts, costs?, links, stop?}`. `id` SHOULD equal the attempt id in the work
ledger the factory records attempts in, when the producer knows it, so a
reader can join the two. `state` is a run state. An attempt MAY have no steps
(see run 5 of the golden feed).

**Step** — `{id, kind, name?, label?, iteration?, state, result?,
result_detail?, started_at?, ended_at?, seconds?, failure?: {code?, text?},
session?, facts, links, documents}`.

- `id` is unique within the run.
- `kind` is open (`^[a-z][a-z0-9_.-]*$`). The core kinds are `context`,
  `implement`, `gate`, `review`, `publish`, `ci`, `ship`, `recovery`, `plan`
  and `other`; a reader treats any other kind as `other`.
- `iteration` counts repeats of the same kind within an attempt (gate 1,
  gate 2).
- `state` is where the step is: `pending`, `running`, `done`, `unknown`.
  `result` is how it came out: `pass`, `fail`, `running`, `skipped`,
  `cancelled`, `unknown`. `result_detail` refines it in the producer's words
  (`approve`, `request-changes`).
- `session` is the id of an entry in the run's `sessions`; `documents` are ids
  of entries in the run's `documents`.

**Cost** — `{usd?, usd_with_overhead?, tokens?: {input?, output?, cache_read?,
cache_write?}, by_model?: {model: {usd?, tokens?}}, by_role?: {role: {usd?}},
complete, gaps}`. `complete` is false whenever usage the producer knows of is
missing, and `gaps` says which; a reader marks an incomplete total as a lower
bound.

**Link** — `{rel, href?, ref?, text?}` with at least one of `href` and `ref`.
`rel` is one of `pull_request`, `commit`, `branch`, `tree`, `ci`, `item`,
`attempt`, `external`. `ref` is an identifier (a commit sha, an item ref, an
attempt id) a reader can link internally; `href` is an absolute external URL.

**Stop** — why a run halted for a person, and what that person can do:
`{code, cause?, class?, title?, why?, detail?: string[], next?, commands?:
string[], owner_ask?: {problem?, question?, recommended?, if_yes?, if_not?},
at?}`. `code` is the producer's stable stop code; `class` groups codes;
`commands` are shown verbatim for a person to copy, and a reader MUST NOT run
them. An attempt MAY carry its own `stop`.

### Documents

A **DocRef** describes one document and its pages:

```json
{"id": "transcript", "kind": "transcript", "title": "Implementer transcript",
 "content_type": "application/x-ndjson", "bytes": 1658,
 "pages": [{"href": "docs/run-0002/transcript/0001.ndjson", "bytes": 912,
            "first_at": "…", "last_at": "…"}],
 "state": "ready", "source": {"kind": "session-log", "digest": "…"},
 "redaction": {"policy": "credentials-v1", "replaced": 1}}
```

- `kind`: `transcript`, `log`, `diagnostic`, `notes`, `contract`, `receipt`,
  `text`. `content_type`: `text/plain`, `application/x-ndjson`,
  `application/json`. Every page is UTF-8.
- `state`: `ready` (complete), `live` (still growing; more pages may follow),
  `truncated` (complete up to a size limit), `pending` (not produced yet),
  `unavailable` (the source is gone or unreadable), `withheld` (the producer
  will not publish it), `expired` (past retention). **Only `ready`, `live` and
  `truncated` documents have readable pages**; every other state has an empty
  `pages` list.
- `bytes` is the sum of the page sizes; each page's `bytes` is its file size.
- `source` identifies what the document was rendered from, opaquely.
- `redaction` names the policy applied and how many values it replaced.

**Transcript pages** are `application/x-ndjson`, one turn per line:

```json
{"at": "2026-11-14T11:00:32Z", "role": "tool_result", "tool": "read_file",
 "text": "pub fn total(…", "omitted_bytes": 18234}
```

`role` is `system`, `user`, `assistant`, `thinking`, `tool_call` or
`tool_result`; `tool` names the tool of a call or result; `model` the model
that produced the turn; `omitted_bytes` how much of the text the producer cut.
`text` is plain text. A reader MUST escape it for display and MUST NOT
interpret it as markup.

### `boards/<name>.json`

```json
{"schema": "runfeed.board.v1", "name": "stop-ratio", "title": "Stops per merge",
 "source_schema": "acme.stop_ratio.v2", "generated_at": "…", "body": {…}}
```

A board is a producer read model (a rollup, a report) passed through for
display. `name` equals its key in `feed.json`; `source_schema` names the
body's own format; `body` is any JSON value, which a reader renders
generically unless it knows `source_schema`.

### `changes.jsonl`

One JSON object per line, oldest first: `{revision, at, run, digest}`, written
whenever a run doc changes. Revisions never decrease; a publish that changes
no run (a board refresh) adds no line. The last line for a run carries that
run's current digest. A producer MAY drop lines from the head; a reader whose
cursor is older than the first line rereads the index. A reader ignores a
final line with no newline, which is an append in progress.

## States

| Run state | Meaning |
| --- | --- |
| `running` | In progress. |
| `passed` | Finished and achieved its subject (for example, landed). |
| `failed` | Finished without achieving it, and will not continue. |
| `stopped` | Halted for a person; `stop` says why and what to do. |
| `declined` | The factory decided not to do the work. |
| `superseded` | Replaced by a later run or a re-cut of the subject. |
| `unknown` | The producer cannot tell. |

Attempts use the same states.

## Digest

`digest` is the lowercase hex SHA-256 of the run doc's canonical JSON with the
`digest` and `updated_at` fields removed: UTF-8, object keys sorted by code
point, no insignificant whitespace, non-ASCII characters unescaped, numbers in
shortest round-trip form. In Python:

```python
body = {k: v for k, v in doc.items() if k not in ('digest', 'updated_at')}
canonical = json.dumps(body, sort_keys=True, separators=(',', ':'), ensure_ascii=False)
digest = hashlib.sha256(canonical.encode('utf-8')).hexdigest()
```

Excluding `updated_at` means that rewriting an unchanged run leaves its digest
alone. Readers use digests only to compare: an unchanged digest means an
unchanged run doc, so a cached copy is still good. A reader MUST NOT reject a
run doc because it computes a different digest.

## Publishing

A publish writes files in this order, each by writing a temporary file in the
same directory and renaming it over the target (an atomic replace):

1. document pages,
2. run docs,
3. subject files,
4. index shards,
5. `index.json`,
6. `changes.jsonl` (appended, whole lines only),
7. `feed.json`, last.

So a reader that sees a revision in `feed.json` finds every file that
revision names. A producer removes a file no longer named only after the
`feed.json` that last named it has been replaced for at least one full
publish interval, so a reader in the middle of a read does not lose it.

**Redaction.** A producer MUST remove credentials (passwords, tokens, keys,
connection strings and the like) from every document before publishing it,
and SHOULD remove them from facts and stop text too. If redaction fails for
any reason the document is published as `withheld`, with no pages; it never
falls back to publishing the unredacted bytes.

**Size guidance.** Keep each page at or under 512 KiB (the checker enforces
this), each run doc under 1 MiB, and `index.json` small enough to read on
every poll by moving older runs into shards. Cap the bytes per run and mark a
document that hit the cap `truncated`. Cut long tool results and record what
was cut in `omitted_bytes`.

## Producing a feed

1. Pick a stable run id per run and an attempt id per attempt (the ledger's,
   when there is one). Give each run a subject `ref` when the subject has a
   human-facing reference, so its subject file exists.
2. Build each run doc from the factory's own records. Leave out what you do
   not know rather than guessing, and name it in `gaps` (run, cost or
   `source_health`).
3. Render documents only when policy allows it, redact them, and page them.
4. Compute each digest, write in the publish order, bump `revision` once per
   publish, and append a `changes.jsonl` line for every run whose digest
   changed.
5. Check the result with `python3 tests/runfeed_schema.py --feed DIR
   --allow-orphans`.

## Reading a feed

1. Read `feed.json`. Refuse a `schema` other than `runfeed.v1`. Remember
   `revision`.
2. Read the index, and shards when older runs are asked for. Show overviews
   from RunSummary alone.
3. Fetch a run doc when it is opened, or when its digest differs from the one
   cached.
4. For an item page, request `subjects/<key>.json` directly; a missing file
   means the feed knows no runs for it.
5. Fetch document pages only on request, and only for documents whose state
   has pages. Escape every string the feed supplies before display, and make
   only `https://` links clickable.
6. Poll `feed.json`; when the revision moves, use `changes.jsonl` to find the
   runs that changed, or reread the index.
7. Resolve every href against the feed root and refuse any that would leave
   it. Ignore unknown fields; render unknown enum values neutrally.

## Versioning

Additive changes stay `runfeed.v1`: a new optional field, a new enum value, a
new step kind, fact type, document kind or board. Readers already ignore and
neutrally render what they do not know, and the schema in this repository is
updated in the same change. Anything else is `runfeed.v2`: removing or
renaming a field, making an optional field required, changing a field's type
or meaning, or changing the layout, digest rule or publish order. A producer
publishing v2 changes `feed.json` `schema`, and the per-file `schema` tags
(`runfeed.index.v1`, `runfeed.subject.v1`, `runfeed.run.v1`,
`runfeed.board.v1`) move with it.
