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

test('series come with their books flattened in order', async () => {
  const b = (id, t) => ({ id, libraryId: 'lib1', media: { duration: 60, metadata: { title: t, authorName: 'A' } } });
  const api = absClient(async () => reply({ results: [{ id: 's1', name: 'Dune', books: [b('i1', 'Dune'), b('i2', 'Messiah')] }] }), { server: 'https://x.example' });
  const [s] = await api.series('lib1');
  assert.deepEqual([s.name, s.books.map(x => x.title)], ['Dune', ['Dune', 'Messiah']]);
});

test('requests carry the bearer token and books are flattened', async () => {
  let auth;
  const api = absClient(async (u, init) => (auth = init.headers.Authorization, reply({ total: 1, results: [{ id: 'i1', libraryId: 'lib1', addedAt: 1700000000000, media: { duration: 3600, metadata: { title: 'Emma', authorName: 'Jane Austen' } } }] })), { server: 'https://x.example', token: 'tok' });
  assert.deepEqual(await api.items('lib1'), { total: 1, books: [{ id: 'i1', libraryId: 'lib1', title: 'Emma', author: 'Jane Austen', duration: 3600, added: 1700000000000 }] });
  assert.equal(auth, 'Bearer tok');
});

test('renaming a series patches its name', async () => {
  let seen;
  const api = absClient(async (url, init) => (seen = { url, init }, reply({})), { server: 'https://x.example', token: 't' });
  await api.renameSeries('s1', 'Dune Chronicles');
  assert.equal(seen.url, 'https://x.example/api/series/s1');
  assert.equal(seen.init.method, 'PATCH');
  assert.deepEqual(JSON.parse(seen.init.body), { name: 'Dune Chronicles' });
});

test('details flatten the people, series and progress of one book', async () => {
  const api = absClient(async () => reply({ media: { duration: 60, size: 9, chapters: [{}, {}], metadata: { title: 'Dune', authors: [{ name: 'Frank Herbert' }], narrators: ['S. Brick'], series: [{ id: 's1', name: 'Dune', sequence: '1' }], publishedYear: '1965' } }, userMediaProgress: { progress: 0.5, isFinished: false } }), { server: 'https://x.example' });
  const d = await api.details('i1');
  assert.deepEqual([d.title, d.authors, d.narrators, d.series, d.chapters, d.progress], ['Dune', ['Frank Herbert'], ['S. Brick'], [{ id: 's1', name: 'Dune', sequence: '1' }], 2, 0.5]);
});

test('stats flatten per-book listening time and finished books', async () => {
  const api = absClient(async url => reply(url.endsWith('/api/me') ? { mediaProgress: [{ libraryItemId: 'i1', isFinished: true, finishedAt: 5 }, { libraryItemId: 'i2', isFinished: false }] } : { days: { '2026-11-12': 60 }, items: { i1: { id: 'i1', timeListening: 90, mediaMetadata: { title: 'Emma', authors: [{ name: 'Jane Austen' }] } } } }), { server: 'https://x.example' });
  assert.deepEqual(await api.stats(), { days: { '2026-11-12': 60 }, items: [{ id: 'i1', title: 'Emma', author: 'Jane Austen', secs: 90 }], finished: [{ id: 'i1', at: 5 }] });
});

test('mark as finished, bookmarks and finished-as-complete progress', async () => {
  const seen = [];
  const api = absClient(async (url, init = {}) => {
    seen.push({ url, method: init.method, body: init.body && JSON.parse(init.body) });
    return reply(url.endsWith('/api/me') ? { mediaProgress: [{ libraryItemId: 'a', progress: 0.4 }, { libraryItemId: 'b', progress: 0.9, isFinished: true }], bookmarks: [{ libraryItemId: 'a', time: 90, title: 'Late' }, { libraryItemId: 'a', time: 10, title: 'Early' }, { libraryItemId: 'z', time: 5, title: 'Other' }] } : {});
  }, { server: 'https://x.example' });
  assert.deepEqual(await api.progress(), { a: 0.4, b: 1 });
  assert.deepEqual((await api.bookmarks('a')).map(b => b.title), ['Early', 'Late']);
  seen.length = 0;
  await api.setFinished('a', true);
  await api.addBookmark('a', 42, 'Ch 3');
  await api.removeBookmark('a', 42);
  assert.deepEqual(seen, [
    { url: 'https://x.example/api/me/progress/a', method: 'PATCH', body: { isFinished: true } },
    { url: 'https://x.example/api/me/item/a/bookmark', method: 'POST', body: { time: 42, title: 'Ch 3' } },
    { url: 'https://x.example/api/me/item/a/bookmark/42', method: 'DELETE', body: undefined }
  ]);
});
