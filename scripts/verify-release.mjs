// 核对发布目录的哈希、版本和更新签名，使用 App 内同一份公钥。
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import assert from 'node:assert/strict';

const config = JSON.parse(fs.readFileSync(new URL('../src-tauri/tauri.conf.json', import.meta.url)));
const out = path.resolve(process.argv[2] ?? `release-assets/v${config.version}/macos-arm64`);
const manifest = JSON.parse(fs.readFileSync(path.join(out, 'latest.json')));
assert.equal(manifest.version, config.version, 'Version mismatch');
const platform = manifest.platforms['darwin-aarch64'];
const archive = 'AgentPulse_aarch64.app.tar.gz';
assert.equal(platform.url, `https://github.com/Q1ngSong/AgentPulse/releases/download/v${config.version}/${archive}`);
assert.equal(platform.signature, fs.readFileSync(path.join(out, `${archive}.sig`), 'utf8').trim());
const expectedFiles = [`AgentPulse_${config.version}_aarch64.dmg`, archive, `${archive}.sig`, 'latest.json'];
const checksums = fs.readFileSync(path.join(out, 'SHA256SUMS.txt'), 'utf8').trim().split('\n');
assert.equal(checksums.length, expectedFiles.length);
for (const [index, line] of checksums.entries()) {
  const [hash, file] = line.split('  ');
  assert.equal(file, expectedFiles[index]);
  assert.equal(crypto.createHash('sha256').update(fs.readFileSync(path.join(out, file))).digest('hex'), hash, file);
}
const publicLines = Buffer.from(config.plugins.updater.pubkey, 'base64').toString('utf8').trim().split('\n');
const publicBytes = Buffer.from(publicLines[1], 'base64');
const signatureLines = Buffer.from(platform.signature, 'base64').toString('utf8').trim().split('\n');
const signatureBytes = Buffer.from(signatureLines[1], 'base64');
assert.equal(publicBytes.length, 42);
assert.equal(signatureBytes.length, 74);
assert.equal(signatureBytes.subarray(0, 2).toString(), 'ED', 'Expected prehashed Minisign signature');
assert.deepEqual(signatureBytes.subarray(2, 10), publicBytes.subarray(2, 10), 'Signing key mismatch');
const key = crypto.createPublicKey({
  key: Buffer.concat([Buffer.from('302a300506032b6570032100', 'hex'), publicBytes.subarray(10)]),
  format: 'der', type: 'spki',
});
const digest = crypto.createHash('blake2b512').update(fs.readFileSync(path.join(out, archive))).digest();
const signature = signatureBytes.subarray(10);
assert(crypto.verify(null, digest, key, signature), 'Invalid archive signature');
assert(signatureLines[2].startsWith('trusted comment: '));
const comment = Buffer.from(signatureLines[2].slice('trusted comment: '.length));
assert(crypto.verify(null, Buffer.concat([signature, comment]), key, Buffer.from(signatureLines[3], 'base64')), 'Invalid trusted comment signature');
console.log('PASS: release hashes, version, download URL and both Minisign signatures');
