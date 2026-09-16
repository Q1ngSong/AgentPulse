import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import crypto from 'node:crypto';
import { assemble, releaseInfo } from './release-artifacts.mjs';

const info = { version: '0.1.1', tag: 'v0.1.1', commit: 'a'.repeat(40) };
const sha256 = bytes => crypto.createHash('sha256').update(bytes).digest('hex');
function fixture(t, msi = false) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'agentpulse-release-'));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const input = path.join(root, 'input');
  const output = path.join(root, 'all');
  const dmg = Buffer.alloc(512); dmg.write('koly');
  const packages = {
    'macos-arm64': { 'AgentPulse_0.1.1_aarch64.dmg': dmg },
    'windows-x64': { 'AgentPulse_0.1.1_x64-setup.exe': Buffer.from('MZtest fixture') },
  };
  if (msi) packages['windows-x64']['AgentPulse_0.1.1_x64_en-US.msi'] = Buffer.from('d0cf11e0a1b11ae1', 'hex');
  for (const [platform, files] of Object.entries(packages)) {
    const dir = path.join(input, platform);
    fs.mkdirSync(dir, { recursive: true });
    const manifest = { ...info, platform, files: Object.entries(files).map(([name, bytes]) => {
      fs.writeFileSync(path.join(dir, name), bytes);
      return { name, bytes: bytes.length, sha256: sha256(bytes) };
    }) };
    fs.writeFileSync(path.join(dir, 'build-info.json'), JSON.stringify(manifest));
  }
  const changeManifest = change => {
    const file = path.join(input, 'windows-x64/build-info.json');
    const manifest = JSON.parse(fs.readFileSync(file));
    change(manifest);
    fs.writeFileSync(file, JSON.stringify(manifest));
  };
  return { root, input, output, changeManifest };
}

for (const msi of [false, true]) test(`assemble both platforms, MSI=${msi}`, t => {
  const { input, output } = fixture(t, msi);
  assemble(input, output, info, msi);
  const sums = fs.readFileSync(path.join(output, 'SHA256SUMS.txt'), 'utf8').trim().split('\n');
  assert.equal(sums.length, msi ? 4 : 3);
  for (const line of sums) {
    const [hash, file] = line.split('  ');
    assert.equal(sha256(fs.readFileSync(path.join(output, file))), hash);
  }
  const build = JSON.parse(fs.readFileSync(path.join(output, 'BUILD-INFO.json')));
  assert.equal(build.commit, info.commit);
  assert.equal(build.platforms.length, 2);
});

for (const [name, change, error] of [
  ['different source', manifest => { manifest.commit = 'b'.repeat(40); }, /commit mismatch/],
  ['different version', manifest => { manifest.version = '0.1.2'; }, /version mismatch/],
  ['wrong platform', manifest => { manifest.platform = 'macos-arm64'; }, /platform mismatch/],
  ['unsafe filename', manifest => { manifest.files[0].name = '../payload.exe'; }, /Unexpected installer set/],
  ['duplicate installer', manifest => { manifest.files.push(manifest.files[0]); }, /Unexpected installer set/],
]) test(`reject ${name} before writing final assets`, t => {
  const { input, output, changeManifest } = fixture(t);
  changeManifest(change);
  assert.throws(() => assemble(input, output, info), error);
  assert.equal(fs.existsSync(output), false);
});

test('reject altered bytes even when the size is unchanged', t => {
  const { input, output } = fixture(t);
  fs.writeFileSync(path.join(input, 'windows-x64/AgentPulse_0.1.1_x64-setup.exe'), 'MZtest fixturX');
  assert.throws(() => assemble(input, output, info), /SHA-256 mismatch/);
  assert.equal(fs.existsSync(output), false);
});

test('reject a missing MSI when requested', t => {
  const { input, output } = fixture(t);
  assert.throws(() => assemble(input, output, info, true), /Unexpected installer set/);
});

test('do not overwrite an existing final directory', t => {
  const { input, output } = fixture(t);
  fs.mkdirSync(output);
  fs.writeFileSync(path.join(output, 'keep'), 'existing');
  assert.throws(() => assemble(input, output, info), /must be empty/);
  assert.equal(fs.readFileSync(path.join(output, 'keep'), 'utf8'), 'existing');
});

test('require matching app, Cargo and lockfile versions', t => {
  const { root } = fixture(t);
  fs.mkdirSync(path.join(root, 'src-tauri'));
  fs.writeFileSync(path.join(root, 'package.json'), JSON.stringify({ version: info.version }));
  fs.writeFileSync(path.join(root, 'src-tauri/tauri.conf.json'), JSON.stringify({ version: info.version }));
  fs.writeFileSync(path.join(root, 'src-tauri/Cargo.toml'), '[package]\nversion = "0.1.1"\n');
  fs.writeFileSync(path.join(root, 'src-tauri/Cargo.lock'), '[[package]]\nname = "agentpulse"\nversion = "0.1.1"\n');
  assert.deepEqual(releaseInfo(root), { version: info.version, tag: info.tag });
  fs.writeFileSync(path.join(root, 'src-tauri/Cargo.lock'), '[[package]]\nname = "agentpulse"\nversion = "0.1.0"\n');
  assert.throws(() => releaseInfo(root), /Cargo.lock version mismatch/);
});
