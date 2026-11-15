# Profiles (`fabriscope.profile.v1`)

A profile tells Fabriscope how one store uses its metadata, so the pages can
show held work, kinds, groups, people, typed fields, history and receipts
without Fabriscope knowing anything about the system that wrote the store.
Without a profile every row is a work item and the pages show what Beads
stores.

```sh
fabriscope --beads DIR --profile work.profile.toml
fabriscope profile check work.profile.toml   # validate and print it resolved
```

A profile is TOML. Unknown keys are refused, and every error names the key
that is wrong (`states[1].since: …`). `--parent-key PATH` still works and
wins over `[parent]`.

The complete example used by the tests is
[`tests/fixtures/work/profile.toml`](../tests/fixtures/work/profile.toml). It
uses a neutral `work.*` metadata namespace; a real profile uses whatever
namespace its store writes.

## Sources

Most keys name a **source**: where a value comes from.

| Source | Value |
| --- | --- |
| `a.b.c` or `meta:a.b.c` | The metadata value at that path. Lookup tries nested keys first, then the rest of the path as one literal dotted key at each level, so `a.b.c` also finds `{"a": {"b.c": …}}` and `{"a.b": {"c": …}}`. |
| `field:<name>` | A native column: `id`, `title`, `status`, `issue_type`, `priority`, `assignee`, `external_ref`, `created_at`, `updated_at`, `closed_at`, `close_reason`, `started_at`, `created_by`, `owner`, `estimated_minutes`, `due_at`, `defer_until`, `lease_expires_at`, `heartbeat_at`, `is_blocked`, `comment_count`. |
| `label:<prefix>` | The rest of the first label starting with the prefix (`label:area:` on `area:billing` is `billing`). |
| `seat:<n>` | The n-th `/` segment of the assignee, from 1. |
| `ref:<path>` | A metadata value naming another bead by id or external reference. |
| `event:<kind>` | The actor of the first history event of that kind. Read when a page is drawn, so allowed only in `[flow]` and `[[people.roles]]`. |
| `provenance:<kind>[.<key>]` | The first provenance event of that kind, or one payload key of it. Request time, so `[flow]` only. |

A missing value, `null`, `""` and `[]` all count as no value. Values are
compared and shown as text: a metadata `true` equals `"true"`, and a list has
one text per element.

## Conditions

A `when = { … }` table is a condition. Keys AND together; a key given a list
matches any one of its values; an empty condition matches everything.

| Key | Matches when |
| --- | --- |
| `status` | the stored status is one of the values |
| `type` | the issue type is one of the values |
| `has` / `lacks` | every source has a value / none has |
| `eq = { "source" = [values] }` | each source has one of the values (quote the source: TOML reads bare dotted keys as tables) |
| `label` / `label_prefix` / `id_prefix` | a label equals / a label starts with / the id starts with one of the values |
| `ready` | `bd ready` listed the bead (`true`) or not (`false`) |
| `past` | each source holds a timestamp before now, or a date before today (UTC) |
| `any = [ {…}, {…} ]` | at least one of the conditions matches |
| `not = { … }` | the condition does not match |

## Sections

- `schema = "fabriscope.profile.v1"` (required) and `name`.
- `[parent] path` — a source naming each bead's parent by id or external
  reference, for stores without parent-child links.
- `[documents] base_path` — a source holding the bead's document path;
  relative Markdown links resolve from it.
- `[[rows]] class, when` — `item` (the default), `receipt` or `hidden`; the
  first match wins. Receipts and hidden rows are left out of lists, counts,
  history and flow, and are never ready.
- `[[kinds]] key, label, when` — a display kind (first match) and the `kind`
  filter.
- `[[states]] key, label, tone, when, since, read_ahead, overrides_ready,
  facts, banner` — derived states (a bead can be in several). `tone` is
  `good`, `warn`, `bad`, `waiting` or `neutral`. `overrides_ready = true`
  means a bead in the state is never shown or counted as ready, whatever
  `bd ready` says. `banner` is a sentence with `{source}` holes. The Held tab
  appears when any state overrides ready or has the `bad` tone.
- `[[groups]] key, label, from, subgroup?, declared?` — group-bys at
  `/by/{key}` and URL facets `?{key}=value`. `from` sources are tried in
  order; `subgroup` is a value within the group (a lane); `declared` is read
  from the bead the group value names and gives the ordered subgroup values.
  Keys are lower case and may not be a reserved query key (`q`, `type`,
  `kind`, `priority`, `state`, `dstate`, `ready`, `person`, `epic`, `page`,
  `from`, `to`, `days`, `sort`, `by`, `event`). `by` picks what a flow or
  held chart is split by, and `event` an /activity event kind.
- `[people] segments` — how many leading `/` segments of a name identify a
  person. `[[people.roles]] key, label, from` names who holds each role.
  Sources are tried in order. A field or metadata source is read from the
  snapshot; an `event:<kind>` source is the actor of the bead's first
  history event of that kind, so a role that only an event names appears on
  /people and /person/{name} once the bead's thread has been read, and a
  profile without `[events]` never fills it. `[[people.classes]] prefix,
  class, segments` classes names apart, such as factory seats.
- `[[panels]] key, title, when, fields = [{path, label, kind, unit?, repo?}]`
  — typed field panels on the bead page. Kinds: `text`, `list`, `markdown`,
  `duration` (seconds, or written `2h`, `2-3h`, `4-6h active work`), `money`,
  `link` (only `https://` becomes a link), `id-ref`, `enum-badge`, `json`,
  `timestamp`, `code` (a commit links through `[links]` with `repo`),
  `count`, `bool` (`true`/`false` or their strings), `person`.
- `[events] marker, schema?, kind, at, actor, order, changes, notes` and
  `[events.kinds] <kind> = {label, tone?, claim?, stop?, created?}` — which
  comments are history events. A comment is an event when one of its lines
  starts with `marker`; the **last** such line holds the event as JSON after
  the marker. `schema = {key, equals}` checks it; `kind`, `at` and `actor`
  name its keys (defaults `kind`, `at`, `actor`; time and actor fall back to
  the comment's); `changes` names a `{field: [old, new]}` key; `notes` keys
  become notes; `order` keys sort events at the same time. Any other comment
  is plain Markdown.
- `[[receipts]] when, kind, group, at, actor, subject, attempt, fields` — how
  receipt rows name their kind, subject (sources tried in order, each by id
  or external reference, so both joins work) and attempt.
- `[links] repos = {name = "https://…"}`, `commit`, `pull_request`, `tree` —
  templates with `{repo}` and `{value}`; `[[links.ids]] prefix, target =
  "bead" | "scroll-alias" | "template", template?` turns identifiers into
  links. Only `https://` ever becomes an external link.
- `[flow] claim_at, claimant, items, wip_status` and `[flow.decisions] when,
  since, assignee, due` — what the flow pages measure. Defaults: `claim_at =
  "field:started_at"`, `wip_status = ["in_progress"]`, decisions are open
  `decision` rows since `field:created_at`.
- `[labels]` — labels for opaque fact keys, such as a run feed's.

## What each section turns on

A tab shows only when there is something behind it, so a store without a
profile looks as it always did.

| Tab or page | Needs |
| --- | --- |
| Held (`/held`) | a `[[states]]` entry that overrides ready or has the `bad` tone |
| Receipts (`/receipts`) | a `[[receipts]]` entry |
| a group tab (`/by/{key}`) | a `[[groups]]` entry |
| People, Activity | always; roles and history events need `[[people.roles]]` and `[events]` |
| Flow | always; claim times and decisions read `[flow]` |
| Runs, Boards | `--runs` |
| Costs | a Beads store or `--runs`; spend comes from provenance `used` events and the feed, and an unknown amount shows as unknown, never $0 |

An absent `state` filter means two things. The lists (/beads, /epics,
/held, /by/…) show every item not closed. The pages that count over time
(/flow/…, /activity, /people, /costs) count any status. Only an explicit
`state=` value narrows those, and their links to a list carry
`state=all` so the list shows the same items.

## The work fixture

`tests/fixtures/work/` is a small store in the profile's shape, used by unit
tests (`testing::work_view()`), live tests (`common::work_store`) and the
browser checks (`tests/theme.py`). Subject refs match the golden run feed in
`tests/fixtures/runfeed/v1/`.

| Bead | Ref | What it is |
| --- | --- | --- |
| `wk-e1` | `EPIC-0001` | Epic; parent of wk-1 … wk-4 through `work.fields.parent_epic`. |
| `wk-m1` | `v1` | Milestone declaring lanes `build`, `polish`. |
| `wk-1` | `WORK-0001` | In progress: packet, claimed by the seat `factory/hostA/rigA`, `started_at`, claim event, plain comment by bob; provenance attempt `att-0001-1`; run-0001 (running). |
| `wk-2` | `WORK-0002` | Closed packet with two attempts (`att-0002-1` stopped and resumed, `att-0002-2` landed), usage on both; gate, ship and operation receipts; run-0002 (passed) and run-0004 (superseded). |
| `wk-3` | `WORK-0003` | **Held**: open, `work.suspended` (`scope-exceeded`, attempt `att-0003-1`, rig `rigB`), listed by `bd ready` but never ready; run-0003 (stopped). |
| `wk-4` | `WORK-0004` | Gate-type item (`issue_type = "gate"`, `gate = "true"`). |
| `wk-5` | `WORK-0005` | Closed early; run-0005. |
| `wk-q1` | `Q-0001` | Open decision past its due date (`overdue`), asked of bob. |
| `wk-rc-gate`, `wk-rc-ship` | — | Receipts for wk-2 attempt `att-0002-2`, joined by external ref and by bead id. |
| `wk-op-1`, `wk-op-2` | — | Operation receipts joined by `result.id` and `result.bead_id`. |
| `wk-coordination`, `wk-slot` | — | Hidden rows (a `work-state` row, a `work:coordination` label). |

People: `alice` (creator, comments), `bob` (decision, plain comment), and
the factory seats `factory/hostA/rigA` and `factory/hostA/rigB`.
