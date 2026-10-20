/* Run before the stylesheet so a saved choice applies before first paint. */
(() => {
  const key = 'fabriscope.theme';
  const preference = value => ['light', 'dark'].includes(value) ? value : 'system';
  const apply = value => {
    const theme = preference(value);
    document.documentElement.dataset.theme = theme;
    const control = document.getElementById('color-theme');
    if (control) control.value = theme;
  };
  let saved;
  try { saved = localStorage.getItem(key); } catch (_) { /* Storage may be disabled. */ }
  apply(saved);

  document.addEventListener('DOMContentLoaded', () => {
    const control = document.getElementById('color-theme');
    control.value = document.documentElement.dataset.theme;
    control.closest('.theme-control').hidden = false;
    control.addEventListener('change', () => {
      apply(control.value);
      try { localStorage.setItem(key, control.value); } catch (_) { /* Keep this page usable. */ }
    });
  });
  window.addEventListener('storage', event => {
    if (event.key === key || event.key === null) apply(event.newValue);
  });
})();
