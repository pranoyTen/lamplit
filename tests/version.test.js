const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { SEMVER } = require('../scripts/version.js');

const read = f => fs.readFileSync(path.join(__dirname, '..', f), 'utf8');

test('the app version is one semver number, the same in every file that holds it', () => {
  const found = {
    'package.json': JSON.parse(read('package.json')).version,
    'tauri.conf.json': JSON.parse(read('src-tauri/tauri.conf.json')).version,
    'Cargo.toml': /\[package\][^[]*?\nversion = "([^"]+)"/.exec(read('src-tauri/Cargo.toml'))?.[1],
    'Cargo.lock': /name = "lectern"\r?\nversion = "([^"]+)"/.exec(read('src-tauri/Cargo.lock'))?.[1]
  };
  const v = found['package.json'];
  assert.match(v, SEMVER);
  assert.deepEqual(found, { 'package.json': v, 'tauri.conf.json': v, 'Cargo.toml': v, 'Cargo.lock': v });
});
