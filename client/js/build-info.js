/* Build identity is served by this binary, never inferred from browser assets. */
(function (root) {
  'use strict';
  const normalize = info => info && typeof info === 'object' && !Array.isArray(info) ? info : {};
  function label(info) {
    info = normalize(info);
    const version = typeof info.version === 'string' ? `v${info.version}` : 'Version unknown';
    const commit = /^[0-9a-f]{40}$/.test(info.commit || '') ? info.commit : null;
    const state = info.dirty === true ? ' · dirty' : info.dirty === false ? '' : ' · state unknown';
    return `${version} · ${commit ? commit.slice(0, 7) : 'revision unknown'}${state}`;
  }
  function render(info, scope = document) {
    info = normalize(info);
    for (const element of scope.querySelectorAll('[data-build-info]')) {
      element.textContent = label(info);
      element.title = `Commit: ${info.commit || 'unknown'}; commits since tag: ${Number.isSafeInteger(info.commits_since_tag) ? info.commits_since_tag : 'unknown'}; dirty: ${info.dirty === true ? 'yes' : info.dirty === false ? 'no' : 'unknown'}`;
    }
  }
  let identity = {};
  root.FraydBuild = { label, render: (scope) => render(identity, scope) };
  if (typeof module !== 'undefined') module.exports = { label, render };
  if (typeof document !== 'undefined') {
    fetch('/api/build', { cache: 'no-store' }).then(response => {
      if (!response.ok) throw new Error('Build identity unavailable');
      return response.json();
    }).then(info => { identity = normalize(info); render(identity); }).catch(() => { identity = {}; render(identity); });
  }
})(typeof window === 'undefined' ? globalThis : window);
