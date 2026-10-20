#!/usr/bin/env python3
"""Browser regression checks against the real server and disposable stores.

Requires playwright; run `playwright install chromium` once, then
`python3 tests/theme.py --binary target/debug/fabriscope`.
"""

import argparse
import shutil
import subprocess
import tempfile
import time
from pathlib import Path

from playwright.sync_api import expect, sync_playwright

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--browser', help='Optional Chromium executable path')
    parser.add_argument('--screenshots', type=Path)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix='fabriscope-theme-') as scratch:
        store = Path(scratch)
        issues = store / 'issues.jsonl'
        shutil.copy(ROOT / 'tests/fixtures/issues.jsonl', issues)
        (store / 'commit.json').write_text('{"commit":"before"}')
        bd = store / 'bd'
        bd.write_text('''#!/bin/sh
case "$1" in
  serve) echo 'Error: test uses export' >&2; exit 1 ;;
  export) cat issues.jsonl ;;
  ready) echo '[{"id":"demo-1"}]' ;;
  vc) cat commit.json ;;
  *) exit 2 ;;
esac
''')
        bd.chmod(0o755)
        scrolls = store / 'scrolls'
        shutil.copy(ROOT / 'tests/fixtures/scrolls.py', scrolls)
        scrolls.chmod(0o755)
        shutil.copy(ROOT / 'tests/fixtures/graph.json', store / 'graph.json')
        (store / '.scrolls').mkdir()
        with (store / 'server.log').open('w+') as log:
            server = subprocess.Popen([
                str(args.binary.resolve()), '--beads', str(store), '--bd', str(bd),
                '--scrolls', str(store / '.scrolls'), '--scrolls-bin', str(scrolls),
                '--title', 'Theme test', '--bind', '127.0.0.1:0', '--no-update',
            ], stdout=log, stderr=log)
            try:
                url = None
                deadline = time.monotonic() + 30
                while time.monotonic() < deadline:
                    log.seek(0)
                    for line in log:
                        if 'http://127.0.0.1:' in line:
                            url = line[line.index('http://127.0.0.1:'):].split()[0]
                    if url:
                        break
                    assert server.poll() is None, 'Server exited during startup'
                    time.sleep(0.05)
                assert url, 'Server did not announce its address'
                run_browser(args, url, issues, store)
            finally:
                server.terminate()
                server.wait(timeout=10)
    print('Theme browser checks passed')


def run_browser(args, url, issues, store):
    with sync_playwright() as pw:
        browser = pw.chromium.launch(executable_path=args.browser)
        try:
            context = browser.new_context(color_scheme='light', viewport={'width': 1440, 'height': 1000})
            page = context.new_page()
            errors = []
            page.on('pageerror', lambda error: errors.append(str(error)))
            page.on('console', lambda msg: errors.append(msg.text) if msg.type == 'error' and not msg.location.get('url', '').endswith('/favicon.ico') else None)
            page.goto(url)
            control = page.get_by_label('Theme', exact=True)
            expect(control).to_have_value('system')

            def scheme(wanted, target=page):
                expect(target.locator('html')).to_have_css('color-scheme', wanted)

            scheme('light')
            page.emulate_media(color_scheme='dark')
            scheme('dark')
            control.select_option('light')
            scheme('light')
            control.select_option('dark')
            page.emulate_media(color_scheme='light')
            scheme('dark')
            page.reload()
            scheme('dark')
            expect(control).to_have_value('dark')
            # Every page uses the shared header and palette.
            for path in ['/beads', '/epics', '/scrolls', '/bead/demo-1', '/scroll/sc-d1']:
                response = page.goto(url + path)
                assert response.status == 200, path
                scheme('dark')
                expect(control).to_have_value('dark')
            page.goto(url)
            # Another tab observes a change without a reload.
            other = context.new_page()
            other.goto(url + '/beads')
            control.select_option('light')
            scheme('light', other)
            expect(other.get_by_label('Theme', exact=True)).to_have_value('light')
            control.select_option('dark')
            scheme('dark', other)
            other.close()
            # Automatic HTML replacement must preserve the header's choice.
            issues.write_text(issues.read_text().replace('Write the README', 'Theme refresh verified'))
            (store / 'commit.json').write_text('{"commit":"after"}')
            expect(page.get_by_role('link', name='Theme refresh verified', exact=True)).to_be_visible(timeout=15000)
            scheme('dark')
            expect(control).to_have_value('dark')
            # Native keyboard operation and a visible focus ring.
            control.focus()
            page.keyboard.press('ArrowUp')
            page.keyboard.press('Enter')
            expect(control).to_have_value('light')
            scheme('light')
            assert control.evaluate('el => getComputedStyle(el).outlineStyle') != 'none'
            # Check representative text pairs in both palettes (WCAG AA).
            for theme in ['light', 'dark']:
                control.select_option(theme)
                contrast = page.evaluate('''() => {
                  const s = getComputedStyle(document.documentElement);
                  const color = key => {
                    let h = s.getPropertyValue(key).trim().slice(1);
                    if (h.length === 3) h = [...h].map(c => c+c).join('');
                    const rgb = h.match(/../g).map(v => parseInt(v,16)/255)
                      .map(v => v <= .04045 ? v/12.92 : ((v+.055)/1.055)**2.4);
                    return rgb[0]*.2126 + rgb[1]*.7152 + rgb[2]*.0722;
                  };
                  return [['--ink','--paper'],['--muted','--paper'],['--muted','--bg'],
                    ['--blue','--paper'],['--green','--paper'],['--amber','--paper'],
                    ['--ink','--table-head'],['--ink','--row-hover'],['--ink','--code-bg'],
                    ['--badge-blue','--badge-blue-bg'],['--badge-green','--badge-green-bg'],
                    ['--badge-amber','--badge-amber-bg'],['--badge-neutral','--badge-neutral-bg'],
                    ['--alert-ink','--alert-bg'],['--button-ink','--ink']].map(([a,b]) => {
                      const x=color(a), y=color(b);
                      return [a,b,(Math.max(x,y)+.05)/(Math.min(x,y)+.05)];
                    });
                }''')
                assert all(ratio >= 4.5 for _, _, ratio in contrast), (theme, contrast)
                for width in [1440, 768, 375, 320]:
                    page.set_viewport_size({'width': width, 'height': 1000})
                    expect(control).to_be_visible()
                    assert page.evaluate('document.documentElement.scrollWidth <= innerWidth'), (theme, width)
                    if args.screenshots and width in [1440, 375]:
                        args.screenshots.mkdir(parents=True, exist_ok=True)
                        page.screenshot(path=str(args.screenshots / f'{theme}-{width}.png'), full_page=True)
            page.evaluate("localStorage.setItem('fabriscope.theme', 'invalid')")
            page.reload()
            expect(control).to_have_value('system')
            scheme('light')
            # Clearing site storage restores System in all open tabs.
            other = context.new_page()
            other.goto(url)
            control.select_option('dark')
            scheme('dark', other)
            page.evaluate('localStorage.clear()')
            scheme('light', other)
            other.close()
            assert not errors, errors
            context.close()
            for javascript, blocked in [(False, False), (True, True)]:
                context = browser.new_context(java_script_enabled=javascript, color_scheme='dark')
                if blocked:
                    context.add_init_script("Object.defineProperty(window, 'localStorage', {get() {throw new Error('blocked')}})")
                page = context.new_page()
                page.goto(url)
                scheme('dark', page)
                control = page.get_by_label('Theme', exact=True)
                if javascript:
                    control.select_option('light')
                    scheme('light', page)
                    expect(page.locator('#connection')).to_have_text('● Live')
                else:
                    expect(control).to_be_hidden()
                context.close()
        finally:
            browser.close()


if __name__ == '__main__':
    main()
