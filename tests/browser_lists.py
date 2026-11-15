"""Browser check for WP1 (lists, groups, search, overview) against the work
fixture: held work shows under way, the bead list filters by derived state
and group, a milestone shows its lanes, search finds and redirects, and every
page fits a 320 px screen.

Run by tests/theme.py, which calls `check(page, url)`.
"""

from playwright.sync_api import expect


def fits(page, where):
    for width in [1440, 320]:
        page.set_viewport_size({'width': width, 'height': 900})
        assert page.evaluate('document.documentElement.scrollWidth <= innerWidth'), (where, width)
    page.set_viewport_size({'width': 1440, 'height': 900})


def check(page, url):
    page.goto(url + '/')
    expect(page.get_by_role('heading', name='Under way')).to_be_visible()
    held = page.locator('#in-progress tr.stopped')
    expect(held).to_have_count(1)
    expect(held.locator('.stop-line')).to_contain_text('scope-exceeded')
    expect(page.locator('a.metric[href="/beads?dstate=held"] strong')).to_have_text('1')
    expect(page.locator('a[href="/beads?ready=yes"]').first).to_be_visible()
    expect(page.locator('#by-group a[href="/by/milestone/v1"]')).to_be_visible()
    fits(page, '/')

    page.goto(url + '/beads')
    page.locator('select[name="dstate"]').select_option('held')
    page.locator('select[name="area"]').select_option('reports')
    page.get_by_role('button', name='Apply').click()
    page.wait_for_url(lambda u: 'dstate=held' in u and 'area=reports' in u)
    rows = page.locator('table.items tbody tr')
    expect(rows).to_have_count(1)
    expect(rows.first).to_contain_text('WORK-0003')
    expect(rows.first.locator('.badge.bad')).to_have_text('Held')
    rows.first.locator('.chip a', has_text='reports').click()
    expect(page).to_have_url(url + '/by/area/reports')
    fits(page, '/by/area/reports')
    page.goto(url + '/beads?state=all')
    expect(page.locator('table.items tbody tr')).to_have_count(8)
    fits(page, '/beads')

    page.goto(url + '/by/milestone')
    expect(page.locator('.group-tabs a[aria-current="page"]')).to_have_text('Milestone')
    page.locator('table.group-values a', has_text='v1').click()
    expect(page).to_have_url(url + '/by/milestone/v1')
    expect(page.locator('#lanes tbody tr')).to_have_count(2)
    expect(page.locator('#members table.items tbody tr')).to_have_count(2)
    fits(page, '/by/milestone/v1')

    page.goto(url + '/search?q=reports')
    expect(page.locator('#beads')).to_contain_text('Move reports to the new table')
    expect(page.locator('#groups a[href="/by/area/reports"]')).to_be_visible()
    fits(page, '/search')
    page.goto(url + '/search?q=WORK-0003')
    page.wait_for_url(lambda u: u.startswith(url + '/bead/wk'))
    expect(page.locator('h1')).to_contain_text('Move reports to the new table')
