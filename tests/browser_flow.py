"""Browser check for the flow pages (WP4): /flow and /flow/cycle render
their charts, keep the filters on every flow tab, and fit a 320 px screen
without horizontal page scroll (wide charts scroll inside their own box).

Run by tests/theme.py against the work fixture server.
"""

from playwright.sync_api import expect

RANGE = 'from=2026-09-01&to=2026-11-30'


def no_page_scroll(page, where):
    for width in [1440, 320]:
        page.set_viewport_size({'width': width, 'height': 900})
        assert page.evaluate('document.documentElement.scrollWidth <= innerWidth'), (where, width)
    page.set_viewport_size({'width': 1440, 'height': 900})


def check(page, url):
    response = page.goto(f'{url}/flow?{RANGE}&milestone=v1')
    assert response.status == 200
    expect(page.locator('#flow-throughput svg.chart')).to_have_count(1)
    expect(page.locator('#flow-throughput rect.bar.series-2').first).to_be_attached()
    expect(page.locator('#flow-views a.metric')).to_have_count(5)
    # The table view opens and shows the same buckets.
    page.locator('#flow-throughput-table summary').click()
    expect(page.locator('#flow-throughput-table table')).to_be_visible()
    no_page_scroll(page, '/flow')
    # At 320 px the chart scrolls inside its own box.
    page.set_viewport_size({'width': 320, 'height': 900})
    box = page.locator('#flow-throughput .chart-scroll')
    assert box.evaluate('el => el.scrollWidth > el.clientWidth'), 'the wide chart scrolls in its box'
    page.set_viewport_size({'width': 1440, 'height': 900})

    # Going to Cycle through the flow tabs keeps the filters.
    page.locator('nav.flow-tabs a', has_text='Cycle and lead').click()
    expect(page).to_have_url(f'{url}/flow/cycle?from=2026%2D09%2D01&to=2026%2D11%2D30&milestone=v1')
    expect(page.locator('.flow-stat').first).to_contain_text('left out: claim time unknown')
    # Choosing a breakdown keeps them too.
    page.locator('nav.flow-by a', has_text='Rig').first.click()
    expect(page).to_have_url(f'{url}/flow/cycle?from=2026%2D09%2D01&to=2026%2D11%2D30&milestone=v1&by=rig')
    expect(page.locator('#flow-breakdown')).to_contain_text('Cycle time by rig')
    no_page_scroll(page, '/flow/cycle')

    # The filter form keeps hidden keys and submits the range as dates.
    page.get_by_label('Type').select_option('task')
    page.locator('form.flow-filters button[type=submit]').click()
    page.wait_for_url('**/flow/cycle?**type=task**')
    params = page.evaluate('Object.fromEntries(new URL(location.href).searchParams)')
    assert params['by'] == 'rig' and params['milestone'] == 'v1', params
    assert (params['from'], params['to']) == ('2026-09-01', '2026-11-30'), params
    expect(page.locator('nav.flow-by[aria-label="Break down by"] a[aria-current]')).to_have_text('Rig')
