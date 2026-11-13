const test = require('node:test');
const assert = require('node:assert/strict');
const { makeDownloads } = require('../src/downloads.js');

const bytes = (...n) => Uint8Array.from(n);
// a server that honours Range (or ignores it when ranges = false) and records every request
const server = (data, { ranges = true, failOnCall = 0 } = {}) => {
  const calls = [];
  const fetchFn = async (url, init = {}) => {
    calls.push(init.headers?.Range);
    if (calls.length === failOnCall) throw new Error('network down');
    const body = r => ({ getReader: () => { let sent = false; return { read: async () => sent ? { done: true } : (sent = true, { done: false, value: r }) }; } });
    const m = /bytes=(\d+)-(\d+)/.exec(init.headers?.Range ?? '');
    return m && ranges ? { ok: true, status: 206, body: body(data[url].slice(+m[1], +m[2] + 1)) } : { ok: true, status: 200, body: body(data[url]) };
  };
  return { fetchFn, calls };
};
const setup = (fetchFn, { limit = 0, chunk = 4, sizes = [5, 3] } = {}) => {
  const files = {}, removed = [];
  const fs = {
    mkdir: async () => {},
    open: async (path, o) => {
      if (o.truncate || !files[path]) files[path] = [];
      let pos = 0;
      return { seek: async n => { pos = n; }, write: async b => { for (const x of b) files[path][pos++] = x; }, close: async () => {} };
    },
    remove: async p => { removed.push(p); },
    stat: async p => { if (!files[p]) throw new Error('missing'); return { size: files[p].length }; }
  };
  const api = () => ({ item: async () => ({ duration: 20, chapters: [], tracks: [{ url: 'u1', ext: '.mp3', size: sizes[0], duration: 10, start: 0 }, { url: 'u2', ext: '.mp3', size: sizes[1], duration: 10, start: 10 }] }) });
  const dl = makeDownloads({ fetchFn, fs, base: 14, cfg: () => ({ root: 'downloads', abs: false, limit }), api, urlFor: async f => 'asset://' + f, load: () => null, save: () => {}, chunk });
  return { dl, files, removed };
};
const settled = dl => new Promise(r => { const t = setInterval(() => { if (dl.list().every(i => i.status !== 'queued' && i.status !== 'active')) { clearInterval(t); r(); } }, 5); });
const DATA = { u1: bytes(1, 2, 3, 4, 5), u2: bytes(6, 7, 8) };

test('a queued book downloads every file in chunks and becomes playable offline', async () => {
  const { fetchFn, calls } = server(DATA);
  const { dl, files } = setup(fetchFn);
  dl.add({ id: 'b1', title: 'Emma', author: 'Jane Austen' });
  await settled(dl);
  assert.equal(dl.get('b1').status, 'done');
  assert.deepEqual(calls, ['bytes=0-3', 'bytes=4-4', 'bytes=0-2']);
  assert.deepEqual(files, { 'downloads/b1/0.mp3': [1, 2, 3, 4, 5], 'downloads/b1/1.mp3': [6, 7, 8] });
  assert.equal(dl.bytes(), 8);
  const s = await dl.session('b1', 12);
  assert.deepEqual(s.tracks.map(t => [t.url, t.start]), [['asset://downloads/b1/0.mp3', 0], ['asset://downloads/b1/1.mp3', 10]]);
});

test('a download cut off mid-way resumes from the last saved byte', async () => {
  const { fetchFn, calls } = server(DATA, { failOnCall: 2 }); // dies when asking for the second chunk
  const { dl, files } = setup(fetchFn);
  dl.add({ id: 'b1', title: 'Emma', author: 'A' });
  await settled(dl);
  assert.equal(dl.get('b1').status, 'error');
  assert.equal(dl.get('b1').tracks[0].got, 4);
  dl.retry('b1');
  await settled(dl);
  assert.equal(dl.get('b1').status, 'done');
  assert.deepEqual(calls.slice(0, 3), ['bytes=0-3', 'bytes=4-4', 'bytes=4-4']); // retried from byte 4, not 0
  assert.deepEqual(files['downloads/b1/0.mp3'], [1, 2, 3, 4, 5]);
});

test('a server that ignores Range still delivers the whole file', async () => {
  const { fetchFn } = server(DATA, { ranges: false });
  const { dl, files } = setup(fetchFn);
  dl.add({ id: 'b1', title: 'Emma', author: 'A' });
  await settled(dl);
  assert.equal(dl.get('b1').status, 'done');
  assert.deepEqual(files['downloads/b1/0.mp3'], [1, 2, 3, 4, 5]);
  assert.equal(dl.bytes(), 8);
});

test('a server error marks the book failed and retry queues it again', async () => {
  let fail = true;
  const ok = server(DATA).fetchFn;
  const { dl } = setup(async (u, i) => fail ? { ok: false, status: 503 } : ok(u, i));
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
  const { dl, removed } = setup(server(DATA).fetchFn);
  dl.add({ id: 'b1', title: 'Emma', author: 'A' });
  await settled(dl);
  await dl.remove('b1');
  assert.deepEqual(removed, ['downloads/b1']);
  assert.equal(dl.get('b1'), undefined);
});

test('a book that would pass the storage limit fails with a clear message', async () => {
  const { dl } = setup(server(DATA).fetchFn, { limit: 6 });
  dl.add({ id: 'b1', title: 'Emma', author: 'A' });
  await settled(dl);
  assert.equal(dl.get('b1').status, 'error');
  assert.match(dl.get('b1').error, /storage limit/);
});

test('a file-system failure is reported as such, not as a network problem', async () => {
  const { fetchFn } = server(DATA);


  const fs2 = { mkdir: async () => {}, open: async () => { return { seek: async () => { throw 'fs.seek not allowed'; }, write: async () => {}, close: async () => {} }; }, remove: async () => {}, stat: async () => ({ size: 1e9 }) };
  const api = () => ({ item: async () => ({ duration: 10, chapters: [], tracks: [{ url: 'u1', ext: '.mp3', size: 5, duration: 10, start: 0 }] }) });
  const d = makeDownloads({ fetchFn, fs: fs2, base: 14, cfg: () => ({ root: 'downloads', abs: false, limit: 0 }), api, urlFor: async f => f, load: () => null, save: () => {}, chunk: 4 });
  d.add({ id: 'b2', title: 'X', author: 'A' });
  await settled(d);
  assert.equal(d.get('b2').status, 'error');
  assert.match(d.get('b2').error, /Couldn't write the file: fs.seek not allowed/);
});

test('a file shorter than the saved byte count resumes from its real end, leaving no hole', async () => {
  const { fetchFn } = server(DATA, { failOnCall: 2 });
  const { dl, files } = setup(fetchFn);
  dl.add({ id: 'b1', title: 'Emma', author: 'A' });
  await settled(dl);
  files['downloads/b1/0.mp3'].length = 2; // the disk kept less than we counted
  dl.retry('b1');
  await settled(dl);
  assert.equal(dl.get('b1').status, 'done');
  assert.deepEqual(files['downloads/b1/0.mp3'], [1, 2, 3, 4, 5]);
});

test('onDone fires once when a book finishes', async () => {
  const { fetchFn } = server(DATA);
  const done = [];
  const files = {};
  const fs = { mkdir: async () => {}, open: async p => ({ seek: async () => {}, write: async () => {}, close: async () => {} }), remove: async () => {}, stat: async () => ({ size: 1e9 }) };
  const api = () => ({ item: async () => ({ duration: 10, chapters: [], tracks: [{ url: 'u1', ext: '.mp3', size: 5, duration: 10, start: 0 }] }) });
  const d = makeDownloads({ fetchFn, fs, base: 14, cfg: () => ({ root: 'downloads', abs: false, limit: 0 }), api, urlFor: async f => f, load: () => null, save: () => {}, onDone: id => done.push(id), chunk: 4 });
  void files;
  d.add({ id: 'b1', title: 'X', author: 'A' });
  await settled(d);
  assert.deepEqual(done, ['b1']);
});

test('a failing size check stops the download with a disk error instead of restarting it', async () => {
  const { fetchFn } = server(DATA);
  const fs2 = { mkdir: async () => {}, open: async () => ({ seek: async () => {}, write: async () => {}, close: async () => {} }), remove: async () => {}, stat: async () => { throw 'denied'; } };
  const api = () => ({ item: async () => ({ duration: 10, chapters: [], tracks: [{ url: 'u1', ext: '.mp3', size: 5, duration: 10, start: 0 }] }) });
  const d = makeDownloads({ fetchFn, fs: fs2, base: 14, cfg: () => ({ root: 'downloads', abs: false, limit: 0 }), api, urlFor: async f => f, load: () => null, save: () => {}, chunk: 4 });
  d.add({ id: 'b3', title: 'X', author: 'A' });
  await settled(d);
  assert.equal(d.get('b3').status, 'error');
  assert.match(d.get('b3').error, /Couldn't write the file: denied/);
});

test('a finished book whose files lost bytes is not offered for offline play', async () => {
  const { fetchFn } = server(DATA);
  const { dl, files } = setup(fetchFn);
  dl.add({ id: 'b1', title: 'Emma', author: 'A' });
  await settled(dl);
  assert.ok(await dl.session('b1', 0));
  files['downloads/b1/0.mp3'].length = 3;
  assert.equal(await dl.session('b1', 0), null);
});

test('the queue waits while downloads are not allowed and starts when they are', async () => {
  const { fetchFn } = server(DATA);
  let ok = false;
  const fs = { mkdir: async () => {}, open: async () => ({ seek: async () => {}, write: async () => {}, close: async () => {} }), remove: async () => {}, stat: async () => ({ size: 1e9 }) };
  const api = () => ({ item: async () => ({ duration: 10, chapters: [], tracks: [{ url: 'u1', ext: '.mp3', size: 5, duration: 10, start: 0 }] }) });
  const d = makeDownloads({ fetchFn, fs, base: 14, cfg: () => ({ root: 'downloads', abs: false, limit: 0 }), api, urlFor: async f => f, load: () => null, save: () => {}, allowed: () => ok, chunk: 4 });
  d.add({ id: 'b5', title: 'X', author: 'A' });
  await new Promise(r => setTimeout(r, 30));
  assert.equal(d.get('b5').status, 'queued');
  ok = true;
  d.pump();
  await settled(d);
  assert.equal(d.get('b5').status, 'done');
});
