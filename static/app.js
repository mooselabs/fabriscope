/* Live HTML replacement keeps navigation, filters, open groups and scroll. */
(() => {
  const status = document.getElementById('connection');
  let pending = false;
  let busy = false;
  let connected = false;
  const editing = () => /^(INPUT|SELECT|TEXTAREA)$/.test(document.activeElement?.tagName || '');
  async function refresh() {
    if (!pending || busy || editing() || document.hidden) return;
    pending = false;
    busy = true;
    try {
      const response = await fetch(location.href, {cache: 'no-store'});
      if (!response.ok && response.status !== 404) throw new Error('Read failed');
      const next = new DOMParser().parseFromString(await response.text(), 'text/html');
      const content = next.getElementById('content');
      if (!content) throw new Error('Missing page');
      const old = document.getElementById('content');
      const expanded = new Map([...old.querySelectorAll('details[id]')].map(el => [el.id, el.open]));
      for (const el of content.querySelectorAll('details[id]')) {
        if (expanded.has(el.id)) el.open = expanded.get(el.id);
      }
      const y = window.scrollY;
      old.replaceWith(content);
      document.title = next.title;
      window.scrollTo(0, y);
      status.textContent = connected ? '● Live' : 'Reconnecting…';
    } catch (_) {
      status.textContent = 'Update failed · retrying';
      pending = true;
    } finally { busy = false; }
  }
  const events = new EventSource('/events');
  events.onopen = () => { connected = true; status.textContent = '● Live'; };
  events.onmessage = event => {
    if (event.data !== document.getElementById('content').dataset.revision) {
      pending = true;
      if (editing()) status.textContent = 'Update ready';
      refresh();
    }
  };
  events.onerror = () => { connected = false; status.textContent = 'Reconnecting…'; };
  document.addEventListener('focusout', () => setTimeout(refresh, 100));
  document.addEventListener('visibilitychange', refresh);
  setInterval(refresh, 3000);
})();
