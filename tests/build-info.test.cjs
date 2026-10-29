'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { label, render } = require('../client/js/build-info.js');
test('binary revision, dirty and unavailable provenance remain distinct', () => {
  assert.equal(label({ version: '0.1.1', commit: 'a'.repeat(40), dirty: false }), 'v0.1.1 · aaaaaaa');
  assert.match(label({ version: '0.1.1', commit: 'a'.repeat(40), dirty: true }), / · dirty$/);
  assert.match(label({}), /revision unknown · state unknown$/);
});
test('untrusted metadata uses textContent and full identity tooltip', () => {
  const element = { set innerHTML(_) { throw new Error('HTML interpretation forbidden'); } };
  const scope = { querySelectorAll: () => [element] };
  render({ version: '<img onerror=bad>', commit: 'b'.repeat(40), commits_since_tag: 131, dirty: false }, scope);
  assert.match(element.textContent, /<img onerror=bad>/);
  assert.match(element.title, new RegExp('b'.repeat(40)));
  assert.match(element.title, /131; dirty: no/);
});

test('null, arrays and primitive API shapes cannot break late Settings rendering', () => {
  for (const info of [null, [], 12, 'unexpected', false]) {
    const element = {};
    render(info, { querySelectorAll: () => [element] });
    assert.equal(element.textContent, label({}));
    assert.match(element.title, /Commit: unknown/);
  }
});
