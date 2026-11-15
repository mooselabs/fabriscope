"""Browser check for the factory pages (WP6): /held, /costs and /receipts
render on the work fixture, their controls keep the URL as the state, and
each fits a 320 px screen without horizontal scrolling.

Run by tests/theme.py, which calls `check(page, url)` against the work
fixture server.
"""

import re

from playwright.sync_api import expect


def fits(page, where):
    for width in [1440, 320]:
        page.set_viewport_size({'width': width, 'height': 900})
        assert page.evaluate('document.documentElement.scrollWidth <= innerWidth'), (where, width)
    page.set_viewport_size({'width': 1440, 'height': 900})


def check(page, url):
    page.goto(url + '/held')
    expect(page.locator('h1')).to_have_text('Held')
    expect(page.locator('#held-held')).to_contain_text('Move reports to the new table')
    expect(page.locator('#stop-history')).to_contain_text('Stop history')
    expect(page.locator('.subnav a[aria-current="page"]')).to_have_text('Held')
    fits(page, '/held')
    # Group by the cause instead of the stop code.
    page.locator('form.held-controls select[name="by"]').select_option('work.suspended.cause')
    page.locator('form.held-controls button').click()
    expect(page).to_have_url(re.compile(r'/held\?by=work\.suspended\.cause(&|$)'))
    expect(page.locator('#held-held-scope')).to_be_visible()
    fits(page, '/held?by=work.suspended.cause')

    page.goto(url + '/costs?from=2026-11-01&to=2026-11-30')
    expect(page.locator('h1')).to_have_text('Costs')
    expect(page.locator('#coverage')).to_contain_text('Cost known for')
    expect(page.locator('#cost-by-model')).to_be_visible()
    fits(page, '/costs')
    page.locator('#cost-items th a', has_text='Tokens').click()
    expect(page).to_have_url(url + '/costs?from=2026%2D11%2D01&to=2026%2D11%2D30&sort=tokens#cost-items')
    fits(page, '/costs?sort=tokens')

    page.goto(url + '/receipts')
    expect(page.locator('h1')).to_have_text('Receipts')
    expect(page.locator('#receipt-list tbody tr')).to_have_count(4)
    fits(page, '/receipts')
    page.locator('form.filters select[name="kind"]').select_option('gate')
    page.locator('form.filters button').click()
    expect(page.locator('#receipt-list tbody tr')).to_have_count(1)
    expect(page.locator('#receipt-list')).to_contain_text('Fix rounding in invoice totals')
    fits(page, '/receipts?kind=gate')
