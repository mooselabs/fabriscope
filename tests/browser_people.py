"""Browser check for activity and people (WP3): the timeline filters by
event kind through its form, /people classes factory seats apart from
people, a seat's page resolves through its percent-encoded name, and each
page fits a 320 px screen.

Run by tests/theme.py against the work fixture server.
"""

from playwright.sync_api import expect

SINCE = 'from=2026-10-01'


def fits(page, where):
    for width in [1440, 320]:
        page.set_viewport_size({'width': width, 'height': 900})
        assert page.evaluate('document.documentElement.scrollWidth <= innerWidth'), (where, width)
    page.set_viewport_size({'width': 1440, 'height': 900})


def check(page, url):
    # The timeline, newest first, narrowed by the event-kind select.
    page.goto(f'{url}/activity?{SINCE}')
    rows = page.locator('ol.activity-list > li')
    assert rows.count() >= 8, rows.count()
    times = rows.locator('time').evaluate_all('els => els.map(e => e.getAttribute("datetime"))')
    assert times == sorted(times, reverse=True), times
    expect(page.get_by_text('Looks right to me {mostly}; check the **header row**.')).to_be_visible()
    fits(page, '/activity')
    page.locator('select[name=event]').select_option('suspended')
    page.locator('form.filters button[type=submit]').click()
    assert 'event=suspended' in page.url, page.url
    expect(rows).to_have_count(1)
    expect(rows.first.locator('.badge')).to_have_text('Held')

    # People and seats in separate tables; a seat's page through its link.
    page.goto(f'{url}/people?{SINCE}')
    expect(page.locator('#class-person a[href="/person/alice"]')).to_have_count(1)
    expect(page.locator('#class-person a[href="/person/bob"]')).to_have_count(1)
    seat = page.locator('#class-seat a[href="/person/factory%2FhostA%2FrigA"]')
    expect(seat).to_have_count(1)
    fits(page, '/people')
    seat.click()
    expect(page.locator('h1')).to_have_text('factory/hostA/rigA')
    expect(page.locator('#person-groups a[href="/by/rig/rigA"]')).to_have_count(1)
    expect(page.locator('#person-activity ol.activity-list > li').first).to_be_visible()
    fits(page, '/person/factory%2FhostA%2FrigA')

    # A person's page.
    page.goto(f'{url}/person/alice?{SINCE}')
    expect(page.locator('h1')).to_have_text('alice')
    expect(page.locator('#person-groups')).to_have_count(0)
    fits(page, '/person/alice')
