const test = require('node:test');
const assert = require('node:assert/strict');
const { makeDownloads } = require('../src/downloads.js');

const stream = chunks => ({ ok: true, body: { getReader: () => { const q = [...chunks]; return { read: async () => q.length ? { done: false, value: q.shift() } : { done: true } }; } } });
const setup = (fetchFn, limit = 0) => {
  const written = {}, removed = [];
  let saved;
  const fs = {
    mkdir: async () => {},
    open: async path => ({ write: async b => { written[path] = (written[path] ?? 0) + b.length; }, close: async () => {} }),
    remove: async p => { removed.push(p); }
  };
  const api = () => ({ item: async () => ({ duration: 20, chapters: [], tracks: [{ url: 'u1', ext: '.mp3', size: 5, duration: 10, start: 0 }, { url: 'u2', ext: '.mp3', size: 3, duration: 10, start: 10 }] }) });
  const dl = makeDownloads({ fetchFn, fs, base: 14, cfg: () => ({ root: 'downloads', abs: false, limit }), api, urlFor: async f => 'asset://' + f, load: () => null, save: s => { saved = s; } });
  return { dl, written, removed, saved: () => saved };
};
const settled = dl => new Promise(r => { const t = setInterval(() => { if (dl.list().every(i => i.status !== 'queued' && i.status !== 'active')) { clearInterval(t); r(); } }, 5); });

test('a queued book downloads every file and becomes playable offline', async () => {
  const { dl, written } = setup(async url => stream(url === 'u1' ? [new Uint8Array(5)] : [new Uint8Array(2), new Uint8Array(1)]));
  dl.add({ id: 'b1', title: 'Emma', author: 'Jane Austen' });
  await settled(dl);
  assert.equal(dl.get('b1').status, 'done');
  assert.deepEqual(written, { 'downloads/b1/0.mp3': 5, 'downloads/b1/1.mp3': 3 });
  assert.equal(dl.bytes(), 8);
  const s = await dl.session('b1', 12);
  assert.deepEqual(s.tracks.map(t => [t.url, t.start]), [['asset://downloads/b1/0.mp3', 0], ['asset://downloads/b1/1.mp3', 10]]);
});

test('a server error marks the book failed and retry queues it again', async () => {
  let fail = true;
  const { dl } = setup(async () => fail ? { ok: false, status: 503 } : stream([new Uint8Array(5)]));
  dl.add({ id: 'b1', title: 'Emma', author: 'A' });
  await settled(dl);
  assert.equal(dl.get('b1').status, 'error');
  assert.match(dl.get('b1').error, /503/);
  fail = false;
  dl.retry('b1');
  await settled(dl);
  assert.equal(dl.get('b1').status, 'done');
});

test('removing a book deletes its folder', async () => {
  const { dl, removed } = setup(async () => stream([new Uint8Array(5)]));
  dl.add({ id: 'b1', title: 'Emma', author: 'A' });
  await settled(dl);
  await dl.remove('b1');
  assert.deepEqual(removed, ['downloads/b1']);
  assert.equal(dl.get('b1'), undefined);
});

test('a book that would pass the storage limit fails with a clear message', async () => {
  const { dl } = setup(async () => stream([new Uint8Array(5)]), 6);
  dl.add({ id: 'b1', title: 'Emma', author: 'A' });
  await settled(dl);
  assert.equal(dl.get('b1').status, 'error');
  assert.match(dl.get('b1').error, /storage limit/);
});
