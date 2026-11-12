// Lectern: Home, Library and Settings (server connection) against an Audiobookshelf server.
const store = {
  get(k, d) { try { return JSON.parse(localStorage.getItem('lectern.' + k)) ?? d; } catch { return d; } },
  set(k, v) { try { localStorage.setItem('lectern.' + k, JSON.stringify(v)); } catch {} }
};
const tfetch = window.__TAURI__?.http?.fetch ?? fetch.bind(window);

const state = {
  theme: store.get('theme', 'light'),
  // shortcut: token sits in localStorage, move to the OS keychain before sharing builds
  session: store.get('session', null),
  libs: null,
  libId: store.get('lib', null),
  hidden: store.get('hidden', []), // ids of libraries switched off in Settings
  items: [],
  acct: { address: store.get('session', null)?.server ?? '', username: '', password: '', showPw: false, busy: false, error: '' }
};
const client = () => absClient(tfetch, state.session);
const shown = () => (state.libs ?? []).filter(l => !state.hidden.includes(l.id));

const esc = s => String(s).replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
const svg = (d, s = 24) => `<svg width="${s}" height="${s}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${d}</svg>`;
const ICON = {
  home: '<path d="M3 10.5 12 3l9 7.5V20a1 1 0 0 1-1 1h-5v-6H9v6H4a1 1 0 0 1-1-1z"/>',
  library: '<path d="M4 19.5V5a2 2 0 0 1 2-2h13v14H6a2 2 0 0 0-2 2.5zM4 19.5A2 2 0 0 0 6 21h13"/>',
  settings: '<path d="M4 7h10M18 7h2M4 17h2M10 17h10"/><circle cx="16" cy="7" r="2"/><circle cx="8" cy="17" r="2"/>',
  search: '<circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/>',
  theme: '<circle cx="12" cy="12" r="8"/><path d="M12 4a8 8 0 0 1 0 16z" fill="currentColor"/>'
};
const len = s => { const h = Math.floor(s / 3600), m = Math.floor(s % 3600 / 60); return h ? `${h}h ${m}m` : `${m}m`; };
const tint = t => `hsl(${[...t].reduce((a, c) => (a * 31 + c.charCodeAt(0)) % 360, 7)} 30% 28%)`;
const bar = pct => `<div class="bar thin"><i style="width:${Math.round(pct * 100)}%"></i></div>`;

const card = (b, pct) => `<div class="book"><button class="${coverCls(b.id)}" data-act="play" data-v="${esc(b.id)}" data-t="${esc(b.title)}" data-a="${esc(b.author)}" aria-label="Play ${esc(b.title)}" style="background:${tint(b.title)}">${esc(b.title)}<img class="cover-img" src="${esc(client().coverUrl(b.id))}" alt="" loading="lazy"><span class="ov" aria-hidden="true">${svg(PL.play, 40)}${svg(PL.pause, 40)}</span></button>
<div class="col" style="gap:2px"><div class="strong">${esc(b.title)}</div><div class="cap muted">${esc(b.author)}${b.duration ? ' · ' + len(b.duration) : ''}</div></div>${pct > 0 && pct < 1 ? bar(pct) : ''}</div>`;
const grid = (books, pct = {}) => books.map(b => card(b, pct[b.id])).join('') || '<p class="muted">Nothing here yet</p>';

// ---- views ----
const NAV = ['home', 'library', 'settings'];
const sidebar = route => `<nav aria-label="Main"><div class="logo">Lectern</div>
<div class="navlinks">${NAV.map(r => `<a href="#/${r}"${r === route ? ' aria-current="page"' : ''}>${svg(ICON[r], 20)}${r[0].toUpperCase() + r.slice(1)}</a>`).join('')}</div>
${shown().length ? `<div class="col g8" style="padding:0 8px"><div class="label muted">Libraries</div>${shown().map(l => `<button class="lnk" id="lib-${esc(l.id)}" data-act="lib" data-v="${esc(l.id)}" aria-pressed="${l.id === state.libId}">${esc(l.name)}</button>`).join('')}</div>` : ''}
<div class="navfoot"><button class="btn" id="theme" data-act="theme" aria-label="Switch to ${state.theme === 'dark' ? 'light' : 'dark'} theme">${svg(ICON.theme, 20)}Theme: ${state.theme === 'dark' ? 'Dark' : 'Light'}</button>
<div class="cap muted">${state.session ? 'Signed in to ' + esc(state.session.server) : 'Not signed in'}</div></div></nav>`;

const views = {};
views.home = async () => {
  const libs = shown();
  if (!libs.length) return '<h1>Home</h1><p class="muted">No libraries found on this server</p>';
  const api = client();
  const [cont, pct, ...recent] = await Promise.all([api.inProgress(), api.progress(), ...libs.map(l => api.items(l.id, { limit: 10, sort: 'addedAt', desc: 1 }))]);
  const ids = new Set(libs.map(l => l.id));
  return `<h1>Home</h1><section class="col g16"><h2>Continue listening</h2><div class="grid">${grid(cont.filter(b => ids.has(b.libraryId)), pct)}</div></section>
${libs.map((l, i) => `<section class="col g16"><h2>Recently added${libs.length > 1 ? ' in ' + esc(l.name) : ''}</h2><div class="grid">${grid(recent[i].books, pct)}</div></section>`).join('')}`;
};

views.library = async () => {
  if (!state.libId) return '<h1>Library</h1><p class="muted">No libraries found on this server</p>';
  const { total, books } = await client().items(state.libId, { limit: 200, sort: 'media.metadata.title' });
  state.items = books;
  return `<div class="row"><div class="col g4"><h1>${esc(state.libs.find(l => l.id === state.libId)?.name ?? 'Library')}</h1><div class="muted">${total} books${total > books.length ? ` · showing the first ${books.length}` : ''}</div></div>
<div class="search"><svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" aria-hidden="true" style="color:var(--muted)">${ICON.search}</svg><input id="q" type="search" aria-label="Search this library" placeholder="Search" style="width:240px"></div></div>
<div class="grid lib" id="grid">${grid(books)}</div>`;
};

const setrow = (label, desc, id, ctl) => `<div class="setrow field"><div class="info"><label for="${id}">${label}</label>${desc ? `<div class="cap muted">${desc}</div>` : ''}</div>${ctl}</div>`;
const settings = () => {
  const a = state.acct, s = state.session;
  const body = s
    ? `<div class="setrow"><div class="row g16" style="flex:1 1 240px;flex-wrap:nowrap;align-items:center;justify-content:flex-start"><div class="avatar" aria-hidden="true">${esc(s.username[0]?.toUpperCase() ?? '?')}</div><div style="min-width:0"><div class="strong">Signed in as ${esc(s.username)}</div><div class="cap muted">${esc(s.server)}</div></div></div><div class="label accent">Connected</div></div>
<div class="setrow"><div class="info"><div class="strong">Switch account or server</div></div><button class="btn" id="signout" data-act="signout">Sign out</button></div>`
    : `<form id="login">${setrow('Server address', 'Where your Audiobookshelf server is hosted, including http:// or https://', 'server', `<input id="server" type="text" autocomplete="url" placeholder="https://library.example.com" value="${esc(a.address)}" data-f="address">`)}
${setrow('Username', 'The account set up in Audiobookshelf', 'user', `<input id="user" type="text" autocomplete="username" placeholder="Username" value="${esc(a.username)}" data-f="username">`)}
${setrow('Password', '', 'pass', `<div class="row g8" style="align-items:center"><input id="pass" type="${a.showPw ? 'text' : 'password'}" autocomplete="current-password" placeholder="Password" value="${esc(a.password)}" data-f="password"><button type="button" class="btn" id="showpw" data-act="showpw" aria-pressed="${a.showPw}">${a.showPw ? 'Hide' : 'Show'}</button></div>`)}
${a.error ? `<div role="alert" class="danger" style="padding:12px 0;border-top:1px solid var(--hairline)">${esc(a.error)}</div>` : ''}
<div class="setrow" style="justify-content:flex-start"><button type="submit" class="btn primary" id="signin"${a.busy ? ' disabled' : ''}>${a.busy ? 'Signing in…' : 'Sign in'}</button><div class="cap muted">Use the same username and password as the Audiobookshelf web app</div></div></form>`;
  const libs = s && state.libs?.length
    ? `<section class="col g12"><h2>Libraries</h2><div class="card panel">${state.libs.map(l => {
      const on = !state.hidden.includes(l.id);
      return setrow(esc(l.name), on ? 'Shown in the sidebar and on Home' : 'Hidden', 'sl-' + esc(l.id), `<input type="checkbox" id="sl-${esc(l.id)}" data-act="togglelib" data-v="${esc(l.id)}"${on ? ' checked' : ''}${on && shown().length < 2 ? ' disabled' : ''}>`);
    }).join('')}</div></section>` : '';
  return `<h1>Settings</h1><div class="set"><section class="col g12"><h2>Server connection</h2><div class="card panel">${body}</div></section>${libs}</div>`;
};

// ---- render ----
let seq = 0;
const failure = e => `<div role="alert" class="danger">Error: ${e.status ? `The server answered ${e.status}.` : `Can't reach ${esc(state.session.server)}. Check the address or your connection.`} <a href="#/settings" style="text-decoration:underline">Check Settings</a></div>`;

async function render() {
  const n = ++seq, r = NAV.includes(location.hash.slice(2)) ? location.hash.slice(2) : (state.session ? 'home' : 'settings');
  document.documentElement.dataset.theme = state.theme;
  if (state.session && !state.libs) await loadLibs();
  if (n !== seq) return;
  const loading = state.session && r !== 'settings';
  const focus = document.activeElement?.id;
  document.getElementById('app').innerHTML = `<div class="shell">${sidebar(r)}<main id="main">${r === 'settings' ? settings() : loading ? '<p class="muted">Loading…</p>' : '<h1>Welcome</h1><p class="muted">Connect to your server in <a href="#/settings" style="text-decoration:underline">Settings</a> to see your books.</p>'}</main></div>`;
  if (focus) document.getElementById(focus)?.focus();
  if (!loading) return;
  let html;
  try { html = await views[r](); } catch (e) { if (e.status === 401) return signOut('Error: Your session expired. Sign in again.'); html = failure(e); }
  if (n === seq) document.getElementById('main').innerHTML = html;
}

// the Library page needs a selected library that is still switched on
function pickLib() {
  if (!shown().some(l => l.id === state.libId)) state.libId = shown()[0]?.id ?? null;
  store.set('lib', state.libId);
}

async function loadLibs() {
  try {
    state.libs = await client().libraries();
    pickLib();
  } catch (e) { if (e.status === 401) signOut('Error: Your session expired. Sign in again.'); }
}

function signOut(error = '') {
  stopPlayer();
  state.session = null; state.libs = null; state.items = [];
  store.set('session', null);
  Object.assign(state.acct, { password: '', error });
  location.hash === '#/settings' ? render() : (location.hash = '#/settings');
}

async function signIn() {
  const a = state.acct;
  if (a.busy) return;
  if (!a.address.trim() || !a.username.trim() || !a.password) { a.error = 'Error: Enter your server address, username and password.'; return render(); }
  a.busy = true; a.error = ''; render();
  try {
    state.session = await absClient(tfetch, { server: a.address }).login(a.username.trim(), a.password);
    store.set('session', state.session);
    a.password = '';
    location.hash = '#/home';
  } catch (e) {
    a.error = e.badAddress ? 'Error: ' + e.message
      : e.status === 401 ? 'Error: Wrong username or password.'
      : e.status ? `Error: The server answered ${e.status}.`
      : `Error: Can't reach ${a.address.trim()}. Check the address or your connection.`;
  }
  a.busy = false;
  render();
}

// ---- player: lives outside #app so re-renders never interrupt playback ----
// All times below are seconds from the start of the book; a book may be split over several files (tracks).
const player = { id: null, s: null, i: 0, listened: 0, last: 0, rate: 1 };
const RATES = [1, 1.25, 1.5, 1.75, 2, 0.75];
const audio = () => document.getElementById('audio');
const position = () => player.s.tracks[player.i].start + audio().currentTime;
const clock = t => { t = Math.max(0, Math.floor(t)); const h = Math.floor(t / 3600), m = Math.floor(t % 3600 / 60), s = t % 60; return (h ? h + ':' + String(m).padStart(2, '0') : m) + ':' + String(s).padStart(2, '0'); };
const skipIcon = (n, back) => svg(`<path d="${back ? 'M4 12a8 8 0 1 1 2.5 5.8M4 5v5h5' : 'M20 12a8 8 0 1 0-2.5 5.8M20 5v5h-5'}"/><text x="12" y="15" font-size="8" text-anchor="middle" fill="currentColor" stroke="none">${n}</text>`, 24);
const PL = {
  prev: '<path d="M6 5v14M19 5 9 12l10 7z"/>', next: '<path d="M18 5v14M5 5l10 7-10 7z"/>',
  list: '<path d="M9 6h11M9 12h11M9 18h11M4 6h.01M4 12h.01M4 18h.01"/>',
  play: '<path d="M8 5v14l11-7z" fill="currentColor"/>', pause: '<path d="M8 5v14M16 5v14" stroke-width="2.5"/>'
};

function syncPlayer(close = false) {
  if (!player.s) return;
  const body = { currentTime: position(), timeListened: Math.round(player.listened), duration: player.s.duration };
  player.listened = 0;
  return client().sync(player.s.sessionId, body, close).catch(() => {});
}
function stopPlayer() {
  if (player.s && audio()) syncPlayer(true);
  player.s = null; player.id = null;
  document.getElementById('player').hidden = true;
  document.getElementById('player').innerHTML = '';
  paintCovers();
}
// covers show a play/pause overlay: "now" is the loaded book, "playing" is audio actually running
const coverCls = id => 'cover' + (player.id === id ? ' now' + (player.s && audio() && !audio().paused ? ' playing' : '') : '');
function paintCovers() {
  document.querySelectorAll('.cover[data-v]').forEach(c => {
    c.className = coverCls(c.dataset.v);
    c.setAttribute('aria-label', (c.classList.contains('playing') ? 'Pause ' : 'Play ') + c.dataset.t);
  });
}
const trackAt = (s, t) => Math.max(0, s.tracks.findLastIndex(k => t >= k.start));
const chapterAt = (s, t) => s.chapters.findIndex(c => t >= c.start && t < c.end);

function seek(t) {
  const s = player.s, a = audio();
  t = Math.min(Math.max(0, t), Math.max(0, s.duration - 1));
  const i = trackAt(s, t), at = t - s.tracks[i].start;
  if (i === player.i) { a.currentTime = at; return; }
  const playing = !a.paused;
  player.i = i; player.last = 0; a.src = s.tracks[i].url;
  a.addEventListener('loadedmetadata', () => { a.currentTime = at; if (playing) a.play(); }, { once: true });
}
function chapterStep(dir) {
  const s = player.s, t = position(), c = chapterAt(s, t);
  if (c < 0) return;
  if (dir > 0) { if (s.chapters[c + 1]) seek(s.chapters[c + 1].start); }
  else seek(t - s.chapters[c].start > 3 || !s.chapters[c - 1] ? s.chapters[c].start : s.chapters[c - 1].start);
}
function toggleMenu(open) {
  const m = document.getElementById('np-menu'), b = document.getElementById('np-chapbtn');
  if (!m) return;
  m.hidden = !(open ?? m.hidden);
  b.setAttribute('aria-expanded', String(!m.hidden));
  if (!m.hidden) m.querySelector('[aria-current]')?.scrollIntoView({ block: 'center' });
}
const ctl = {
  chapters: () => toggleMenu(),
  prev: () => chapterStep(-1), next: () => chapterStep(1),
  back: () => seek(position() - 15), fwd: () => seek(position() + 30),
  toggle: () => { const a = audio(); a.paused ? a.play() : a.pause(); },
  rate: () => {
    player.rate = RATES[(RATES.indexOf(player.rate) + 1) % RATES.length];
    audio().defaultPlaybackRate = audio().playbackRate = player.rate;
    document.getElementById('np-rate').textContent = player.rate + '×';
  }
};

function paintPlayer() {
  const s = player.s, a = audio();
  if (!s || !a) return;
  const t = position(), c = chapterAt(s, t);
  document.getElementById('np-seek').value = Math.floor(t);
  document.getElementById('np-time').textContent = `${clock(t)} / ${clock(s.duration)}`;
  document.querySelectorAll('#np-menu [data-ch]').forEach((b, i) => i === c ? b.setAttribute('aria-current', 'true') : b.removeAttribute('aria-current'));
  document.getElementById('np-toggle').innerHTML = svg(a.paused ? PL.play : PL.pause, 24);
  document.getElementById('np-toggle').setAttribute('aria-label', a.paused ? 'Play' : 'Pause');
  paintCovers();
}

async function startPlayer({ v: id, t: title, a: author }) {
  stopPlayer();
  player.id = id; paintCovers();
  const el = document.getElementById('player');
  el.hidden = false;
  el.innerHTML = `<div class="info"><div class="strong">${esc(title)}</div><div class="cap muted" id="np-status">Loading…</div></div>`;
  const status = m => { const n = document.getElementById('np-status'); if (n) n.textContent = m; };
  try {
    const s = await client().play(id);
    if (!s.tracks.length) throw new Error('no audio');
    s.author = author;
    player.s = s; player.i = trackAt(s, s.start); player.listened = 0; player.last = 0;
    const ib = (k, label, icon, cls = '') => `<button class="ib ${cls}" data-k="${k}" id="np-${k}" aria-label="${label}"${s.chapters.length || !/prev|next/.test(k) ? '' : ' disabled'}>${icon}</button>`;
    el.innerHTML = `<div class="info"><div class="strong">${esc(title)}</div><div class="cap muted" id="np-status">${esc(author)}</div></div>
<div class="np-ctl">${ib('prev', 'Previous chapter', svg(PL.prev, 24))}${ib('back', 'Back 15 seconds', skipIcon(15, true))}${ib('toggle', 'Pause', svg(PL.pause, 24), 'main')}${ib('fwd', 'Forward 30 seconds', skipIcon(30, false))}${ib('next', 'Next chapter', svg(PL.next, 24))}<button class="ib txt" data-k="rate" id="np-rate" aria-label="Playback speed">${player.rate}×</button></div>
${s.chapters.length ? `<div class="chap"><button class="btn sm" data-k="chapters" id="np-chapbtn" aria-haspopup="true" aria-expanded="false">${svg(PL.list, 20)}Chapters</button><div class="menu card" id="np-menu" role="menu" hidden>${s.chapters.map((c, i) => `<button role="menuitem" data-ch="${i}"><span>${esc(c.title)}</span><span class="cap muted">${clock(c.start)}</span></button>`).join('')}</div></div>` : ''}
<div class="np-seek"><span class="cap muted" id="np-time"></span><input type="range" id="np-seek" min="0" max="${Math.floor(s.duration)}" step="1" value="0" aria-label="Position in book"></div>
<audio id="audio" autoplay src="${esc(s.tracks[player.i].url)}"></audio>`;
    const a = audio();
    a.defaultPlaybackRate = a.playbackRate = player.rate;
    a.addEventListener('loadedmetadata', () => { if (player.last === 0 && s.start) a.currentTime = Math.max(0, s.start - s.tracks[player.i].start); }, { once: true });
    a.addEventListener('timeupdate', () => {
      const d = a.currentTime - player.last;
      if (d > 0 && d < 2) player.listened += d;
      player.last = a.currentTime;
      if (player.listened >= 15) syncPlayer();
      if (!el.querySelector('#np-seek:active')) paintPlayer();
    });
    a.addEventListener('play', paintPlayer);
    a.addEventListener('pause', () => { paintPlayer(); syncPlayer(); });
    a.addEventListener('ended', () => {
      if (player.i + 1 < s.tracks.length) { player.i++; player.last = 0; a.src = s.tracks[player.i].url; a.play(); } else syncPlayer();
    });
    a.addEventListener('error', () => status("Can't play this file. The server may be sending a format this app can't decode."));
    el.onclick = e => {
      const ch = e.target.closest('[data-ch]');
      if (ch) { seek(s.chapters[ch.dataset.ch].start); toggleMenu(false); } else ctl[e.target.closest('[data-k]')?.dataset.k]?.();
    };
    el.querySelector('#np-seek').oninput = e => { document.getElementById('np-time').textContent = `${clock(e.target.value)} / ${clock(s.duration)}`; };
    el.querySelector('#np-seek').onchange = e => seek(+e.target.value);
    document.onclick = e => { if (!e.target.closest('.chap')) toggleMenu(false); };
    el.onkeydown = e => { if (e.key === 'Escape') { toggleMenu(false); document.getElementById('np-chapbtn')?.focus(); } };
    paintPlayer();
  } catch (e) {
    player.id = null; paintCovers();
    status(e.status === 401 ? 'Your session expired. Sign in again.' : e.status ? `The server answered ${e.status}.` : "Can't start playback. Check your connection.");
  }
}
window.addEventListener('beforeunload', () => syncPlayer());

const act = {
  // same book: toggle pause; another book: start it; ignored while the session is still opening
  play: d => { if (player.id !== d.v) startPlayer(d); else if (player.s) ctl.toggle(); return true; },
  theme: () => { state.theme = state.theme === 'dark' ? 'light' : 'dark'; store.set('theme', state.theme); },
  showpw: () => { state.acct.showPw = !state.acct.showPw; },
  signout: () => { signOut(); return true; },
  togglelib: d => {
    const off = state.hidden.includes(d.v);
    if (!off && shown().length < 2) return; // keep at least one library on
    state.hidden = off ? state.hidden.filter(x => x !== d.v) : [...state.hidden, d.v];
    store.set('hidden', state.hidden);
    pickLib();
  },
  lib: d => { state.libId = d.v; store.set('lib', d.v); }
};
document.addEventListener('click', e => {
  const t = e.target.closest('[data-act]');
  if (t && !act[t.dataset.act](t.dataset)) render();
});
document.addEventListener('submit', e => { e.preventDefault(); signIn(); });
document.addEventListener('input', e => {
  if (e.target.dataset.f) state.acct[e.target.dataset.f] = e.target.value;
  if (e.target.id === 'q') {
    const q = e.target.value.toLowerCase();
    document.getElementById('grid').innerHTML = grid(state.items.filter(b => (b.title + ' ' + b.author).toLowerCase().includes(q)));
  }
});
document.addEventListener('error', e => e.target.classList?.contains('cover-img') && e.target.remove(), true);
window.addEventListener('hashchange', render);
render();
