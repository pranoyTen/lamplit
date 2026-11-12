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
  dlcfg: store.get('dlcfg', { limitGB: 10, dir: null }), // download cap in GB and chosen folder (null = app data)
  statsRange: 'year', stats: null,
  hidden: store.get('hidden', []), // ids of libraries switched off in Settings
  items: [], q: '',
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
  series: '<path d="M5 5h11a2 2 0 0 1 2 2v12H7a2 2 0 0 1-2-2zM9 3h10a2 2 0 0 1 2 2v12"/>',
  stats: '<path d="M5 20V10M12 20V4M19 20v-7"/>',
  downloads: '<path d="M12 3v12m0 0-4-4m4 4 4-4M4 20h16"/>',
  search: '<circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/>',
  theme: '<circle cx="12" cy="12" r="8"/><path d="M12 4a8 8 0 0 1 0 16z" fill="currentColor"/>'
};
const len = s => { const h = Math.floor(s / 3600), m = Math.floor(s % 3600 / 60); return h ? `${h}h ${m}m` : `${m}m`; };
const tint = t => `hsl(${[...t].reduce((a, c) => (a * 31 + c.charCodeAt(0)) % 360, 7)} 30% 28%)`;
const bar = pct => `<div class="bar thin"><i style="width:${Math.round(pct * 100)}%"></i></div>`;

const card = (b, pct, extra = '') => `<div class="book"><button class="${coverCls(b.id)}" data-act="play" data-v="${esc(b.id)}" data-t="${esc(b.title)}" data-a="${esc(b.author)}" aria-label="Play ${esc(b.title)}" style="background:${tint(b.title)}">${esc(b.title)}<img class="cover-img" src="${esc(client().coverUrl(b.id))}" alt="" loading="lazy"><span class="ov" aria-hidden="true">${svg(PL.play, 40)}${svg(PL.pause, 40)}</span></button>
<a class="col btxt" href="#/book/${esc(b.id)}"><div class="strong">${esc(b.title)}</div><div class="cap muted">${esc(b.author)}${b.duration ? ' · ' + len(b.duration) : ''}</div></a>${pct > 0 && pct < 1 ? bar(pct) : ''}${extra}</div>`;
const grid = (books, pct = {}, extra = dlExtra) => books.map(b => card(b, pct[b.id], extra(b))).join('') || '<p class="muted">Nothing here yet</p>';

// ---- views ----
const NAV = ['home', 'library', 'series', 'downloads', 'stats', 'settings'];
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
  state.items = books; state.q = '';
  return `<div class="row"><div class="col g4"><h1>${esc(state.libs.find(l => l.id === state.libId)?.name ?? 'Library')}</h1><div class="muted">${total} books${total > books.length ? ` · showing the first ${books.length}` : ''}</div></div>
<div class="search"><svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" aria-hidden="true" style="color:var(--muted)">${ICON.search}</svg><input id="q" type="search" aria-label="Search this library" placeholder="Search" style="width:240px"></div></div>
<div class="grid lib" id="grid">${grid(libBooks())}</div>`;
};

const libBooks = () => { const q = state.q.toLowerCase(); return state.items.filter(b => (b.title + ' ' + b.author).toLowerCase().includes(q)); };

// ---- series: a list page, and a page per series (read from the server's own metadata) ----
const seriesCard = s => `<a class="book" href="#/series/${esc(s.id)}"><div class="cover" style="background:${tint(s.name)}">${esc(s.name)}${s.books[0] ? `<img class="cover-img" src="${esc(client().coverUrl(s.books[0].id))}" alt="" loading="lazy">` : ''}</div>
<div class="col" style="gap:2px"><div class="strong">${esc(s.name)}</div><div class="cap muted">${s.books.length} ${s.books.length === 1 ? 'book' : 'books'}</div></div></a>`;
// header of one series page, with an inline rename form (renaming needs edit rights on the server)
const serHead = () => {
  const s = state.cur, n = s.books.length;
  return `<a href="#/series" class="cap muted" style="text-decoration:underline">All series</a>
${state.renaming
    ? `<form id="renameser" class="row g8" style="justify-content:flex-start;align-items:center"><input id="sername" name="sername" type="text" value="${esc(s.name)}" maxlength="120" aria-label="Series name" style="width:320px"><button type="submit" class="btn primary sm">Save</button><button type="button" class="btn sm" data-act="rename">Cancel</button></form>`
    : `<div class="row g12" style="justify-content:flex-start;align-items:center"><h1>${esc(s.name)}</h1><button class="btn sm" data-act="rename">Rename</button></div>`}
${state.renameErr ? `<div role="alert" class="danger">${esc(state.renameErr)}</div>` : ''}
<div class="muted">${n} ${n === 1 ? 'book' : 'books'}</div>`;
};
function paintSerHead() {
  const h = document.getElementById('serhead');
  if (!h) return;
  h.innerHTML = serHead();
  document.getElementById('sername')?.select();
}
async function renameSeries(name) {
  name = name.trim();
  if (!name || name === state.cur.name) { state.renaming = false; return paintSerHead(); }
  try {
    await client().renameSeries(state.cur.id, name);
    state.cur.name = name; state.renaming = false; state.renameErr = '';
  } catch (e) {
    state.renameErr = e.status === 403 ? "Error: Your account can't edit series on this server." : e.status ? `Error: The server answered ${e.status}.` : "Error: Can't reach the server. Check your connection.";
  }
  paintSerHead();
}
views.series = async id => {
  if (!state.libId) return '<h1>Series</h1><p class="muted">No libraries found on this server</p>';
  const list = await client().series(state.libId);
  const one = id && list.find(s => s.id === id);
  if (one) { state.cur = one; state.renaming = false; state.renameErr = ''; }
  if (one) return `<div class="col g4" id="serhead">${serHead()}</div>
<div class="grid lib">${grid(one.books)}</div>`;
  return `<h1>Series</h1>${list.length ? `<div class="grid lib">${list.map(seriesCard).join('')}</div>` : '<p class="muted">No series in this library</p>'}`;
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
      return setrow(esc(l.name), on ? (shown().length < 2 ? 'Shown in the sidebar and on Home. At least one library stays on' : 'Shown in the sidebar and on Home') : 'Hidden', 'sl-' + esc(l.id), `<input type="checkbox" id="sl-${esc(l.id)}" data-act="togglelib" data-v="${esc(l.id)}"${on ? ' checked' : ''}${on && shown().length < 2 ? ' disabled' : ''}>`);
    }).join('')}</div></section>` : '';
  const dls = dl ? `<section class="col g12"><h2>Downloads</h2><div class="card panel">${setrow('Storage limit', `${bytes(dl.bytes())} used. Books that would pass it are not downloaded`, 'dllimit', `<div class="row g8" style="align-items:center"><input id="dllimit" type="number" min="1" step="1" value="${state.dlcfg.limitGB}" style="width:96px"><span class="muted">GB</span></div>`)}
${setrow('Download location', `${esc(state.dlcfg.dir ?? 'Lectern app data folder')} · applies to new downloads`, 'dlchoose', `<div class="row g8" style="align-items:center"><button class="btn" id="dlchoose" data-act="dlchoose">Choose folder</button>${state.dlcfg.dir ? '<button class="btn" data-act="dlreset">Use default</button>' : ''}</div>`)}</div></section>` : '';
  return `<h1>Settings</h1><div class="set"><section class="col g12"><h2>Server connection</h2><div class="card panel">${body}</div></section>${libs}${dls}</div>`;
};

// ---- downloads: offline copies in the app data folder (desktop app only) ----
const T = window.__TAURI__;
// the window has no system title bar (decorations are off), so the app draws its own: drag area plus minimize, maximize, close
if (T?.window) {
  const w = T.window.getCurrentWindow(), bar = document.getElementById('titlebar');
  const WIN = { min: '<path d="M6 12h12"/>', max: '<rect x="6" y="6" width="12" height="12" rx="1"/>', restore: '<path d="M9 9V6h9v9h-3"/><rect x="6" y="9" width="9" height="9" rx="1"/>', close: '<path d="M6 6l12 12M18 6 6 18"/>' };
  const btn = (k, label) => `<button class="wbtn${k === 'close' ? ' close' : ''}" data-w="${k}" aria-label="${label}">${svg(WIN[k], 16)}</button>`;
  bar.innerHTML = `<span class="wtitle">Lectern</span><div class="wspace" data-tauri-drag-region></div>${btn('min', 'Minimize')}<button class="wbtn" data-w="max" id="wmax" aria-label="Maximize"></button>${btn('close', 'Close')}`;
  bar.hidden = false;
  bar.setAttribute('data-tauri-drag-region', '');
  const paintMax = async () => { const m = await w.isMaximized(); const b = document.getElementById('wmax'); b.innerHTML = svg(m ? WIN.restore : WIN.max, 16); b.setAttribute('aria-label', m ? 'Restore' : 'Maximize'); };
  bar.onclick = e => { const k = e.target.closest('[data-w]')?.dataset.w; if (k === 'min') w.minimize(); else if (k === 'max') w.toggleMaximize(); else if (k === 'close') w.close(); };
  paintMax();
  w.onResized(paintMax);
}
let appData = '';
T?.path?.appDataDir().then(p => { appData = p; });
let repaint = null;
const dl = T?.fs ? makeDownloads({
  fetchFn: tfetch, fs: T.fs, base: T.path.BaseDirectory.AppData, api: () => client(),
  cfg: () => ({ root: state.dlcfg.dir ?? 'downloads', abs: !!state.dlcfg.dir, limit: state.dlcfg.limitGB * 1e9 }),
  urlFor: async (f, abs) => T.core.convertFileSrc(abs ? f : await T.path.join(appData, f)),
  load: () => store.get('downloads', null), save: v => store.set('downloads', v), onDone: id => swapToLocal(id),
  onChange: () => { repaint ??= setTimeout(() => { repaint = null; paintDownloads(); }, 300); }
}) : null;

const bytes = n => n >= 1e9 ? (n / 1e9).toFixed(1) + ' GB' : n >= 1e6 ? Math.round(n / 1e6) + ' MB' : Math.max(1, Math.round(n / 1e3)) + ' KB';
const pctOf = it => it.total ? Math.min(100, Math.floor(it.bytes / it.total * 100)) : 0;
const dlStatus = it => it.status === 'done' ? 'Downloaded' : it.status === 'active' ? `Downloading ${pctOf(it)}%` : it.status === 'queued' ? 'Queued' : 'Error';
const dlColor = it => it.status === 'error' ? 'danger' : it.status === 'queued' ? 'muted' : 'accent';

// the small control under a book cover: start a download, or link to where it is
const dlExtra = b => {
  if (!dl) return '';
  const it = dl.get(b.id);
  return it ? `<a class="cap muted" href="#/downloads">${dlStatus(it)}</a>` : `<button class="lnk" data-act="dl" data-v="${esc(b.id)}" data-t="${esc(b.title)}" data-a="${esc(b.author)}">Download</button>`;
};

const dlRow = it => `<li><div class="dcov" style="background:${tint(it.title)}"><img class="cover-img" src="${esc(client().coverUrl(it.id))}" alt="" loading="lazy"></div>
<div class="col g4" style="flex:1;min-width:0"><div class="row g12" style="justify-content:flex-start;align-items:baseline"><span class="strong">${esc(it.title)}</span><span class="cap muted">${esc(it.author)}${it.total ? ' · ' + bytes(it.total) : ''}</span></div>
${it.status === 'active' ? `<div class="bar thin" style="max-width:360px" role="progressbar" aria-valuenow="${pctOf(it)}" aria-valuemin="0" aria-valuemax="100"><i style="width:${pctOf(it)}%"></i></div>` : ''}
<div class="label ${dlColor(it)}">${dlStatus(it)}</div>${it.status === 'error' ? `<div class="danger">Error: ${esc(it.error)}</div>` : ''}</div>
${it.status === 'error' ? `<button class="btn primary" data-act="dlretry" data-v="${esc(it.id)}">Retry</button>` : ''}
<button class="ib" data-act="dlremove" data-v="${esc(it.id)}" aria-label="${it.status === 'done' ? 'Remove download of ' : 'Cancel download of '}${esc(it.title)}">${svg('<path d="M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3"/>', 20)}</button></li>`;

const downloadsHtml = () => {
  const all = dl.list(), done = all.filter(i => i.status === 'done').length;
  return `<div class="row"><div class="col g4"><h1>Downloads</h1><div class="muted">${bytes(dl.bytes())} on this computer · ${done} ${done === 1 ? 'book' : 'books'} ready offline</div></div>
<div class="row g12" style="flex-wrap:nowrap"><button class="btn" data-act="dlpause">${dl.paused ? 'Resume all' : 'Pause all'}</button><button class="btn" data-act="dlclean"${done ? '' : ' disabled'}>Remove finished books</button></div></div>
${state.dlcfg.limitGB ? `<section class="col g8" style="max-width:720px"><div class="row" style="flex-wrap:nowrap"><span class="strong">Offline storage</span><span class="muted">${bytes(dl.bytes())} of ${state.dlcfg.limitGB} GB limit</span></div><div class="bar" role="progressbar" aria-label="Offline storage used" aria-valuenow="${Math.min(100, Math.round(dl.bytes() / (state.dlcfg.limitGB * 1e9) * 100))}" aria-valuemin="0" aria-valuemax="100"><i style="width:${Math.min(100, dl.bytes() / (state.dlcfg.limitGB * 1e9) * 100)}%"></i></div></section>` : ''}
<section class="col g12" style="max-width:960px"><h2>Queue</h2>${all.length ? `<ul class="card dl">${all.map(dlRow).join('')}</ul>` : '<div class="col g4"><div class="strong">Nothing downloaded yet</div><div class="cap muted">Use Download under a book to save it for offline listening</div></div>'}</section>`;
};
function paintDownloads() {
  const m = document.getElementById('dlpage');
  if (m) m.innerHTML = downloadsHtml();
}
views.downloads = async () => dl
  ? `<div id="dlpage" class="col" style="gap:24px">${downloadsHtml()}</div>`
  : '<h1>Downloads</h1><p class="muted">Offline downloads need the Lectern desktop app</p>';

// ---- book details: everything the server knows about one book, plus play and download ----
const playLabel = id => { const c = coverCls(id); return c.includes('playing') ? 'Pause' : c.includes('now') ? 'Resume' : 'Play'; };
views.book = async id => {
  const b = await client().details(id), it = dl?.get(id), authors = b.authors.join(', ');
  const meta = `data-v="${esc(id)}" data-t="${esc(b.title)}" data-a="${esc(authors)}"`;
  const facts = [['Narrated by', b.narrators.join(', ')], ['Length', b.duration ? len(b.duration) : ''], ['Chapters', b.chapters || ''], ['Published', [b.publisher, b.year].filter(Boolean).join(', ')], ['Language', b.language], ['Genres', b.genres.join(', ')], ['Size', b.size ? bytes(b.size) : '']].filter(([, v]) => v);
  const text = b.description.replace(/<\/?(p|div|br)[^>]*>/gi, '\n').replace(/<[^>]+>/g, '').trim();
  return `<div><button class="lnk back" data-act="back">${svg('<path d="M15 6l-6 6 6 6"/>', 16)}Back</button></div>
<div class="row" style="align-items:flex-start;justify-content:flex-start;gap:32px">
<div style="width:240px;max-width:100%;flex:none"><div class="cover" style="background:${tint(b.title)}">${esc(b.title)}<img class="cover-img" src="${esc(client().coverUrl(id))}" alt=""></div></div>
<div class="col g16" style="flex:1 1 360px;min-width:0;max-width:720px">
<div class="col g4"><h1>${esc(b.title)}</h1>${b.subtitle ? `<div class="muted">${esc(b.subtitle)}</div>` : ''}${authors ? `<div class="muted">by ${esc(authors)}</div>` : ''}
${b.series.length ? `<div class="muted">${b.series.map(s => `<a href="#/series/${esc(s.id)}" style="text-decoration:underline">${esc(s.name)}${s.sequence ? ' #' + esc(s.sequence) : ''}</a>`).join(', ')}</div>` : ''}</div>
${b.finished ? '<div class="label accent">Finished</div>' : b.progress > 0 ? `<div class="col g4" style="max-width:360px">${bar(b.progress)}<div class="cap muted">${Math.round(b.progress * 100)}% listened</div></div>` : ''}
<div class="row g12" style="justify-content:flex-start;align-items:center"><button class="btn primary" data-act="play" data-playbtn ${meta}>${playLabel(id)}</button>
${!dl ? '' : !it ? `<button class="btn" data-act="dl" ${meta}>Download</button>` : `${it.status === 'done' ? '' : `<span class="muted">${dlStatus(it)}</span>`}<button class="btn" data-act="dlremove" data-v="${esc(id)}">${it.status === 'done' ? 'Remove download' : 'Cancel download'}</button>`}</div>
<div class="col g8">${facts.map(([k, v]) => `<div class="row g12" style="justify-content:flex-start;flex-wrap:nowrap;align-items:baseline"><span class="muted" style="width:96px;flex:none">${k}</span><span>${esc(v)}</span></div>`).join('')}</div>
${text ? `<div style="white-space:pre-line">${esc(text)}</div>` : ''}
</div></div>`;
};

// ---- stats: listening time, streak, authors and finished books (computed from the server's listening stats) ----
const RANGES = { week: ['Week', 'Listening over the last 7 days'], month: ['Month', 'Listening so far this month'], year: ['Year', 'Listening so far in ' + new Date().getFullYear()] };
const hrs = s => s >= 36000 ? Math.round(s / 3600) + 'h' : len(s);
const shortDate = ms => new Date(ms).toLocaleDateString('en', { month: 'short', day: 'numeric', ...(new Date(ms).getFullYear() !== new Date().getFullYear() && { year: 'numeric' }) });

function statsHtml() {
  const d = state.stats, r = state.statsRange, s = summarize(d, r);
  const tile = (label, value) => `<div class="card tile"><div class="cap muted">${label}</div><div class="big">${value}</div></div>`;
  const max = Math.max(...s.bars.map(b => b.secs), 1), few = s.bars.length <= 12;
  const val = secs => r === 'year' ? Math.round(secs / 3600) + 'h' : Math.round(secs / 60) + 'm';
  const top = s.authors[0]?.secs || 1;
  return `<div class="row"><div class="col g4"><h1>Stats</h1><div class="muted">${RANGES[r][1]}</div></div>
<div class="row g8" role="group" aria-label="Time range" style="flex-wrap:nowrap">${Object.entries(RANGES).map(([k, [name]]) => `<button class="seg" data-act="range" data-v="${k}" aria-pressed="${k === r}">${name}</button>`).join('')}</div></div>
<section class="tiles" aria-label="Totals">${tile('Time listened', hrs(s.time))}${tile('Books finished', s.finishedCount)}${tile('Daily average', len(s.avg))}${tile('Current streak', s.streak + (s.streak === 1 ? ' day' : ' days'))}</section>
<section class="card" style="padding:24px;display:flex;flex-direction:column;gap:16px"><h2>${r === 'year' ? 'Hours listened per month' : 'Minutes listened per day'}</h2>
<figure class="chart" role="img" aria-label="Bar chart of time listened. ${s.bars.map(b => b.label + ' ' + val(b.secs)).join(', ')}" style="gap:${few ? 12 : 4}px">${s.bars.map((b, i) => `<div class="barcol" title="${esc(b.label)}: ${val(b.secs)}">${few ? `<div class="cap strong">${val(b.secs)}</div>` : ''}<div style="width:100%;height:${Math.max(b.secs ? 2 : 0, Math.round(b.secs / max * 196))}px;background:${i === s.bars.length - 1 ? 'var(--accent)' : 'var(--track)'};border-radius:4px 4px 0 0"></div></div>`).join('')}</figure>
<div aria-hidden="true" class="chart-x" style="gap:${few ? 12 : 4}px">${s.bars.map(b => `<div class="cap muted">${esc(b.label)}</div>`).join('')}</div></section>
<div class="row" style="align-items:flex-start;justify-content:flex-start;gap:24px">
<section class="col g12" style="flex:1 1 360px;min-width:0"><div class="col g4"><h2>Most listened authors</h2><div class="cap muted">All time</div></div>${s.authors.length ? `<ul class="plain col g12">${s.authors.map(a => `<li class="col g4"><div class="row" style="flex-wrap:nowrap"><span class="strong">${esc(a.name)}</span><span class="muted">${hrs(a.secs)}</span></div><div class="bar"><i style="width:${Math.round(a.secs / top * 100)}%"></i></div></li>`).join('')}</ul>` : '<p class="muted">No listening yet</p>'}</section>
<section class="col g12" style="flex:1 1 360px;min-width:0"><div class="col g4"><h2>Recently finished</h2><div class="cap muted">Latest five</div></div>${d.recent.length ? `<ul class="plain fin">${d.recent.map(f => `<li><div class="dcov" style="width:40px;height:40px;background:${tint(f.title)}"><img class="cover-img" src="${esc(client().coverUrl(f.id))}" alt="" loading="lazy"></div><a class="col btxt" href="#/book/${esc(f.id)}" style="flex:1;min-width:0"><span class="strong">${esc(f.title)}</span><span class="cap muted">${esc(f.author)}</span></a><span class="cap muted">${shortDate(f.at)}</span></li>`).join('')}</ul>` : '<p class="muted">No finished books yet</p>'}</section></div>`;
}
views.stats = async () => {
  const api = client(), d = await api.stats();
  const known = Object.fromEntries(d.items.map(i => [i.id, i]));
  d.recent = (await Promise.all([...d.finished].sort((a, b) => b.at - a.at).slice(0, 5).map(async f => {
    const k = known[f.id] ?? await api.details(f.id).then(x => ({ title: x.title, author: x.authors.join(', ') }), () => null);
    return k && { id: f.id, at: f.at, title: k.title, author: k.author };
  }))).filter(Boolean);
  state.stats = d;
  return `<div id="statspage" class="col" style="gap:32px">${statsHtml()}</div>`;
};

views.player = async () => {
  const s = player.s;
  const back = `<div><button class="lnk back" data-act="back">${svg('<path d="m6 9 6 6 6-6"/>', 16)}Collapse player</button></div>`;
  if (!s) return `${back}<div class="col g4"><h1>Player</h1><div class="muted">Nothing is playing. Pick a book from Home or Library</div></div>`;
  const pk = (k, label, icon, cls = '') => `<button class="ib ${cls}" data-act="pk" data-v="${k}" id="pp-${k}" aria-label="${label}"${s.chapters.length || !/prev|next/.test(k) ? '' : ' disabled'}>${icon}</button>`;
  return `${back}<div class="pp">
<section class="pp-main"><div class="cover pp-cover" style="background:${tint(s.title)}">${esc(s.title)}<img class="cover-img" src="${esc(client().coverUrl(player.id))}" alt=""></div>
<div class="col g4"><h1>${esc(s.title)}</h1><div class="muted">${esc(s.author)}</div><div class="muted" id="pp-sub"></div></div>
<div class="col g8"><input type="range" class="pp-seek" id="pp-seek" min="0" max="${Math.floor(s.duration)}" step="1" value="0" aria-label="Position in book"><div class="row cap muted" style="flex-wrap:nowrap"><span id="pp-t0"></span><span id="pp-t1"></span></div></div>
<div class="pp-ctl">${pk('prev', 'Previous chapter', svg(PL.prev, 24))}${pk('back', 'Back 15 seconds', skipIcon(15, true))}${pk('toggle', 'Pause', svg(PL.pause, 28), 'main big')}${pk('fwd', 'Forward 30 seconds', skipIcon(30, false))}${pk('next', 'Next chapter', svg(PL.next, 24))}</div>
<div class="row g8" style="justify-content:center" role="group" aria-label="Playback speed">${RATES.map(r => `<button class="seg" data-act="prate" data-v="${r}" aria-pressed="false">${r}×</button>`).join('')}</div></section>
${s.chapters.length ? `<section class="col g12 pp-side"><h2>Chapters</h2><ul class="plain card pp-chs" id="pp-chs">${s.chapters.map((c, i) => `<li><button data-act="pch" data-v="${i}"><span>${esc(c.title)}</span><span class="cap muted">${clock(c.start)}</span></button></li>`).join('')}</ul></section>` : ''}
</div>`;
};

// ---- render ----
let seq = 0;
const failure = e => `<div role="alert" class="danger">Error: ${e.status ? `The server answered ${e.status}.` : `Can't reach ${esc(state.session.server)}. Check the address or your connection.`} <a href="#/settings" style="text-decoration:underline">Check Settings</a></div>`;

async function render() {
  const [route, arg] = location.hash.slice(2).split('/'), n = ++seq, r = [...NAV, 'book', 'player'].includes(route) ? route : (state.session ? 'home' : 'settings');
  document.documentElement.dataset.theme = state.theme;
  document.body.classList.toggle('full', r === 'player'); // the full player replaces the mini bar
  if (state.session && !state.libs) await loadLibs();
  if (n !== seq) return;
  const loading = state.session && r !== 'settings';
  const focus = document.activeElement?.id;
  document.getElementById('app').innerHTML = `<div class="shell">${sidebar(r)}<main id="main">${r === 'settings' ? settings() : loading ? '<p class="muted">Loading…</p>' : '<h1>Welcome</h1><p class="muted">Connect to your server in <a href="#/settings" style="text-decoration:underline">Settings</a> to see your books.</p>'}</main></div>`;
  if (focus) document.getElementById(focus)?.focus();
  if (!loading) return;
  let html;
  try { html = await views[r](arg); } catch (e) { if (e.status === 401) return signOut('Error: Your session expired. Sign in again.'); html = failure(e); }
  if (n === seq) { document.getElementById('main').innerHTML = html; if (r === 'player') paintPage(true); }
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
    preloadLast();
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
const player = { id: null, s: null, i: 0, listened: 0, last: 0, rate: 1, vol: store.get('vol', 1), muted: false };
const RATES = [0.75, 1, 1.25, 1.5, 1.75, 2];
const audio = () => document.getElementById('audio');
const position = () => player.s.tracks[player.i].start + audio().currentTime;
const clock = t => { t = Math.max(0, Math.floor(t)); const h = Math.floor(t / 3600), m = Math.floor(t % 3600 / 60), s = t % 60; return (h ? h + ':' + String(m).padStart(2, '0') : m) + ':' + String(s).padStart(2, '0'); };
const skipIcon = (n, back) => svg(`<path d="${back ? 'M4 12a8 8 0 1 1 2.5 5.8M4 5v5h5' : 'M20 12a8 8 0 1 0-2.5 5.8M20 5v5h-5'}"/><text x="12" y="15" font-size="8" text-anchor="middle" fill="currentColor" stroke="none">${n}</text>`, 24);
const PL = {
  prev: '<path d="M6 5v14M19 5 9 12l10 7z"/>', next: '<path d="M18 5v14M5 5l10 7-10 7z"/>',
  list: '<path d="M9 6h11M9 12h11M9 18h11M4 6h.01M4 12h.01M4 18h.01"/>',
  vol: '<path d="M4 9v6h4l5 4V5L8 9zM16.5 8.5a5 5 0 0 1 0 7M19 6a8.5 8.5 0 0 1 0 12"/>', mute: '<path d="M4 9v6h4l5 4V5L8 9zM17 9l5 6M22 9l-5 6"/>',
  play: '<path d="M8 5v14l11-7z" fill="currentColor"/>', pause: '<path d="M8 5v14M16 5v14" stroke-width="2.5"/>'
};

// listening queued while the server was out of reach; each book is replayed as one short server session
const listenQ = makeListenQueue({
  load: () => store.get('listenq', null), save: v => store.set('listenq', v),
  send: async (id, e) => { const s = await client().play(id); await client().sync(s.sessionId, { currentTime: e.currentTime, timeListened: Math.round(e.timeListened), duration: e.duration }, true); }
});
const flushListening = () => state.session && listenQ.size() ? listenQ.flush(player.id) : undefined;
window.addEventListener('online', flushListening);
function syncPlayer(close = false, queueOnly = false) {
  if (!player.s) return;
  store.set('pos.' + player.id, position()); // the place is always kept here too, for offline starts
  const { sessionId, duration } = player.s, body = { currentTime: position(), timeListened: Math.round(player.listened), duration };
  player.listened = 0;
  // no session (server was unreachable at start), a dead one, or leaving the app: keep the time queued, it is sent when the server answers
  const keep = () => listenQ.add(player.id, body);
  if (!sessionId || queueOnly) { keep(); return queueOnly ? undefined : flushListening(); }
  return client().sync(sessionId, body, close).then(flushListening, keep);
}
// a book that was streaming while it downloaded moves to the local copy, at the same spot, once it is complete
async function swapToLocal(id) {
  const s = player.s;
  if (player.id !== id || s?.local || !audio()) return;
  const local = await dl.session(id, 0), a = audio(), pos = position(), playing = !a.paused;
  if (!local || player.s !== s) return;
  s.local = true; s.tracks = local.tracks; // the server session stays open, only the audio source changes
  player.i = trackAt(s, pos); player.last = 0;
  a.addEventListener('loadedmetadata', () => { a.currentTime = Math.max(0, pos - s.tracks[player.i].start); if (playing) a.play(); }, { once: true });
  a.src = s.tracks[player.i].url;
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
  document.querySelectorAll('[data-playbtn]').forEach(b => { b.textContent = playLabel(b.dataset.v); });
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
// two popover menus share one open/close routine: opening one closes the other, no argument closes both
const MENUS = { chap: ['np-menu', 'np-chapbtn'], speed: ['np-smenu', 'np-rate'] };
function toggleMenu(which, open) {
  for (const [k, [menuId, btnId]] of Object.entries(MENUS)) {
    const m = document.getElementById(menuId), b = document.getElementById(btnId);
    if (!m) continue;
    m.hidden = k === which ? !(open ?? m.hidden) : true;
    b.setAttribute('aria-expanded', String(!m.hidden));
    // keep the menu inside the window: slide it left when it would run past the right edge
    m.style.left = '0px';
    if (m.hidden) continue;
    const r = m.getBoundingClientRect();
    m.style.left = Math.min(0, innerWidth - 16 - r.right) + 'px';
    // scroll the menu itself; scrollIntoView would also scroll the page behind the player
    const cur = m.querySelector('[aria-current]');
    if (cur) m.scrollTop = cur.offsetTop - (m.clientHeight - cur.offsetHeight) / 2;
  }
}
function paintVolume() {
  const a = audio(), off = player.muted || player.vol === 0;
  if (a) a.volume = player.vol, a.muted = off;
  const b = document.getElementById('np-mute'), r = document.getElementById('np-vol');
  if (b) { b.innerHTML = svg(off ? PL.mute : PL.vol, 24); b.setAttribute('aria-label', off ? 'Unmute' : 'Mute'); }
  if (r) r.value = off ? 0 : player.vol;
}
const ctl = {
  mute: () => { player.muted = !player.muted; if (!player.muted && player.vol === 0) player.vol = 0.5; paintVolume(); },
  chapters: () => toggleMenu('chap'),
  speed: () => toggleMenu('speed'),
  prev: () => chapterStep(-1), next: () => chapterStep(1),
  back: () => seek(position() - 15), fwd: () => seek(position() + 30),
  toggle: () => { const a = audio(); a.paused ? a.play() : a.pause(); },
  rate: r => {
    player.rate = r;
    audio().defaultPlaybackRate = audio().playbackRate = r;
    document.getElementById('np-rate').textContent = r + '×';
    document.querySelectorAll('#np-smenu [data-rate]').forEach(b => +b.dataset.rate === r ? b.setAttribute('aria-current', 'true') : b.removeAttribute('aria-current'));
  }
};

function paintPage(first = false) {
  const s = player.s, a = audio(), sk = document.getElementById('pp-seek');
  if (!s || !a || !sk) return;
  const t = position(), c = chapterAt(s, t);
  if (!sk.matches(':active')) { sk.value = Math.floor(t); sk.style.setProperty('--p', (s.duration ? t / s.duration * 100 : 0) + '%'); }
  document.getElementById('pp-t0').textContent = clock(t);
  document.getElementById('pp-t1').textContent = '-' + clock(s.duration - t);
  document.getElementById('pp-sub').textContent = (c >= 0 ? `Chapter ${c + 1} of ${s.chapters.length} · ` : '') + len(Math.max(0, s.duration - t)) + ' left';
  const tg = document.getElementById('pp-toggle');
  tg.innerHTML = svg(a.paused ? PL.play : PL.pause, 28);
  tg.setAttribute('aria-label', a.paused ? 'Play' : 'Pause');
  document.querySelectorAll('#pp-chs [data-v]').forEach((b, i) => i === c ? b.setAttribute('aria-current', 'true') : b.removeAttribute('aria-current'));
  document.querySelectorAll('[data-act=prate]').forEach(b => b.setAttribute('aria-pressed', String(+b.dataset.v === player.rate)));
  const list = document.getElementById('pp-chs'), cur = list?.querySelector('[aria-current]');
  if (first && cur) list.scrollTop = cur.offsetTop - (list.clientHeight - cur.offsetHeight) / 2; // centre the current chapter when the page opens
}
function paintPlayer() {
  const s = player.s, a = audio();
  if (!s || !a) return;
  const t = position(), c = chapterAt(s, t);
  const sk = document.getElementById('np-seek');
  sk.value = Math.floor(t);
  sk.style.setProperty('--p', (s.duration ? t / s.duration * 100 : 0) + '%');
  document.getElementById('np-time').textContent = `${clock(t)} / ${clock(s.duration)}`;
  document.querySelectorAll('#np-menu [data-ch]').forEach((b, i) => i === c ? b.setAttribute('aria-current', 'true') : b.removeAttribute('aria-current'));
  document.getElementById('np-toggle').innerHTML = svg(a.paused ? PL.play : PL.pause, 24);
  document.getElementById('np-toggle').setAttribute('aria-label', a.paused ? 'Play' : 'Pause');
  paintCovers();
  paintPage();
}

async function startPlayer({ v: id, t: title, a: author }, autoplay = true) {
  stopPlayer();
  player.id = id; paintCovers();
  const el = document.getElementById('player');
  el.hidden = false;
  el.innerHTML = `<div class="info"><div class="strong">${esc(title)}</div><div class="cap muted" id="np-status">Loading…</div></div>`;
  const status = m => { const n = document.getElementById('np-status'); if (n) n.textContent = m; };
  try {
    // audio comes from the local copy when it is complete, otherwise from the server. Either way a server session is opened when it can
    // be reached, so the resume point and the listening stats are the server's. Offline, only the place is remembered here.
    const local = dl && await dl.session(id, store.get('pos.' + id, 0));
    const s = local ?? await client().play(id);
    if (local) {
      const srv = await client().play(id).catch(() => null);
      Object.assign(local, { local: true, sessionId: srv?.sessionId ?? null, start: srv?.start ?? local.start });
    }
    if (!s.tracks.length) throw new Error('no audio');
    s.author = author; s.title = title;
    player.s = s; player.i = trackAt(s, s.start); player.listened = 0; player.last = 0;
    const ib = (k, label, icon, cls = '') => `<button class="ib ${cls}" data-k="${k}" id="np-${k}" aria-label="${label}"${s.chapters.length || !/prev|next/.test(k) ? '' : ' disabled'}>${icon}</button>`;
    el.innerHTML = `<input type="range" class="np-seek" id="np-seek" min="0" max="${Math.floor(s.duration)}" step="1" value="0" aria-label="Position in book">
<div class="info"><div class="strong">${esc(title)}</div><div class="cap muted"><span id="np-status">${esc(author)}</span> · <span id="np-time"></span></div></div>
<div class="np-ctl">${ib('prev', 'Previous chapter', svg(PL.prev, 24))}${ib('back', 'Back 15 seconds', skipIcon(15, true))}${ib('toggle', 'Pause', svg(PL.pause, 24), 'main')}${ib('fwd', 'Forward 30 seconds', skipIcon(30, false))}${ib('next', 'Next chapter', svg(PL.next, 24))}<div class="chap"><button class="ib txt" data-k="speed" id="np-rate" aria-haspopup="true" aria-expanded="false" aria-label="Playback speed">${player.rate}×</button><div class="menu card narrow" id="np-smenu" role="menu" hidden>${RATES.map(r => `<button role="menuitem" data-rate="${r}"${r === player.rate ? ' aria-current="true"' : ''}>${r}×</button>`).join('')}</div></div></div>
<div class="vol"><button class="ib" data-k="mute" id="np-mute"></button><input type="range" id="np-vol" min="0" max="1" step="0.05" aria-label="Volume"></div>
<a class="ib" href="#/player" id="np-open" aria-label="Open player">${svg('<path d="m6 15 6-6 6 6"/>', 24)}</a>
${s.chapters.length ? `<div class="chap"><button class="btn sm" data-k="chapters" id="np-chapbtn" aria-haspopup="true" aria-expanded="false">${svg(PL.list, 20)}Chapters</button><div class="menu card" id="np-menu" role="menu" hidden>${s.chapters.map((c, i) => `<button role="menuitem" data-ch="${i}"><span>${esc(c.title)}</span><span class="cap muted">${clock(c.start)}</span></button>`).join('')}</div></div>` : ''}
<audio id="audio"${autoplay ? ' autoplay' : ''} src="${esc(s.tracks[player.i].url)}"></audio>`;
    const a = audio();
    a.defaultPlaybackRate = a.playbackRate = player.rate;
    paintVolume();
    el.querySelector('#np-vol').oninput = e => { player.vol = +e.target.value; player.muted = false; store.set('vol', player.vol); paintVolume(); };
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
    a.addEventListener('error', () => {
      console.warn('audio error', a.error?.code, a.error?.message);
      // MediaError codes: 2 network, 3 decode, 4 source not supported
      status(a.error?.code === 2 ? "Can't read the file. Check your connection" : s.local ? 'This download looks damaged. Remove it in Downloads and download it again' : "Can't decode this file");
    });
    el.onclick = e => {
      const ch = e.target.closest('[data-ch]');
      const rt = e.target.closest('[data-rate]');
      if (ch) { seek(s.chapters[ch.dataset.ch].start); toggleMenu(); } else if (rt) { ctl.rate(+rt.dataset.rate); toggleMenu(); } else ctl[e.target.closest('[data-k]')?.dataset.k]?.();
    };
    el.querySelector('#np-seek').oninput = e => { e.target.style.setProperty('--p', (s.duration ? e.target.value / s.duration * 100 : 0) + '%'); document.getElementById('np-time').textContent = `${clock(e.target.value)} / ${clock(s.duration)}`; };
    el.querySelector('#np-seek').onchange = e => seek(+e.target.value);
    document.onclick = e => { if (!e.target.closest('.chap')) toggleMenu(); };
    el.onkeydown = e => { if (e.key === 'Escape') { toggleMenu(); e.target.closest('.chap')?.querySelector('[aria-haspopup]')?.focus(); } };
    paintPlayer();
    if (location.hash === '#/player') render(); // the page was open before anything was loaded
  } catch (e) {
    if (!autoplay) return stopPlayer(); // a quiet preload that fails just leaves the player hidden
    player.id = null; paintCovers();
    status(e.status === 401 ? 'Your session expired. Sign in again.' : e.status ? `The server answered ${e.status}.` : "Can't start playback. Check your connection.");
  }
}
window.addEventListener('beforeunload', () => syncPlayer(false, true)); // the page is going away: queue now, send next launch
// the player is always there when there is something to resume: load the most recent Continue listening book, paused
// shortcut: this opens a server session without listening to anything yet; harmless, but it shows in the server's session list until closed
async function preloadLast() {
  if (!state.session || player.id) return;
  await flushListening(); // so the server's place below includes what was listened offline
  const b = (await client().inProgress().catch(() => [])).find(x => !state.hidden.includes(x.libraryId));
  if (b && !player.id) startPlayer({ v: b.id, t: b.title, a: b.author }, false);
}

function confirmBox({ title, message, ok, cancel = 'Keep' }) {
  const m = document.getElementById('modal');
  m.innerHTML = `<h2 id="modal-title">${esc(title)}</h2><p class="muted">${esc(message)}</p><div class="mactions"><button type="button" class="btn" data-r="no" autofocus>${esc(cancel)}</button><button type="button" class="btn primary" data-r="yes">${esc(ok)}</button></div>`;
  m.returnValue = ''; // Escape closes without setting it, so clear the last answer
  return new Promise(done => {
    m.onclose = () => done(m.returnValue === 'yes');
    m.onclick = e => { const r = e.target.closest('[data-r]')?.dataset.r; if (r) m.close(r); else if (e.target === m) m.close('no'); };
    m.showModal();
  });
}

const act = {
  pk: d => { ctl[d.v]?.(); return true; },
  prate: d => { ctl.rate(+d.v); paintPage(); return true; },
  pch: d => { seek(player.s.chapters[d.v].start); return true; },
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
  lib: d => { state.libId = d.v; store.set('lib', d.v); },
  back: () => { history.back(); return true; },
  dlchoose: async () => {
    const path = await T.dialog.open({ directory: true, title: 'Choose download folder' });
    if (!path) return;
    await T.core.invoke('allow_dir', { path });
    state.dlcfg.dir = path; store.set('dlcfg', state.dlcfg); render();
  },
  dlreset: () => { state.dlcfg.dir = null; store.set('dlcfg', state.dlcfg); },
  dl: d => {
    dl.add({ id: d.v, title: d.t, author: d.a });
    if (location.hash.startsWith('#/book/')) return false;
    document.querySelector(`[data-act=dl][data-v="${CSS.escape(d.v)}"]`)?.replaceWith(Object.assign(document.createElement('div'), { innerHTML: dlExtra({ id: d.v }) }).firstChild);
    return true;
  },
  dlpause: () => { dl.paused ? dl.resumeAll() : dl.pauseAll(); paintDownloads(); return true; },
  dlclean: () => { dl.removeFinished().then(paintDownloads); return true; },
  dlretry: d => { dl.retry(d.v); paintDownloads(); return true; },
  dlremove: async d => {
    const it = dl.get(d.v), done = it?.status === 'done';
    const ok = await confirmBox(done
      ? { title: 'Remove download', message: `Remove "${it.title}" from this computer? You can download it again later.`, ok: 'Remove' }
      : { title: 'Cancel download', message: `Cancel the download of "${it?.title}" and delete what was saved so far?`, ok: 'Cancel download' });
    if (ok) await dl.remove(d.v).then(() => location.hash.startsWith('#/book/') ? render() : paintDownloads());
    return true;
  },
  range: d => { state.statsRange = d.v; document.getElementById('statspage').innerHTML = statsHtml(); return true; },
  rename: () => { state.renaming = !state.renaming; state.renameErr = ''; paintSerHead(); return true; }
};
document.addEventListener('click', e => {
  const t = e.target.closest('[data-act]');
  if (t && !act[t.dataset.act](t.dataset)) render();
});
document.addEventListener('submit', e => { e.preventDefault(); e.target.id === 'renameser' ? renameSeries(e.target.elements.sername.value) : signIn(); });
document.addEventListener('change', e => { if (e.target.id === 'pp-seek') seek(+e.target.value); });
document.addEventListener('input', e => {
  if (e.target.id === 'pp-seek') { const t = +e.target.value; e.target.style.setProperty('--p', (player.s.duration ? t / player.s.duration * 100 : 0) + '%'); document.getElementById('pp-t0').textContent = clock(t); }
  if (e.target.id === 'dllimit' && +e.target.value > 0) { state.dlcfg.limitGB = +e.target.value; store.set('dlcfg', state.dlcfg); }
  if (e.target.dataset.f) state.acct[e.target.dataset.f] = e.target.value;
  if (e.target.id === 'q') {
    state.q = e.target.value;
    document.getElementById('grid').innerHTML = grid(libBooks());
  }
});
document.addEventListener('error', e => e.target.classList?.contains('cover-img') && e.target.remove(), true);
// space toggles play/pause, unless the key belongs to what has focus (typing, or a button/link that space already activates)
document.addEventListener('keydown', e => {
  if (e.key !== ' ' || e.repeat || !player.s || !audio()) return;
  const t = e.target;
  if (t.isContentEditable || t.closest('textarea, select, button, a, input:not([type=range])')) return;
  e.preventDefault();
  ctl.toggle();
});
window.addEventListener('hashchange', render);
render();
preloadLast();
// a chosen folder must be re-allowed on every launch before anything reads or writes it
if (dl) Promise.all([...new Set([state.dlcfg.dir, ...dl.list().filter(i => i.abs).map(i => i.root)])].filter(Boolean).map(path => T.core.invoke('allow_dir', { path }))).catch(() => {}).finally(() => state.session && dl.pump());
