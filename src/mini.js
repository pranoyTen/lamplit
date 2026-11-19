// The mini player shown from the tray icon. It owns no audio: the main window sends its state ("mini-state") and gets the same
// "taskbar" commands the Windows thumbnail buttons use (prev, back, toggle, fwd, next).
const T = window.__TAURI__;
const $ = id => document.getElementById(id);
// on a Mac the window is native glass and follows the system light/dark setting; elsewhere it follows the app's theme
const mac = /Mac/.test(navigator.userAgent);
document.documentElement.classList.toggle('mac', mac);
if (!mac) try { const th = JSON.parse(localStorage.getItem('lectern.theme')); if (th) document.documentElement.dataset.theme = th; } catch {}

const svg = d => `<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${d}</svg>`;
const skip = (n, back) => svg(`<path d="${back ? 'M4 12a8 8 0 1 1 2.5 5.8M4 5v5h5' : 'M20 12a8 8 0 1 0-2.5 5.8M20 5v5h-5'}"/><text x="12" y="15" font-size="8" text-anchor="middle" fill="currentColor" stroke="none">${n}</text>`);
const ICON = { prev: svg('<path d="M6 5v14M19 5 9 12l10 7z"/>'), next: svg('<path d="M18 5v14M5 5l10 7-10 7z"/>'), play: svg('<path d="M8 5v14l11-7z" fill="currentColor"/>'), pause: svg('<path d="M8 5v14M16 5v14" stroke-width="2.5"/>') };
const clock = t => { t = Math.max(0, Math.floor(t)); const h = Math.floor(t / 3600), m = Math.floor(t % 3600 / 60), s = t % 60; return (h ? h + ':' + String(m).padStart(2, '0') : m) + ':' + String(s).padStart(2, '0'); };

function paint(p) {
  $('title').textContent = p.has ? p.title : 'Nothing playing';
  $('chap').textContent = p.has ? p.chapter : '';
  $('author').textContent = p.has ? p.author : '';
  $('cover').style.backgroundImage = p.has ? `url("${p.cover}")` : '';
  $('fill').style.width = p.has && p.dur ? Math.min(100, p.pos / p.dur * 100) + '%' : '0';
  $('t0').textContent = p.has ? clock(p.pos) : '0:00';
  $('t1').textContent = p.has ? clock(p.dur) : '0:00';
  const b = (k, label, icon, cls = '') => `<button data-k="${k}" class="${cls}" aria-label="${label}"${p.has && (p.hasChapters || !/prev|next/.test(k)) ? '' : ' disabled'}>${icon}</button>`;
  $('ctl').innerHTML = b('prev', 'Previous chapter', ICON.prev) + b('back', 'Skip back', skip(p.back ?? 15, true)) + b('toggle', p.playing ? 'Pause' : 'Play', p.playing ? ICON.pause : ICON.play, 'main') + b('fwd', 'Skip forward', skip(p.fwd ?? 30, false)) + b('next', 'Next chapter', ICON.next);
}

$('ctl').onclick = e => { const k = e.target.closest('[data-k]')?.dataset.k; if (k) T.event.emit('taskbar', k); };
$('open').onclick = () => T.core.invoke('show_main');
document.onkeydown = e => { if (e.key === 'Escape') T.window.getCurrentWindow().destroy(); };
paint({ has: false });
T.event.listen('mini-state', e => paint(e.payload));
// ask the main window for the current state when this one opens or is shown again
const ask = () => T.event.emit('mini-ready');
window.addEventListener('focus', ask);
ask();
