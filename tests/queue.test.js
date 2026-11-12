const test = require('node:test');
const assert = require('node:assert/strict');
const { makeListenQueue } = require('../src/queue.js');

const make = send => { let saved = null; const q = makeListenQueue({ load: () => null, save: v => { saved = JSON.parse(JSON.stringify(v)); }, send }); return { q, saved: () => saved }; };

test('time adds up per book and the place is the latest', () => {
  const { q, saved } = make(async () => {});
  q.add('b1', { currentTime: 10, timeListened: 15, duration: 100 });
  q.add('b1', { currentTime: 40, timeListened: 20, duration: 100 });
  q.add('b2', { currentTime: 5, timeListened: 5, duration: 50 });
  assert.deepEqual(saved(), { b1: { currentTime: 40, duration: 100, timeListened: 35 }, b2: { currentTime: 5, duration: 50, timeListened: 5 } });
});

test('flush sends each book once and empties the queue; a failure stays queued', async () => {
  const sent = [];
  const { q, saved } = make(async (id, e) => { if (id === 'b2') throw new Error('offline'); sent.push([id, e.timeListened]); });
  q.add('b1', { currentTime: 1, timeListened: 30, duration: 9 });
  q.add('b2', { currentTime: 2, timeListened: 40, duration: 9 });
  await q.flush();
  assert.deepEqual(sent, [['b1', 30]]);
  assert.deepEqual(Object.keys(saved()), ['b2']);
});

test('the book being played is skipped, and a book gone from the server is dropped', async () => {
  const sent = [];
  const { q } = make(async id => { if (id === 'gone') throw Object.assign(new Error('x'), { status: 404 }); sent.push(id); });
  for (const id of ['now', 'gone', 'b3']) q.add(id, { currentTime: 1, timeListened: 1, duration: 9 });
  await q.flush('now');
  assert.deepEqual(sent, ['b3']);
  assert.equal(q.size(), 1); // only 'now' is left
});

test('time added while a send is in flight is kept', async () => {
  let q;
  const made = make(async () => { q.add('b1', { currentTime: 9, timeListened: 7, duration: 9 }); });
  q = made.q;
  q.add('b1', { currentTime: 1, timeListened: 10, duration: 9 });
  await q.flush();
  assert.deepEqual(made.saved().b1, { currentTime: 9, duration: 9, timeListened: 7 });
});
