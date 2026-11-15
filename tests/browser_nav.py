"""Browser check for the shared navigation (WP0): every tab of the work
fixture answers, marks itself current, and fits a 320 px screen.

Run by tests/theme.py, which calls `check(page, url)` against the work
fixture server. Copy this shape for a package's own `browser_<area>.py`.
"""

from playwright.sync_api import expect


def check(page, url):
    page.goto(url + '/')
    sections = page.locator('header nav[aria-label="Main navigation"] a')
    expect(sections).to_have_count(4)  # Overview, Work, Flow, Factory
    tabs = [url + '/']
    for href in sections.evaluate_all('els => els.map(e => e.getAttribute("href"))'):
        page.goto(url + href)
        tabs += [url + h for h in page.locator('.subnav a').evaluate_all(
            'els => els.map(e => e.getAttribute("href"))')]
    assert len(set(tabs)) == 17, tabs
    for tab in dict.fromkeys(tabs):
        response = page.goto(tab)
        assert response.status == 200, tab
        expect(page.locator('header nav a[aria-current="page"]')).to_have_count(1)
        for width in [1440, 320]:
            page.set_viewport_size({'width': width, 'height': 900})
            assert page.evaluate('document.documentElement.scrollWidth <= innerWidth'), (tab, width)
        page.set_viewport_size({'width': 1440, 'height': 900})
    # The search box sends its words to /search.
    page.goto(url + '/beads')
    page.get_by_label('Search everything').fill('csv')
    page.get_by_label('Search everything').press('Enter')
    expect(page).to_have_url(url + '/search?q=csv')
