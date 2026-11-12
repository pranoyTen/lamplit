// Listening that could not reach the server, kept until it can. One entry per book: the time adds up, the place is the latest.
function makeListenQueue({ load, save, send }) {
  const items = load() ?? {};
  let busy = false;
  return {
    add(id, { currentTime, timeListened, duration }) {
      items[id] = { currentTime, duration, timeListened: (items[id]?.timeListened ?? 0) + timeListened };
      save(items);
    },
    size: () => Object.keys(items).length,
    // sends every entry (except skipId, the book being played right now, whose live session reports it); an entry that fails stays for next time
    async flush(skipId) {
      if (busy) return;
      busy = true;
      try {
        for (const [id, sent] of Object.entries(items)) {
          if (id === skipId) continue;
          try { await send(id, { ...sent }); } catch (e) { if (e?.status !== 404) continue; } // 404: the book is gone from the server, drop it
          const cur = items[id]; // more may have been added while sending
          if (!cur || cur.timeListened <= sent.timeListened) delete items[id]; else cur.timeListened -= sent.timeListened;
          save(items);
        }
      } finally { busy = false; }
    }
  };
}
if (typeof module !== 'undefined') module.exports = { makeListenQueue };
