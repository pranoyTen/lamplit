// Update check and install on top of Tauri's updater plugin. The repo is private, so every request carries a GitHub token.
// `check` and `relaunch` are the plugin's own functions; onState gets { s: 'checking' | 'current' | 'available' | 'downloading' | 'error' | 'notoken', ... }.
function explainUpdateError(e) {
  const m = String(e?.message ?? e);
  if (/40[134]|not found|bad credentials/i.test(m)) return "Couldn't read the update file. Check that the token is valid and a release has been published";
  if (/network|connect|dns|timed out|error sending request|failed to fetch/i.test(m)) return "Couldn't reach GitHub. Check your connection";
  if (/signature/i.test(m)) return 'The update did not pass its signature check, so it was not installed';
  return "Couldn't update: " + m;
}

function makeUpdater({ check, relaunch, token, onState }) {
  let found = null;
  const opts = () => ({ headers: { Authorization: 'Bearer ' + token() } });
  return {
    async check() {
      if (!token()) return onState({ s: 'notoken' });
      onState({ s: 'checking' });
      try {
        found = await check(opts());
        onState(found ? { s: 'available', version: found.version, notes: found.body ?? '' } : { s: 'current' });
      } catch (e) { onState({ s: 'error', msg: explainUpdateError(e) }); }
    },
    async install() {
      if (!found) return;
      let total = 0, got = 0;
      onState({ s: 'downloading', pct: 0 });
      try {
        await found.downloadAndInstall(ev => {
          if (ev.event === 'Started') total = ev.data.contentLength ?? 0;
          if (ev.event === 'Progress') { got += ev.data.chunkLength; onState({ s: 'downloading', pct: total ? Math.min(1, got / total) : 0 }); }
        }, opts());
        await relaunch();
      } catch (e) { onState({ s: 'error', msg: explainUpdateError(e) }); }
    }
  };
}
if (typeof module !== 'undefined') module.exports = { makeUpdater, explainUpdateError };
