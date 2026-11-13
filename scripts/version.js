// Sets the app version everywhere it lives: node scripts/version.js 0.2.0  (or: npm run version:set -- 0.2.0)
const fs = require('fs');
const path = require('path');

const root = path.join(__dirname, '..');
const edit = (file, fn) => { const p = path.join(root, file), s = fs.readFileSync(p, 'utf8'); fs.writeFileSync(p, fn(s)); };
const SEMVER = /^\d+\.\d+\.\d+$/;

if (require.main === module) {
  const v = process.argv[2];
  if (!SEMVER.test(v ?? '')) { console.error('Usage: node scripts/version.js <major.minor.patch>, e.g. 0.2.0'); process.exit(1); }
  edit('package.json', s => s.replace(/("version": ")[^"]+/, `$1${v}`));
  edit('src-tauri/tauri.conf.json', s => s.replace(/("version": ")[^"]+/, `$1${v}`));
  edit('src-tauri/Cargo.toml', s => s.replace(/(\[package\][^[]*?\nversion = ")[^"]+/, `$1${v}`));
  edit('src-tauri/Cargo.lock', s => s.replace(/(name = "lectern"\r?\nversion = ")[^"]+/, `$1${v}`));
  console.log(`Version set to ${v}. Commit it, then tag the release: git tag v${v}`);
}
module.exports = { SEMVER };
