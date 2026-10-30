/* Build identity is served by this binary, never inferred from browser assets. */
(function () {
  'use strict';
  const normalize = info => info && typeof info === 'object' && !Array.isArray(info) ? info : {};
  let identity = {};
  function label(info) {
    const version = typeof info.version === 'string' ? `v${info.version}` : 'Version unknown';
    const commit = /^[0-9a-f]{40}$/.test(info.commit || '') ? info.commit : null;
    const state = info.dirty === true ? ' · dirty' : info.dirty === false ? '' : ' · state unknown';
    return `${version} · ${commit ? commit.slice(0, 7) : 'revision unknown'}${state}`;
  }
  /** Write the fetched identity as text into every [data-build-info] node in scope. */
  function render(scope = document) {
    for (const element of scope.querySelectorAll('[data-build-info]')) {
      element.textContent = label(identity);
      const sinceTag = Number.isSafeInteger(identity.commits_since_tag) ? identity.commits_since_tag : 'unknown';
      const dirty = identity.dirty === true ? 'yes' : identity.dirty === false ? 'no' : 'unknown';
      element.title = `Commit: ${identity.commit || 'unknown'}; commits since tag: ${sinceTag}; dirty: ${dirty}`;
    }
  }
  window.FraydBuild = { render };
  fetch('/api/build', { cache: 'no-store' }).then(response => {
    if (!response.ok) throw new Error('Build identity unavailable');
    return response.json();
  }).then(info => { identity = normalize(info); render(); }).catch(() => { identity = {}; render(); });
})();
