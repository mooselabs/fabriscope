"""Browser check for the bead page (WP2): a landed packet (wk-2) and a held
one (wk-3) render their banner, attempts, cost, runs, receipts and timeline,
fit a 320 px screen without sideways scrolling, and keep an opened
`<details>` open across a live refresh.

Run by tests/theme.py, which calls `check(page, url)` against the work
fixture server.
"""

from playwright.sync_api import TimeoutError as PlaywrightTimeout, expect


def fits(page, where):
    for width in [1440, 320]:
        page.set_viewport_size({'width': width, 'height': 900})
        assert page.evaluate('document.documentElement.scrollWidth <= innerWidth'), (where, width)
    page.set_viewport_size({'width': 1440, 'height': 900})


def check(page, url):
    page.goto(url + '/bead/wk-3')
    banner = page.locator('section.banner#banner-held')
    expect(banner).to_be_visible()
    expect(banner).to_contain_text('scope-exceeded (scope)')
    expect(banner.locator('dl.facts dt', has_text='attempt')).to_have_count(1)
    expect(banner).to_contain_text('run-0003')
    expect(page.locator('.record-meta')).not_to_contain_text('Ready, as bd ready decides')
    expect(page.locator('#timeline ol.timeline > li').first).to_be_visible()
    fits(page, 'wk-3')

    page.goto(url + '/bead/wk-2')
    expect(page.locator('#progress h3', has_text='Attempts · 2')).to_be_visible()
    expect(page.locator('#attempt-1 code', has_text='att-0002-1')).to_be_visible()
    expect(page.locator('#attempt-2 code', has_text='att-0002-2')).to_be_visible()
    expect(page.locator('#cost dt', has_text='Ledger usage')).to_be_visible()
    expect(page.locator('#cost dt', has_text='Run feed')).to_be_visible()
    expect(page.locator('#runs h2')).to_have_text('Runs · 2')
    expect(page.locator('#receipts h2')).to_have_text('Receipts · 3')
    expect(page.locator('#panel-packet')).to_be_visible()
    fits(page, 'wk-2')

    # An opened <details> stays open when a live refresh swaps #content.
    # Asking for a bead whose provenance has not been read starts a read;
    # its result bumps the revision, the SSE stream announces it and app.js
    # replaces #content.
    log = page.locator('details#provenance-log')
    log.locator('summary').click()
    expect(log).to_have_attribute('open', '')
    revision = page.locator('#content').get_attribute('data-revision')
    for other in ['wk-q1', 'wk-4', 'wk-5', 'wk-e1', 'wk-m1', 'wk-slot', 'wk-op-2']:
        page.request.get(url + '/bead/' + other)
        try:
            page.wait_for_function(
                'r => document.getElementById("content").dataset.revision !== r',
                arg=revision, timeout=5000)
            break
        except PlaywrightTimeout:
            continue
    else:
        raise AssertionError('no live refresh reached the page')
    expect(page.locator('details#provenance-log')).to_have_attribute('open', '')
    expect(page.locator('details#metadata')).not_to_have_attribute('open', '')
