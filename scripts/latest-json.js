// Builds the updater's latest.json for a release. Each file is addressed by its GitHub API asset URL, which also works with a token
// while the repository is private. Run in CI: node scripts/latest-json.js v1.2.0 > latest.json
const { execFileSync } = require('node:child_process');

// `format` tells the native app's updater (cargo-packager-updater) how to install the file; the Tauri updater ignores it.
function latestJson({ tag, notes, repo, assets, sigs }) {
  const find = (suffix, format) => {
    const a = assets.find(x => x.name.endsWith(suffix));
    if (!a || !sigs[a.name]) throw new Error(`release ${tag} has no signed ${suffix} file`);
    return { signature: sigs[a.name].trim(), url: `https://api.github.com/repos/${repo}/releases/assets/${a.id}`, format };
  };
  const win = find('-setup.exe', 'nsis');
  // the native app builds each Mac chip on its own runner (Lamplit_aarch64 / Lamplit_x64); the Tauri app built one universal file
  const perChip = assets.some(x => x.name.endsWith('_aarch64.app.tar.gz'));
  const arm = find(perChip ? '_aarch64.app.tar.gz' : '.app.tar.gz', 'app');
  const intel = perChip ? find('_x64.app.tar.gz', 'app') : arm;
  // the native updater calls a Mac 'macos', the Tauri app (1.x) calls it 'darwin'
  return { version: tag.replace(/^v/, ''), notes: notes ?? '', pub_date: new Date().toISOString(), platforms: { 'windows-x86_64': win, 'macos-aarch64': arm, 'macos-x86_64': intel, 'darwin-aarch64': arm, 'darwin-x86_64': intel } };
}

if (require.main === module) {
  const gh = (...a) => execFileSync('gh', a, { encoding: 'utf8', maxBuffer: 1 << 28 });
  const tag = process.argv[2], repo = process.env.GITHUB_REPOSITORY;
  const release = JSON.parse(gh('api', `repos/${repo}/releases`, '--paginate', '--jq', `map(select(.tag_name == "${tag}"))[0]`));
  const sigs = {};
  for (const a of release.assets.filter(x => x.name.endsWith('.sig'))) sigs[a.name.slice(0, -4)] = gh('api', '-H', 'Accept: application/octet-stream', `repos/${repo}/releases/assets/${a.id}`);
  process.stdout.write(JSON.stringify(latestJson({ tag, notes: release.body, repo, assets: release.assets, sigs }), null, 2));
}
module.exports = { latestJson };
