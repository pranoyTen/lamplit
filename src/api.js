// Audiobookshelf REST client. fetchFn is injected: Tauri's http fetch in the app (no CORS limits), a stub in tests.
function absClient(fetchFn, { server = '', token = '' } = {}) {
  const base = server.trim().replace(/\/+$/, '');
  if (!/^https?:\/\/[^/\s]+/.test(base)) throw Object.assign(new Error('Start the address with http:// or https://'), { badAddress: true });

  async function call(path, init = {}) {
    const res = await fetchFn(base + path, { ...init, headers: { 'Content-Type': 'application/json', ...(token && { Authorization: 'Bearer ' + token }) } });
    if (!res.ok) throw Object.assign(new Error('HTTP ' + res.status), { status: res.status });
    return res.json();
  }
  const book = i => ({ id: i.id, title: i.media.metadata.title, author: i.media.metadata.authorName ?? i.media.metadata.author ?? '', duration: i.media.duration || 0 });

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
    coverUrl: id => `${base}/api/items/${id}/cover?width=400&token=${encodeURIComponent(token)}`
  };
}
if (typeof module !== 'undefined') module.exports = { absClient };
