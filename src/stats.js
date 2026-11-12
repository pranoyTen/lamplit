// Turns the server's listening stats into the numbers and bars the Stats page shows.
// data: { days: {'YYYY-MM-DD': seconds}, items: [{ id, title, author, secs }], finished: [{ id, at (ms) }] }
const pad = n => String(n).padStart(2, '0');
const dayKey = d => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

// range: 'week' (last 7 days), 'month' (this calendar month) or 'year' (this calendar year), all up to today
function summarize(data, range, now = new Date()) {
  const y = now.getFullYear(), m = now.getMonth(), d0 = now.getDate();
  const ago = i => new Date(y, m, d0 - i);
  const sec = d => data.days[dayKey(d)] || 0;
  let bars, from, span;
  if (range === 'week') {
    bars = [6, 5, 4, 3, 2, 1, 0].map(i => ({ label: ago(i).toLocaleDateString('en', { weekday: 'short' }), secs: sec(ago(i)) }));
    from = ago(6); span = 7;
  } else if (range === 'month') {
    bars = Array.from({ length: d0 }, (_, i) => ({ label: String(i + 1), secs: sec(new Date(y, m, i + 1)) }));
    from = new Date(y, m, 1); span = d0;
  } else {
    bars = MONTHS.slice(0, m + 1).map((label, i) => ({ label, secs: Object.entries(data.days).filter(([k]) => k.startsWith(`${y}-${pad(i + 1)}-`)).reduce((n, [, s]) => n + s, 0) }));
    from = new Date(y, 0, 1); span = Math.round((new Date(y, m, d0) - from) / 864e5) + 1;
  }
  const time = bars.reduce((n, b) => n + b.secs, 0);
  let streak = 0;
  for (let i = sec(ago(0)) > 0 ? 0 : 1; sec(ago(i)) > 0; i++) streak++;
  const byAuthor = {};
  for (const it of data.items) if (it.author) byAuthor[it.author] = (byAuthor[it.author] || 0) + it.secs;
  const authors = Object.entries(byAuthor).map(([name, secs]) => ({ name, secs })).sort((a, b) => b.secs - a.secs).slice(0, 5);
  return { time, avg: time / span, streak, bars, authors, finishedCount: data.finished.filter(f => f.at >= from.getTime() && f.at <= now.getTime()).length };
}
if (typeof module !== 'undefined') module.exports = { summarize };
