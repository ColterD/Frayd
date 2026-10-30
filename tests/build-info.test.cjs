'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { join } = require('node:path');
const vm = require('node:vm');

const source = readFileSync(join(__dirname, '..', 'client', 'js', 'build-info.js'), 'utf8');
const unknown = 'Version unknown · revision unknown · state unknown';

// Run the production script against one /api/build response, then render one element.
async function rendered(body, { ok = true } = {}) {
  const context = vm.createContext({
    document: { querySelectorAll: () => [] },
    fetch: async () => ({ ok, json: async () => body }),
  });
  context.window = context;
  vm.runInContext(source, context);
  await new Promise(resolve => setImmediate(resolve));
  const element = {};
  context.FraydBuild.render({ querySelectorAll: () => [element] });
  return element;
}

test('binary revision, dirty and unavailable provenance remain distinct', async () => {
  assert.equal((await rendered({ version: '0.1.1', commit: 'a'.repeat(40), dirty: false })).textContent, 'v0.1.1 · aaaaaaa');
  assert.match((await rendered({ version: '0.1.1', commit: 'a'.repeat(40), dirty: true })).textContent, / · dirty$/);
  assert.equal((await rendered({})).textContent, unknown);
  assert.equal((await rendered({ version: '0.1.1' }, { ok: false })).textContent, unknown);
});

test('untrusted metadata uses textContent and full identity tooltip', async () => {
  const element = await rendered({ version: '<img onerror=bad>', commit: 'b'.repeat(40), commits_since_tag: 131, dirty: false });
  assert.match(element.textContent, /<img onerror=bad>/);
  assert.match(element.title, new RegExp('b'.repeat(40)));
  assert.match(element.title, /131; dirty: no/);
});

test('null, arrays and primitive API shapes cannot break late Settings rendering', async () => {
  for (const info of [null, [], 12, 'unexpected', false]) {
    const element = await rendered(info);
    assert.equal(element.textContent, unknown);
    assert.match(element.title, /Commit: unknown/);
  }
});
