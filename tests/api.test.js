const test = require('node:test');
const assert = require('node:assert/strict');
const { absClient } = require('../src/api.js');

const reply = (body, status = 200) => ({ ok: status < 400, status, json: async () => body });

test('login posts credentials and returns the session', async () => {
  let seen;
  const api = absClient(async (url, init) => (seen = { url, init }, reply({ user: { token: 't1', username: 'ann' } })), { server: ' http://abs.local:13378/// ' });
  assert.deepEqual(await api.login('ann', 'pw'), { server: 'http://abs.local:13378', token: 't1', username: 'ann' });
  assert.equal(seen.url, 'http://abs.local:13378/login');
  assert.deepEqual(JSON.parse(seen.init.body), { username: 'ann', password: 'pw' });
});

test('wrong password surfaces the status', async () => {
  const api = absClient(async () => reply({}, 401), { server: 'https://x.example' });
  await assert.rejects(api.login('a', 'b'), { status: 401 });
});

test('address without a scheme is rejected', () => {
  assert.throws(() => absClient(fetch, { server: 'abs.local' }), { badAddress: true });
});

test('play returns tokened track urls and the resume point', async () => {
  const api = absClient(async () => reply({ id: 's1', currentTime: 42, duration: 100, audioTracks: [{ contentUrl: '/api/items/i1/file/9', startOffset: 0, duration: 100 }] }), { server: 'https://x.example', token: 'a b' });
  assert.deepEqual(await api.play('i1'), { sessionId: 's1', start: 42, duration: 100, chapters: [], tracks: [{ url: 'https://x.example/api/items/i1/file/9?token=a%20b', start: 0, duration: 100 }] });
});

test('requests carry the bearer token and books are flattened', async () => {
  let auth;
  const api = absClient(async (u, init) => (auth = init.headers.Authorization, reply({ total: 1, results: [{ id: 'i1', libraryId: 'lib1', media: { duration: 3600, metadata: { title: 'Emma', authorName: 'Jane Austen' } } }] })), { server: 'https://x.example', token: 'tok' });
  assert.deepEqual(await api.items('lib1'), { total: 1, books: [{ id: 'i1', libraryId: 'lib1', title: 'Emma', author: 'Jane Austen', duration: 3600 }] });
  assert.equal(auth, 'Bearer tok');
});
