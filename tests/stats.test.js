const test = require('node:test');
const assert = require('node:assert/strict');
const { summarize } = require('../src/stats.js');

const now = new Date(2026, 10, 12, 15); // Thu 12 Nov 2026
const data = {
  days: { '2026-11-12': 3600, '2026-11-11': 1800, '2026-11-10': 600, '2026-11-08': 900, '2026-03-05': 7200 },
  items: [{ id: 'a', title: 'Emma', author: 'Jane Austen', secs: 5000 }, { id: 'b', title: 'Persuasion', author: 'Jane Austen', secs: 1000 }, { id: 'c', title: 'Dracula', author: 'Bram Stoker', secs: 2000 }, { id: 'd', title: 'Untitled', author: '', secs: 9000 }],
  finished: [{ id: 'a', at: new Date(2026, 10, 3).getTime() }, { id: 'c', at: new Date(2026, 2, 9).getTime() }, { id: 'b', at: new Date(2025, 5, 1).getTime() }]
};

test('week covers the last seven days, streak counts back from today', () => {
  const s = summarize(data, 'week', now);
  assert.equal(s.bars.length, 7);
  assert.equal(s.time, 3600 + 1800 + 600 + 900);
  assert.equal(s.streak, 3);
  assert.equal(s.bars.at(-1).secs, 3600);
});

test('month has one bar per day so far and averages over those days', () => {
  const s = summarize(data, 'month', now);
  assert.equal(s.bars.length, 12);
  assert.equal(s.avg, 6900 / 12);
  assert.equal(s.finishedCount, 1);
});

test('year sums per month and counts books finished this year only', () => {
  const s = summarize(data, 'year', now);
  assert.deepEqual([s.bars.length, s.bars[2].secs, s.bars[10].secs], [11, 7200, 6900]);
  assert.equal(s.finishedCount, 2);
});

test('authors are totalled across books, ranked, and blank authors skipped', () => {
  assert.deepEqual(summarize(data, 'year', now).authors, [{ name: 'Jane Austen', secs: 6000 }, { name: 'Bram Stoker', secs: 2000 }]);
});

test('a streak survives an empty today if yesterday was listened to', () => {
  const s = summarize({ ...data, days: { '2026-11-11': 60, '2026-11-10': 60 } }, 'week', now);
  assert.equal(s.streak, 2);
});
