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
  items: [],
  acct: { address: store.get('session', null)?.server ?? '', username: '', password: '', showPw: false, busy: false, error: '' }
};
const client = () => absClient(tfetch, state.session);

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

const card = (b, pct) => `<div class="book"><div class="cover" aria-hidden="true" style="background:${tint(b.title)}">${esc(b.title)}<img class="cover-img" src="${esc(client().coverUrl(b.id))}" alt="" loading="lazy"></div>
<div class="col" style="gap:2px"><div class="strong">${esc(b.title)}</div><div class="cap muted">${esc(b.author)}${b.duration ? ' · ' + len(b.duration) : ''}</div></div>${pct > 0 && pct < 1 ? bar(pct) : ''}</div>`;
const grid = (books, pct = {}) => books.map(b => card(b, pct[b.id])).join('') || '<p class="muted">Nothing here yet</p>';

// ---- views ----
const NAV = ['home', 'library', 'settings'];
const sidebar = route => `<nav aria-label="Main"><div class="logo">Lectern</div>
<div class="navlinks">${NAV.map(r => `<a href="#/${r}"${r === route ? ' aria-current="page"' : ''}>${svg(ICON[r], 20)}${r[0].toUpperCase() + r.slice(1)}</a>`).join('')}</div>
${state.libs?.length ? `<div class="col g8" style="padding:0 8px"><div class="label muted">Libraries</div>${state.libs.map(l => `<button class="lnk" id="lib-${esc(l.id)}" data-act="lib" data-v="${esc(l.id)}" aria-pressed="${l.id === state.libId}">${esc(l.name)}</button>`).join('')}</div>` : ''}
<div class="navfoot"><button class="btn" id="theme" data-act="theme" aria-label="Switch to ${state.theme === 'dark' ? 'light' : 'dark'} theme">${svg(ICON.theme, 20)}Theme: ${state.theme === 'dark' ? 'Dark' : 'Light'}</button>
<div class="cap muted">${state.session ? 'Signed in to ' + esc(state.session.server) : 'Not signed in'}</div></div></nav>`;

const views = {};
views.home = async () => {
  if (!state.libId) return '<h1>Home</h1><p class="muted">No libraries found on this server</p>';
  const api = client();
  const [cont, pct, recent] = await Promise.all([api.inProgress(), api.progress(), api.items(state.libId, { limit: 10, sort: 'addedAt', desc: 1 })]);
  return `<h1>Home</h1><section class="col g16"><h2>Continue listening</h2><div class="grid">${grid(cont, pct)}</div></section>
<section class="col g16"><h2>Recently added</h2><div class="grid">${grid(recent.books, pct)}</div></section>`;
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
  return `<h1>Settings</h1><div class="set"><section class="col g12"><h2>Server connection</h2><div class="card panel">${body}</div></section></div>`;
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

async function loadLibs() {
  try {
    state.libs = await client().libraries();
    if (!state.libs.some(l => l.id === state.libId)) state.libId = state.libs[0]?.id ?? null;
    store.set('lib', state.libId);
  } catch (e) { if (e.status === 401) signOut('Error: Your session expired. Sign in again.'); }
}

function signOut(error = '') {
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

const act = {
  theme: () => { state.theme = state.theme === 'dark' ? 'light' : 'dark'; store.set('theme', state.theme); },
  showpw: () => { state.acct.showPw = !state.acct.showPw; },
  signout: () => { signOut(); return true; },
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
