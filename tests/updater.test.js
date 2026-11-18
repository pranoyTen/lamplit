const test = require('node:test');
const assert = require('node:assert/strict');
const { makeUpdater, explainUpdateError } = require('../src/updater.js');

const run = (over = {}) => {
  const states = [], calls = [];
  const u = makeUpdater({ token: () => 'tok', onState: s => states.push(s), relaunch: async () => calls.push('relaunch'), check: async o => { calls.push(o); return null; }, ...over });
  return { u, states, calls };
};

test('without a token it says so and never calls the server', async () => {
  const { u, states, calls } = run({ token: () => '' });
  await u.check();
  assert.deepEqual(states, [{ s: 'notoken' }]);
  assert.equal(calls.length, 0);
});

test('up to date: the token goes out as a bearer header', async () => {
  const { u, states, calls } = run();
  await u.check();
  assert.deepEqual(states.map(s => s.s), ['checking', 'current']);
  assert.deepEqual(calls[0], { headers: { Authorization: 'Bearer tok' } });
});

test('an update is reported with its version and notes', async () => {
  const { u, states } = run({ check: async () => ({ version: '1.2.0', body: 'Fixes' }) });
  await u.check();
  assert.deepEqual(states.at(-1), { s: 'available', version: '1.2.0', notes: 'Fixes' });
});

test('install shows progress, sends the token again, then restarts', async () => {
  const seen = [];
  const found = { version: '1.2.0', downloadAndInstall: async (on, o) => { seen.push(o); on({ event: 'Started', data: { contentLength: 100 } }); on({ event: 'Progress', data: { chunkLength: 40 } }); on({ event: 'Progress', data: { chunkLength: 60 } }); on({ event: 'Finished' }); } };
  const { u, states, calls } = run({ check: async () => found });
  await u.check();
  await u.install();
  assert.deepEqual(states.filter(s => s.s === 'downloading').map(s => s.pct), [0, 0.4, 1]);
  assert.deepEqual(seen[0], { headers: { Authorization: 'Bearer tok' } });
  assert.equal(calls.at(-1), 'relaunch');
});

test('a failed download reports the error and does not restart', async () => {
  const found = { version: '1.2.0', downloadAndInstall: async () => { throw new Error('signature mismatch'); } };
  const { u, states, calls } = run({ check: async () => found });
  await u.check();
  await u.install();
  assert.equal(states.at(-1).s, 'error');
  assert.ok(!calls.includes('relaunch'));
});

test('errors are put in plain words', () => {
  assert.match(explainUpdateError(new Error('request failed with status 404')), /token is valid/);
  assert.match(explainUpdateError('error sending request for url'), /connection/);
  assert.match(explainUpdateError(new Error('boom')), /boom/);
});
