"""Browser check for the run feed pages (WP5): /runs, a run that fills in
after its doc is fetched, its transcript page with escaped turns, and the
boards, each at desktop width and at 320 px with no horizontal scroll.

Run by tests/theme.py, which calls `check(page, url)` against the work
fixture server (golden run feed in tests/fixtures/runfeed/v1).
"""

from playwright.sync_api import expect


def fits(page, where):
    for width in [1440, 320]:
        page.set_viewport_size({'width': width, 'height': 900})
        assert page.evaluate('document.documentElement.scrollWidth <= innerWidth'), (where, width)
    page.set_viewport_size({'width': 1440, 'height': 900})


def check(page, url):
    page.goto(url + '/runs')
    expect(page.locator('table.runs tbody tr')).to_have_count(5)
    expect(page.locator('.run-tiles .metric')).to_have_count(5)
    expect(page.locator('.subnav a[aria-current="page"]')).to_have_text('Runs')
    fits(page, '/runs')

    # A filter lives in the URL.
    page.get_by_label('State').select_option('stopped')
    page.get_by_role('button', name='Apply').click()
    expect(page.locator('table.runs tbody tr')).to_have_count(1)

    # The run doc is fetched on the first visit; the page fills in by itself.
    page.goto(url + '/run/run-0002')
    expect(page.locator('#attempt-2')).to_be_visible(timeout=15000)
    expect(page.locator('#attempt-1 .step')).to_have_count(3)
    expect(page.locator('a[href="/run/run%2D0002/doc/transcript"]').first).to_be_visible()
    fits(page, '/run/run-0002')

    # The stopped run shows its stop and commands; the withheld transcript
    # has no link.
    page.goto(url + '/run/run-0003')
    expect(page.locator('.banner.stop').first).to_be_visible(timeout=15000)
    expect(page.locator('pre.command')).to_have_count(2)
    expect(page.locator('a[href="/run/run%2D0003/doc/transcript"]')).to_have_count(0)
    fits(page, '/run/run-0003')

    # Transcript turns are plain text: the script in the fixture is shown,
    # never run or parsed.
    page.goto(url + '/run/run-0002/doc/transcript?page=1')
    expect(page.locator('.transcript')).to_be_visible(timeout=15000)
    expect(page.get_by_text('<script>alert(1)</script>', exact=False)).to_be_visible()
    assert page.locator('.transcript script').count() == 0
    tool = page.locator('details.turn.tool').first
    expect(tool).not_to_have_attribute('open', '')
    tool.locator('summary').click()
    expect(tool.locator('.tool-result')).to_be_visible()
    fits(page, 'transcript')

    page.goto(url + '/run/run-0002/doc/gate-log')
    expect(page.locator('#L10')).to_be_visible(timeout=15000)
    fits(page, 'gate log')

    page.goto(url + '/boards')
    expect(page.locator('a[href="/boards/stop%2Dratio"]')).to_be_visible()
    fits(page, '/boards')
    page.goto(url + '/boards/stop-ratio')
    expect(page.locator('.board-body dl.facts').first).to_be_visible(timeout=15000)
    expect(page.locator('#board-raw')).not_to_have_attribute('open', '')
    fits(page, '/boards/stop-ratio')
