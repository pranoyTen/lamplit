const test = require('node:test');
const assert = require('node:assert/strict');
const { latestJson } = require('../scripts/latest-json.js');

const assets = [{ id: 11, name: 'Lectern_1.2.0_x64-setup.exe' }, { id: 12, name: 'Lectern_1.2.0_x64-setup.exe.sig' }, { id: 13, name: 'Lectern_universal.app.tar.gz' }, { id: 14, name: 'Lectern_universal.app.tar.gz.sig' }, { id: 15, name: 'Lectern_1.2.0_x64_en-US.msi' }];
const sigs = { 'Lectern_1.2.0_x64-setup.exe': 'SIGWIN\n', 'Lectern_universal.app.tar.gz': 'SIGMAC\n' };

test('points each platform at its API asset url with its signature', () => {
  const j = latestJson({ tag: 'v1.2.0', notes: 'Fixes', repo: 'me/lectern', assets, sigs });
  assert.equal(j.version, '1.2.0');
  assert.deepEqual(j.platforms['windows-x86_64'], { signature: 'SIGWIN', url: 'https://api.github.com/repos/me/lectern/releases/assets/11' });
  assert.equal(j.platforms['darwin-aarch64'].url, 'https://api.github.com/repos/me/lectern/releases/assets/13');
  assert.deepEqual(j.platforms['darwin-x86_64'], j.platforms['darwin-aarch64']);
});

test('a missing signed file fails the build instead of publishing a broken update', () => {
  assert.throws(() => latestJson({ tag: 'v1.2.0', repo: 'me/lectern', assets, sigs: { 'Lectern_1.2.0_x64-setup.exe': 'x' } }), /app\.tar\.gz/);
});
