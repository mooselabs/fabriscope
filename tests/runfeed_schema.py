#!/usr/bin/env python3
"""Check a run feed directory against schema/runfeed-v1.json (docs/runfeed-v1.md).

Standard library only. Validates every file of the feed against its schema
definition, then checks the feed as a whole: every href resolves inside the
feed directory, every indexed run has a run doc that agrees with its summary,
every subject file lists runs that exist, and every digest is the digest of
its run doc. Exits non-zero on any failure.

    python3 tests/runfeed_schema.py                      # the golden feed
    python3 tests/runfeed_schema.py --feed DIR --allow-orphans
    python3 tests/runfeed_schema.py --fix-digests        # rewrite digests and byte counts
"""

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PAGE_LIMIT = 512 * 1024
NO_PAGES = {'pending', 'unavailable', 'withheld', 'expired'}


# ---------------------------------------------------------------- JSON Schema subset

ANNOTATIONS = {'$schema', '$id', '$comment', '$defs', 'title', 'description', 'examples', 'default'}
KEYWORDS = {
    'type', 'required', 'enum', 'const', 'properties', 'additionalProperties', 'items',
    'propertyNames', '$ref', 'allOf', 'anyOf', 'oneOf', 'if', 'then', 'else', 'pattern',
    'minimum', 'maximum', 'minLength', 'maxLength', 'minItems', 'maxItems', 'uniqueItems',
}


class SchemaError(Exception):
    pass


def is_type(value, name):
    if name == 'null':
        return value is None
    if name == 'boolean':
        return isinstance(value, bool)
    if name == 'integer':
        if isinstance(value, bool):
            return False
        return isinstance(value, int) or (isinstance(value, float) and value.is_integer())
    if name == 'number':
        return isinstance(value, (int, float)) and not isinstance(value, bool)
    if name == 'string':
        return isinstance(value, str)
    if name == 'array':
        return isinstance(value, list)
    if name == 'object':
        return isinstance(value, dict)
    raise SchemaError(f'unknown type {name!r}')


def json_equal(a, b):
    if isinstance(a, bool) or isinstance(b, bool):
        return type(a) is type(b) and a == b
    if isinstance(a, dict) and isinstance(b, dict):
        return a.keys() == b.keys() and all(json_equal(a[k], b[k]) for k in a)
    if isinstance(a, list) and isinstance(b, list):
        return len(a) == len(b) and all(json_equal(x, y) for x, y in zip(a, b))
    return type(a) is type(b) and a == b or (is_type(a, 'number') and is_type(b, 'number') and a == b)


class Validator:
    """The subset of draft 2020-12 this schema uses. Any other keyword is a SchemaError,
    so the schema can never rely on a rule this checker silently skips."""

    def __init__(self, schema):
        self.root = schema
        self._walk(schema, '#')

    def resolve(self, ref):
        if not ref.startswith('#/'):
            raise SchemaError(f'only local $ref is supported: {ref}')
        node = self.root
        for part in ref[2:].split('/'):
            part = part.replace('~1', '/').replace('~0', '~')
            if not isinstance(node, dict) or part not in node:
                raise SchemaError(f'unresolved $ref {ref}')
            node = node[part]
        return node

    def _walk(self, node, where):
        if isinstance(node, bool):
            return
        if not isinstance(node, dict):
            raise SchemaError(f'{where}: schema must be an object or boolean')
        unknown = set(node) - KEYWORDS - ANNOTATIONS
        if unknown:
            raise SchemaError(f'{where}: unsupported keywords {sorted(unknown)}')
        if '$ref' in node:
            self.resolve(node['$ref'])
        if 'pattern' in node:
            re.compile(node['pattern'])
        for key in ('$defs', 'properties'):
            for name, child in node.get(key, {}).items():
                self._walk(child, f'{where}/{key}/{name}')
        for key in ('additionalProperties', 'items', 'propertyNames', 'if', 'then', 'else'):
            if key in node:
                self._walk(node[key], f'{where}/{key}')
        for key in ('allOf', 'anyOf', 'oneOf'):
            for i, child in enumerate(node.get(key, [])):
                self._walk(child, f'{where}/{key}/{i}')

    def errors(self, value, schema, path='$'):
        out = []
        self._check(value, schema, path, out)
        return out

    def _check(self, value, schema, path, out):
        if schema is True:
            return
        if schema is False:
            out.append(f'{path}: no value is allowed here')
            return
        if '$ref' in schema:
            self._check(value, self.resolve(schema['$ref']), path, out)
        if 'type' in schema:
            types = schema['type'] if isinstance(schema['type'], list) else [schema['type']]
            if not any(is_type(value, t) for t in types):
                out.append(f'{path}: expected {"|".join(types)}, got {type(value).__name__}')
                return
        if 'const' in schema and not json_equal(value, schema['const']):
            out.append(f'{path}: must be {schema["const"]!r}')
        if 'enum' in schema and not any(json_equal(value, e) for e in schema['enum']):
            out.append(f'{path}: {value!r} is not one of {schema["enum"]}')
        if isinstance(value, str):
            if 'pattern' in schema and not re.search(schema['pattern'], value):
                out.append(f'{path}: {value!r} does not match {schema["pattern"]}')
            if len(value) < schema.get('minLength', 0):
                out.append(f'{path}: shorter than {schema["minLength"]}')
            if 'maxLength' in schema and len(value) > schema['maxLength']:
                out.append(f'{path}: longer than {schema["maxLength"]}')
        if is_type(value, 'number'):
            if 'minimum' in schema and value < schema['minimum']:
                out.append(f'{path}: below {schema["minimum"]}')
            if 'maximum' in schema and value > schema['maximum']:
                out.append(f'{path}: above {schema["maximum"]}')
        if isinstance(value, list):
            if len(value) < schema.get('minItems', 0):
                out.append(f'{path}: fewer than {schema["minItems"]} items')
            if 'maxItems' in schema and len(value) > schema['maxItems']:
                out.append(f'{path}: more than {schema["maxItems"]} items')
            if schema.get('uniqueItems'):
                seen = [json.dumps(v, sort_keys=True) for v in value]
                if len(set(seen)) != len(seen):
                    out.append(f'{path}: items are not unique')
            if 'items' in schema:
                for i, item in enumerate(value):
                    self._check(item, schema['items'], f'{path}[{i}]', out)
        if isinstance(value, dict):
            for key in schema.get('required', []):
                if key not in value:
                    out.append(f'{path}: missing {key!r}')
            props = schema.get('properties', {})
            for key, item in value.items():
                if 'propertyNames' in schema:
                    self._check(key, schema['propertyNames'], f'{path} key {key!r}', out)
                if key in props:
                    self._check(item, props[key], f'{path}.{key}', out)
                elif 'additionalProperties' in schema:
                    extra = schema['additionalProperties']
                    if extra is False:
                        out.append(f'{path}: unexpected field {key!r}')
                    else:
                        self._check(item, extra, f'{path}.{key}', out)
        for sub in schema.get('allOf', []):
            self._check(value, sub, path, out)
        if 'anyOf' in schema and not any(not self.errors(value, s, path) for s in schema['anyOf']):
            out.append(f'{path}: matches none of anyOf')
        if 'oneOf' in schema:
            hits = sum(1 for s in schema['oneOf'] if not self.errors(value, s, path))
            if hits != 1:
                out.append(f'{path}: matches {hits} of oneOf, expected 1')
        if 'if' in schema:
            branch = 'then' if not self.errors(value, schema['if'], path) else 'else'
            if branch in schema:
                self._check(value, schema[branch], path, out)


# ---------------------------------------------------------------- feed rules

def segment(name):
    """The file name a subject ref (or run id) is published under (docs/runfeed-v1.md)."""
    safe = b'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_.~'
    out = ''.join(chr(b) if b in safe else f'%{b:02X}' for b in name.encode('utf-8'))
    return '%2E' + out[1:] if out.startswith('.') else out


def digest_of(run_doc):
    body = {k: v for k, v in run_doc.items() if k not in ('digest', 'updated_at')}
    canonical = json.dumps(body, sort_keys=True, separators=(',', ':'), ensure_ascii=False, allow_nan=False)
    return hashlib.sha256(canonical.encode('utf-8')).hexdigest()


def strict_object(pairs):
    out = {}
    for key, value in pairs:
        if key in out:
            raise ValueError(f'duplicate key {key!r}')
        out[key] = value
    return out


def parse_json(text):
    def no_constant(name):
        raise ValueError(f'{name} is not JSON')
    return json.loads(text, object_pairs_hook=strict_object, parse_constant=no_constant)


def dump(path, value):
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + '\n', encoding='utf-8')


class Feed:
    def __init__(self, root, schema_path):
        self.root = root.resolve()
        self.validator = Validator(json.loads(schema_path.read_text(encoding='utf-8')))
        self.problems = []
        self.reached = set()

    def fail(self, message):
        self.problems.append(message)

    def path(self, href, where):
        """Resolve a feed-relative href, refusing anything outside the root."""
        if not isinstance(href, str):
            self.fail(f'{where}: href is not a string')
            return None
        target = (self.root / href).resolve()
        if target != self.root and self.root not in target.parents:
            self.fail(f'{where}: {href!r} escapes the feed root')
            return None
        if not target.is_file():
            self.fail(f'{where}: {href!r} does not exist')
            return None
        self.reached.add(target)
        return target

    def load(self, path, definition):
        rel = path.relative_to(self.root).as_posix()
        try:
            value = parse_json(path.read_text(encoding='utf-8'))
        except (ValueError, UnicodeDecodeError) as error:
            self.fail(f'{rel}: not JSON: {error}')
            return None
        self.validate(value, definition, rel)
        return value

    def validate(self, value, definition, where):
        schema = {'$ref': f'#/$defs/{definition}'}
        for error in self.validator.errors(value, schema, where):
            self.fail(error)

    def check(self, allow_orphans=False, fix=False):
        feed_path = self.root / 'feed.json'
        if not feed_path.is_file():
            self.fail('feed.json is missing')
            return self.problems
        self.reached.add(feed_path.resolve())
        feed = self.load(feed_path, 'feed')
        if not isinstance(feed, dict):
            return self.problems

        listings = {}  # path -> index document
        for href in [feed.get('index')] + list(feed.get('shards', [])):
            path = self.path(href, 'feed.json')
            if path:
                listings[path] = self.load(path, 'index')
        index = listings.get(self.path(feed.get('index'), 'feed.json')) if feed.get('index') else None
        if isinstance(index, dict) and isinstance(feed.get('revision'), int):
            if index.get('revision', -1) < feed['revision']:
                self.fail('index.json: revision is older than feed.json')

        runs = {}  # run id -> (summary, run doc, listing path)
        for listing_path, listing in listings.items():
            for i, summary in enumerate((listing or {}).get('runs', [])):
                where = f'{listing_path.relative_to(self.root).as_posix()} runs[{i}]'
                run_id = summary.get('run')
                if run_id in runs:
                    self.fail(f'{where}: run {run_id!r} is listed twice')
                    continue
                path = self.path(summary.get('href'), where)
                doc = self.load(path, 'run') if path else None
                if isinstance(doc, dict):
                    if fix:
                        self.fix_bytes(doc)
                        doc['digest'] = digest_of(doc)
                        summary['digest'] = doc['digest']
                        dump(path, doc)
                    self.check_run(doc, summary, where)
                runs[run_id] = (summary, doc, listing_path)
        if fix:
            for listing_path, listing in listings.items():
                dump(listing_path, listing)

        self.check_subjects(runs)
        for name, href in (feed.get('boards') or {}).items():
            path = self.path(href, f'feed.json boards.{name}')
            board = self.load(path, 'board') if path else None
            if isinstance(board, dict) and board.get('name') != name:
                self.fail(f'{href}: name {board.get("name")!r} differs from its key {name!r}')
        self.check_changes(runs, fix)

        if not allow_orphans:
            for path in sorted(p for p in self.root.rglob('*') if p.is_file()):
                if path.resolve() not in self.reached:
                    self.fail(f'{path.relative_to(self.root).as_posix()}: no feed file names it')
        return self.problems

    def fix_bytes(self, doc):
        for document in doc.get('documents', []):
            total = 0
            for page in document.get('pages', []):
                path = (self.root / page['href']).resolve()
                if path.is_file():
                    page['bytes'] = path.stat().st_size
                    total += page['bytes']
            if document.get('pages'):
                document['bytes'] = total

    def check_run(self, doc, summary, where):
        name = f'run {doc.get("run")!r}'
        if doc.get('digest') != digest_of(doc):
            self.fail(f'{name}: digest is not the digest of its run doc (run --fix-digests)')
        for key in ('run', 'state', 'updated_at', 'digest', 'subject', 'group', 'started_at', 'ended_at'):
            if not json_equal(summary.get(key), doc.get(key)):
                self.fail(f'{where}: {key} {summary.get(key)!r} differs from the run doc {doc.get(key)!r}')
        attempts = doc.get('attempts', [])
        if summary.get('attempts') != len(attempts):
            self.fail(f'{where}: attempts {summary.get("attempts")} but the run doc has {len(attempts)}')
        stop_code = (doc.get('stop') or {}).get('code')
        if summary.get('stop_code') != stop_code:
            self.fail(f'{where}: stop_code {summary.get("stop_code")!r} differs from stop.code {stop_code!r}')
        if 'cost_usd' in summary and summary['cost_usd'] != (doc.get('costs') or {}).get('usd'):
            self.fail(f'{where}: cost_usd differs from costs.usd')

        attempt_ids = [a.get('id') for a in attempts]
        if len(set(attempt_ids)) != len(attempt_ids):
            self.fail(f'{name}: attempt ids repeat')
        if [a.get('number') for a in attempts] != list(range(1, len(attempts) + 1)):
            self.fail(f'{name}: attempts are not numbered 1..{len(attempts)} in order')
        if 'attempt' in doc.get('refs', {}) and doc['refs']['attempt'] not in attempt_ids:
            self.fail(f'{name}: refs.attempt names no attempt')
        steps = [s for a in attempts for s in a.get('steps', [])]
        step_ids = [s.get('id') for s in steps]
        if len(set(step_ids)) != len(step_ids):
            self.fail(f'{name}: step ids repeat')
        if 'current_step' in doc and doc['current_step'] not in step_ids:
            self.fail(f'{name}: current_step names no step')
        sessions = {s.get('id') for s in doc.get('sessions', [])}
        documents = doc.get('documents', [])
        doc_ids = [d.get('id') for d in documents]
        if len(set(doc_ids)) != len(doc_ids):
            self.fail(f'{name}: document ids repeat')
        for step in steps:
            if 'session' in step and step['session'] not in sessions:
                self.fail(f'{name} step {step.get("id")!r}: session names no session')
            for ref in step.get('documents', []):
                if ref not in doc_ids:
                    self.fail(f'{name} step {step.get("id")!r}: document {ref!r} is not in documents')
        for document in documents:
            self.check_document(document, f'{name} document {document.get("id")!r}')

    def check_document(self, document, where):
        pages = document.get('pages', [])
        if document.get('state') in NO_PAGES and pages:
            self.fail(f'{where}: state {document["state"]!r} has pages')
        total = 0
        for i, page in enumerate(pages):
            path = self.path(page.get('href'), f'{where} pages[{i}]')
            if not path:
                continue
            data = path.read_bytes()
            total += len(data)
            if 'bytes' in page and page['bytes'] != len(data):
                self.fail(f'{where} pages[{i}]: bytes {page["bytes"]} but the file has {len(data)}')
            if len(data) > PAGE_LIMIT:
                self.fail(f'{where} pages[{i}]: larger than 512 KiB')
            try:
                text = data.decode('utf-8')
            except UnicodeDecodeError:
                self.fail(f'{where} pages[{i}]: not UTF-8')
                continue
            kind = document.get('content_type')
            if kind == 'application/json':
                try:
                    parse_json(text)
                except ValueError as error:
                    self.fail(f'{where} pages[{i}]: not JSON: {error}')
            elif kind == 'application/x-ndjson':
                for n, line in enumerate(text.splitlines(), 1):
                    try:
                        value = parse_json(line)
                    except ValueError as error:
                        self.fail(f'{page["href"]}:{n}: not JSON: {error}')
                        continue
                    if document.get('kind') == 'transcript':
                        self.validate(value, 'transcript_turn', f'{page["href"]}:{n}')
        if pages and 'bytes' in document and document['bytes'] != total:
            self.fail(f'{where}: bytes {document["bytes"]} but its pages total {total}')

    def check_subjects(self, runs):
        expected = {}  # file name -> {run ids}
        for run_id, (summary, doc, _) in runs.items():
            ref = (summary.get('subject') or {}).get('ref')
            if ref is not None:
                expected.setdefault(segment(ref) + '.json', set()).add(run_id)
        folder = self.root / 'subjects'
        present = {p.name: p for p in folder.glob('*.json')} if folder.is_dir() else {}
        for name in sorted(set(expected) - set(present)):
            self.fail(f'subjects/{name}: missing for runs {sorted(expected[name])}')
        for name, path in sorted(present.items()):
            self.reached.add(path.resolve())
            subject = self.load(path, 'subject_file')
            if not isinstance(subject, dict):
                continue
            ref = (subject.get('subject') or {}).get('ref', '')
            if segment(ref) + '.json' != name:
                self.fail(f'subjects/{name}: holds ref {ref!r}, which publishes as {segment(ref)}.json')
            listed = set()
            for i, entry in enumerate(subject.get('runs', [])):
                where = f'subjects/{name} runs[{i}]'
                listed.add(entry.get('run'))
                target = self.path(entry.get('href'), where)
                known = runs.get(entry.get('run'))
                if not known or not isinstance(known[1], dict):
                    self.fail(f'{where}: run {entry.get("run")!r} is in no index')
                    continue
                doc = known[1]
                if target and target != (self.root / known[0]['href']).resolve():
                    self.fail(f'{where}: href differs from the index href')
                if (doc.get('subject') or {}).get('ref') != ref:
                    self.fail(f'{where}: the run is for subject {doc.get("subject")!r}')
                if entry.get('state') != doc.get('state'):
                    self.fail(f'{where}: state differs from the run doc')
                if 'attempt' in entry and entry['attempt'] not in [a.get('id') for a in doc.get('attempts', [])]:
                    self.fail(f'{where}: attempt names no attempt of the run')
                if entry.get('started_at') != doc.get('started_at'):
                    self.fail(f'{where}: started_at differs from the run doc')
            for run_id in sorted(expected.get(name, set()) - listed):
                self.fail(f'subjects/{name}: does not list run {run_id!r}')

    def check_changes(self, runs, fix):
        path = self.root / 'changes.jsonl'
        if not path.is_file():
            return
        self.reached.add(path.resolve())
        lines = path.read_text(encoding='utf-8').splitlines()
        values, last = [], {}
        previous = -1
        for n, line in enumerate(lines, 1):
            try:
                value = parse_json(line)
            except ValueError as error:
                self.fail(f'changes.jsonl:{n}: not JSON: {error}')
                continue
            self.validate(value, 'change', f'changes.jsonl:{n}')
            values.append(value)
            revision = value.get('revision', -1)
            if isinstance(revision, int):
                if revision < previous:
                    self.fail(f'changes.jsonl:{n}: revision goes backwards')
                previous = revision
            last[value.get('run')] = value
        if fix:
            for run_id, value in last.items():
                doc = runs.get(run_id, (None, None))[1]
                if isinstance(doc, dict):
                    value['digest'] = doc['digest']
            path.write_text(''.join(json.dumps(v, ensure_ascii=False) + '\n' for v in values), encoding='utf-8')
        for run_id, value in last.items():
            doc = runs.get(run_id, (None, None))[1]
            if isinstance(doc, dict) and value.get('digest') != doc.get('digest'):
                self.fail(f'changes.jsonl: last line for {run_id!r} does not carry its current digest')


def self_test():
    assert segment('WORK-0001') == 'WORK-0001'
    assert segment('a/b c') == 'a%2Fb%20c'
    assert segment('..') == '%2E.'
    assert segment('é') == '%C3%A9'
    v = Validator({'$defs': {'n': {'type': 'integer'}}, 'type': 'array', 'items': {'$ref': '#/$defs/n'}})
    assert v.errors([1, 2.0], v.root) == []
    assert v.errors([True], v.root), 'a boolean is not an integer'
    assert digest_of({'b': 1, 'a': 'é', 'digest': 'x', 'updated_at': 'y'}) == hashlib.sha256(
        '{"a":"é","b":1}'.encode('utf-8')).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--feed', type=Path, default=ROOT / 'tests/fixtures/runfeed/v1')
    parser.add_argument('--schema', type=Path, default=ROOT / 'schema/runfeed-v1.json')
    parser.add_argument('--allow-orphans', action='store_true',
                        help='Accept files no feed file names (a live feed keeps them for a grace period).')
    parser.add_argument('--fix-digests', action='store_true',
                        help='Rewrite run digests, index and changes digests, and page byte counts, then check.')
    args = parser.parse_args()
    self_test()
    try:
        if args.fix_digests:
            Feed(args.feed, args.schema).check(allow_orphans=True, fix=True)
        problems = Feed(args.feed, args.schema).check(allow_orphans=args.allow_orphans)
    except SchemaError as error:
        print(f'schema: {error}', file=sys.stderr)
        return 2
    for problem in problems:
        print(problem, file=sys.stderr)
    if problems:
        print(f'{len(problems)} problem(s) in {args.feed}', file=sys.stderr)
        return 1
    print(f'ok: {args.feed}')
    return 0


if __name__ == '__main__':
    sys.exit(main())
