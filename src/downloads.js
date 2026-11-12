// Offline downloads: a queue that saves each book's audio files to the app data folder, one book at a time.
// Everything the OS provides is injected (fetch, fs, storage) so the queue can be tested without Tauri.
// Files come down in Range chunks and the byte count is saved as it grows, so a cancelled or crashed download resumes where it stopped.
// shortcut: the manifest lives in localStorage; move it to a file if the queue ever holds hundreds of books.
function makeDownloads({ fetchFn, fs, base, cfg, api, urlFor, load, save, onChange, onDone, chunk = 8 * 1024 * 1024 }) {
  // cfg() -> { root, abs, limit }: where new downloads go (abs: an absolute folder instead of one under the app data dir) and the byte cap (0 = none)
  const items = load() ?? {};
  let current = null, paused = false, ctl = null;
  for (const it of Object.values(items)) if (it.status === 'active') it.status = 'queued'; // the app closed mid-download

  const dir = it => `${it.root ?? 'downloads'}/${it.id}`;
  const opt = it => it.abs ? {} : { baseDir: base };
  const notify = () => onChange?.();
  const persist = () => save(items);
  // file-system failures come back as plain strings; tag them so they are not mistaken for a network problem
  const disk = p => Promise.resolve(p).catch(e => { throw Object.assign(new Error(String(e?.message ?? e)), { disk: true }); });
  const fail = e => e.disk ? `Couldn't write the file: ${e.message}` : e.limit ? 'Over the storage limit. Raise it in Settings or remove books.' : e.status ? `The server answered ${e.status}.` : "Can't reach the server. Check your connection.";

  async function run(it) {
    current = it.id; it.status = 'active'; it.error = ''; notify();
    try {
      if (!it.tracks) {
        const info = await api().item(it.id);
        const total = info.tracks.reduce((n, t) => n + t.size, 0), { root, abs, limit } = cfg();
        if (limit && dl.bytes() + total > limit) throw Object.assign(new Error('limit'), { limit: true });
        Object.assign(it, { root, abs, total, bytes: 0, chapters: info.chapters, duration: info.duration });
        it.tracks = info.tracks.map((t, i) => ({ ...t, file: `${dir(it)}/${i}${t.ext}`, got: 0, done: false }));
      }
      await disk(fs.mkdir(dir(it), { ...opt(it), recursive: true }));
      for (const t of it.tracks.filter(t => !t.done)) {
        // whole-file mode: the size is unknown, or the server ignored Range and sent everything
        let whole = !t.size;
        while (whole || t.got < t.size) {
          // trust the file, not the counter: seeking past the real end would leave a hole of zeros that no player can decode
          if (t.got > 0) {
            const { size } = await disk(fs.stat(t.file, opt(it)));
            if (size < t.got) { it.bytes -= t.got - size; t.got = size; }
          }
          ctl = new AbortController();
          const res = await fetchFn(t.url, { signal: ctl.signal, ...(!whole && { headers: { Range: `bytes=${t.got}-${Math.min(t.got + chunk, t.size) - 1}` } }) });
          if (!res.ok) throw Object.assign(new Error('HTTP ' + res.status), { status: res.status });
          if (!whole && res.status === 200) { whole = true; it.bytes -= t.got; t.got = 0; }
          const before = t.got;
          const f = await disk(fs.open(t.file, { write: true, create: true, truncate: t.got === 0, ...opt(it) }));
          try {
            if (t.got > 0) await disk(f.seek(t.got, 0)); // 0 = from the start; overwrites any bytes written after the last save
            const reader = res.body.getReader();
            for (let r = await reader.read(); !r.done; r = await reader.read()) {
              await disk(f.write(r.value));
              t.got += r.value.length; it.bytes += r.value.length; notify();
            }
          } finally { await f.close().catch(() => {}); persist(); } // saved even when cancelled, so the next run starts at t.got
          if (whole) { t.size = t.got; break; }
          if (t.got === before) throw new Error('The server sent no data');
        }
        t.done = true; persist();
      }
      it.status = 'done';
      onDone?.(it.id);
    } catch (e) {
      if (items[it.id] !== it) { /* removed while downloading */ }
      else if (paused) it.status = 'queued';
      else { it.status = 'error'; it.error = fail(e); }
    }
    current = null; ctl = null; persist(); notify(); pump();
  }

  function pump() {
    if (current || paused) return;
    const next = Object.values(items).find(i => i.status === 'queued');
    if (next) run(next);
  }

  const dl = {
    list: () => Object.values(items),
    get: id => items[id],
    bytes: () => Object.values(items).reduce((n, i) => n + (i.bytes || 0), 0),
    get paused() { return paused; },
    add({ id, title, author }) {
      if (items[id]) return;
      items[id] = { id, title, author, status: 'queued', bytes: 0, total: 0, error: '' };
      persist(); notify(); pump();
    },
    retry(id) { const it = items[id]; if (it?.status === 'error') { it.status = 'queued'; persist(); notify(); pump(); } },
    async remove(id) {
      const it = items[id];
      if (!it) return;
      delete items[id]; persist(); notify();
      if (current === id) ctl?.abort();
      await fs.remove(dir(it), { ...opt(it), recursive: true }).catch(() => {});
    },
    removeFinished() { return Promise.all(dl.list().filter(i => i.status === 'done').map(i => dl.remove(i.id))); },
    pauseAll() { paused = true; ctl?.abort(); notify(); },
    resumeAll() { paused = false; notify(); pump(); },
    // a playable session for a finished book, same shape as api.play()
    async session(id, start) {
      const it = items[id];
      if (it?.status !== 'done') return null;
      // a copy whose files lost bytes cannot play; stream it instead (a hole of zeros inside a right-sized file is not detectable)
      for (const t of it.tracks) if (await fs.stat(t.file, opt(it)).then(x => x.size, () => -1) !== t.size) return null;
      return { sessionId: null, start, duration: it.duration, chapters: it.chapters, tracks: await Promise.all(it.tracks.map(async t => ({ url: await urlFor(t.file, it.abs), start: t.start, duration: t.duration }))) };
    },
    pump
  };
  return dl;
}
if (typeof module !== 'undefined') module.exports = { makeDownloads };
