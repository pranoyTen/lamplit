// Audiobookshelf REST client. fetchFn is injected: Tauri's http fetch in the app (no CORS limits), a stub in tests.
function absClient(fetchFn, { server = '', token = '' } = {}) {
  const base = server.trim().replace(/\/+$/, '');
  if (!/^https?:\/\/[^/\s]+/.test(base)) throw Object.assign(new Error('Start the address with http:// or https://'), { badAddress: true });

  async function call(path, init = {}, noBody = false) {
    const res = await fetchFn(base + path, { ...init, headers: { 'Content-Type': 'application/json', ...(token && { Authorization: 'Bearer ' + token }) } });
    if (!res.ok) throw Object.assign(new Error('HTTP ' + res.status), { status: res.status });
    return noBody ? null : res.json();
  }
  const withToken = u => base + u + (u.includes('?') ? '&' : '?') + 'token=' + encodeURIComponent(token);
  const book = i => ({ id: i.id, libraryId: i.libraryId, title: i.media.metadata.title, author: i.media.metadata.authorName ?? i.media.metadata.author ?? '', duration: i.media.duration || 0 });

  return {
    base,
    async login(username, password) {
      const { user } = await call('/login', { method: 'POST', body: JSON.stringify({ username, password }) });
      return { server: base, token: user.token || user.accessToken, username: user.username };
    },
    libraries: async () => (await call('/api/libraries')).libraries.map(l => ({ id: l.id, name: l.name })),
    // shortcut: one page only, no paging UI; add paging when a library passes a few hundred books
    async items(libId, { limit = 50, sort = 'addedAt', desc = 0 } = {}) {
      const r = await call(`/api/libraries/${libId}/items?limit=${limit}&sort=${encodeURIComponent(sort)}&desc=${desc}`);
      return { total: r.total, books: r.results.map(book) };
    },
    inProgress: async () => (await call('/api/me/items-in-progress')).libraryItems.map(book),
    progress: async () => Object.fromEntries((await call('/api/me')).mediaProgress.map(p => [p.libraryItemId, p.progress])),
    // opens a playback session; start is where the server last left off, in seconds from the book's start
    async play(id) {
      const s = await call(`/api/items/${id}/play`, { method: 'POST', body: JSON.stringify({ deviceInfo: { clientName: 'Lectern' }, forceDirectPlay: true, supportedMimeTypes: ['audio/mpeg', 'audio/mp4', 'audio/flac', 'audio/ogg'] }) });
      return { sessionId: s.id, start: s.currentTime || 0, duration: s.duration || 0, chapters: (s.chapters || []).map(c => ({ start: c.start, end: c.end, title: c.title })), tracks: s.audioTracks.map(t => ({ url: withToken(t.contentUrl), start: t.startOffset || 0, duration: t.duration })) };
    },
    sync: (sessionId, body, close = false) => call(`/api/session/${sessionId}/${close ? 'close' : 'sync'}`, { method: 'POST', body: JSON.stringify(body) }, true),
    // series list: each entry carries its books in reading order
    series: async libId => (await call(`/api/libraries/${libId}/series?limit=200&sort=name`)).results.map(x => ({ id: x.id, name: x.name, books: (x.books || []).map(book) })),
    // the files to download for one book, in play order
    async item(id) {
      const m = (await call(`/api/items/${id}?expanded=1`)).media;
      let start = 0;
      const tracks = [...m.audioFiles].sort((a, b) => a.index - b.index).map(f => {
        const t = { url: withToken(`/api/items/${id}/file/${f.ino}`), ext: f.metadata.ext || '.' + f.metadata.filename.split('.').pop(), size: f.metadata.size || 0, duration: f.duration, start };
        start += f.duration;
        return t;
      });
      return { tracks, duration: m.duration || start, chapters: (m.chapters || []).map(c => ({ start: c.start, end: c.end, title: c.title })) };
    },
    async details(id) {
      const i = await call(`/api/items/${id}?expanded=1&include=progress`), m = i.media, md = m.metadata, p = i.userMediaProgress;
      return {
        title: md.title, subtitle: md.subtitle || '', authors: (md.authors || []).map(x => x.name), narrators: md.narrators || [],
        series: (md.series || []).map(x => ({ id: x.id, name: x.name, sequence: x.sequence })), genres: md.genres || [],
        year: md.publishedYear || '', publisher: md.publisher || '', language: md.language || '', description: md.description || '',
        duration: m.duration || 0, size: m.size || 0, chapters: (m.chapters || []).length, progress: p?.progress || 0, finished: !!p?.isFinished
      };
    },
    renameSeries: (id, name) => call(`/api/series/${id}`, { method: 'PATCH', body: JSON.stringify({ name }) }, true),
    coverUrl: id => `${base}/api/items/${id}/cover?width=400&token=${encodeURIComponent(token)}`
  };
}
if (typeof module !== 'undefined') module.exports = { absClient };
