// Builds the updater's latest.json for a release. The repo is private, so each file is addressed by its GitHub API asset URL
// (the app downloads it with a token). Run in CI: node scripts/latest-json.js v1.2.0 > latest.json
const { execFileSync } = require('node:child_process');

function latestJson({ tag, notes, repo, assets, sigs }) {
  const find = suffix => {
    const a = assets.find(x => x.name.endsWith(suffix));
    if (!a || !sigs[a.name]) throw new Error(`release ${tag} has no signed ${suffix} file`);
    return { signature: sigs[a.name].trim(), url: `https://api.github.com/repos/${repo}/releases/assets/${a.id}` };
  };
  const win = find('-setup.exe'), mac = find('.app.tar.gz'); // the Mac build is universal, so both chips get the same file
  return { version: tag.replace(/^v/, ''), notes: notes ?? '', pub_date: new Date().toISOString(), platforms: { 'windows-x86_64': win, 'darwin-aarch64': mac, 'darwin-x86_64': mac } };
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
