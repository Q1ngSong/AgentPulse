// 将各平台安装包汇总到同一版本目录；拒绝混入不同提交、版本或损坏的文件。
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const platforms = ['macos-arm64', 'windows-x64'];
const json = file => JSON.parse(fs.readFileSync(file, 'utf8'));
const git = (...args) => execFileSync('git', args, { cwd: repo, encoding: 'utf8' }).trim();
const hash = file => crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');

export function releaseInfo(root = repo) {
  const version = json(path.join(root, 'package.json')).version;
  assert.match(version, /^\d+\.\d+\.\d+(?:-[a-zA-Z0-9.-]+)?$/);
  assert.equal(json(path.join(root, 'src-tauri/tauri.conf.json')).version, version, 'Tauri version mismatch');
  const cargo = fs.readFileSync(path.join(root, 'src-tauri/Cargo.toml'), 'utf8');
  assert.equal(cargo.match(/^version\s*=\s*"([^"]+)"/m)?.[1], version, 'Cargo version mismatch');
  const lock = fs.readFileSync(path.join(root, 'src-tauri/Cargo.lock'), 'utf8');
  assert.equal(lock.match(/\[\[package\]\]\r?\nname = "agentpulse"\r?\nversion = "([^"]+)"/)?.[1], version, 'Cargo.lock version mismatch');
  return { version, tag: `v${version}` };
}

function filenames(version, platform, msi) {
  assert(platforms.includes(platform), `Unsupported platform: ${platform}`);
  if (platform === 'macos-arm64') return [`AgentPulse_${version}_aarch64.dmg`];
  return [`AgentPulse_${version}_x64-setup.exe`, ...(msi ? [`AgentPulse_${version}_x64_en-US.msi`] : [])];
}

export function verifyManifest(dir, manifest, expected) {
  for (const key of ['version', 'commit', 'platform']) assert.equal(manifest[key], expected[key], `${key} mismatch`);
  assert.deepEqual(manifest.files.map(file => file.name).sort(), filenames(expected.version, expected.platform, expected.msi).sort(), 'Unexpected installer set');
  for (const file of manifest.files) {
    const source = path.join(dir, file.name);
    const bytes = fs.readFileSync(source);
    assert.equal(bytes.length, file.bytes, `Size mismatch: ${file.name}`);
    assert.equal(hash(source), file.sha256, `SHA-256 mismatch: ${file.name}`);
    if (file.name.endsWith('.dmg')) assert.equal(bytes.subarray(-512, -508).toString(), 'koly', 'Invalid DMG');
    if (file.name.endsWith('.exe')) assert.equal(bytes.subarray(0, 2).toString(), 'MZ', 'Invalid Windows executable');
    if (file.name.endsWith('.msi')) assert.equal(bytes.subarray(0, 8).toString('hex'), 'd0cf11e0a1b11ae1', 'Invalid MSI');
  }
}

export function assemble(input, output, info, msi = false) {
  const manifests = platforms.map(platform => {
    const dir = path.join(input, platform);
    const manifest = json(path.join(dir, 'build-info.json'));
    verifyManifest(dir, manifest, { ...info, platform, msi });
    return { dir, manifest };
  });
  // 所有平台验证成功后再写最终目录，防止部分产物被当成完整发布。
  fs.mkdirSync(output, { recursive: true });
  assert.equal(fs.readdirSync(output).length, 0, 'Final release directory must be empty');
  const files = [];
  for (const { dir, manifest } of manifests) for (const file of manifest.files) {
    fs.copyFileSync(path.join(dir, file.name), path.join(output, file.name));
    files.push(file.name);
  }
  fs.writeFileSync(path.join(output, 'BUILD-INFO.json'), JSON.stringify({ ...info, platforms: manifests.map(({ manifest }) => manifest) }, null, 2) + '\n');
  files.push('BUILD-INFO.json');
  fs.writeFileSync(path.join(output, 'SHA256SUMS.txt'), files.sort().map(file => `${hash(path.join(output, file))}  ${file}\n`).join(''));
  return files;
}

async function main() {
  const [command, argument] = process.argv.slice(2);
  const info = { ...releaseInfo(), commit: git('rev-parse', 'HEAD') };
  const msi = process.env.INCLUDE_MSI === 'true';
  const base = path.join(repo, 'release-assets', info.tag);
  if (command === 'check') {
    if (process.env.RELEASE_TAG) assert.equal(process.env.RELEASE_TAG, info.tag, 'Tag must match application version');
    assert.equal(git('status', '--porcelain', '--untracked-files=no'), '', 'Commit source changes before building a release');
    const existingTag = git('tag', '--list', info.tag);
    if (existingTag) assert.equal(git('rev-parse', `${info.tag}^{commit}`), info.commit, 'Existing version tag points to different source');
    if (process.env.GITHUB_ACTIONS) {
      const response = await fetch(`https://api.github.com/repos/${process.env.GITHUB_REPOSITORY}/releases/tags/${info.tag}`, {
        headers: { Authorization: `Bearer ${process.env.GH_TOKEN}`, Accept: 'application/vnd.github+json' },
      });
      if (response.status !== 404) {
        assert(response.ok, `Cannot inspect existing release: HTTP ${response.status}`);
        const release = await response.json();
        assert(release.draft, 'This version is already published; bump the version before releasing again');
        assert.equal(release.target_commitish, info.commit, 'Existing draft belongs to different source');
      }
    }
    const rust = fs.readFileSync(path.join(repo, 'rust-toolchain.toml'), 'utf8').match(/^channel\s*=\s*"([^"]+)"/m)?.[1];
    assert(rust, 'Missing Rust toolchain');
    if (process.env.GITHUB_OUTPUT) fs.appendFileSync(process.env.GITHUB_OUTPUT, Object.entries({ ...info, rust }).map(([k, v]) => `${k}=${v}\n`).join(''));
    console.log(JSON.stringify({ ...info, rust }));
  } else if (command === 'collect') {
    const platform = argument;
    assert.equal(process.platform, platform === 'macos-arm64' ? 'darwin' : 'win32', 'Collect on the build platform');
    assert.equal(process.arch, platform === 'macos-arm64' ? 'arm64' : 'x64', 'Unexpected build architecture');
    const dir = path.join(base, platform);
    fs.mkdirSync(dir, { recursive: true });
    const names = filenames(info.version, platform, msi);
    for (const name of names) if (platform === 'windows-x64') {
      const type = name.endsWith('.msi') ? 'msi' : 'nsis';
      fs.copyFileSync(path.join(repo, 'src-tauri/target/release/bundle', type, name), path.join(dir, name));
    }
    const manifest = { ...info, platform, files: names.map(name => ({ name, bytes: fs.statSync(path.join(dir, name)).size, sha256: hash(path.join(dir, name)) })) };
    verifyManifest(dir, manifest, { ...info, platform, msi });
    fs.writeFileSync(path.join(dir, 'build-info.json'), JSON.stringify(manifest, null, 2) + '\n');
    console.log(`Verified installers: ${dir}`);
  } else if (command === 'assemble') {
    assemble(path.resolve(argument), path.join(base, 'all'), info, msi);
    console.log(`Release ready: ${path.join(base, 'all')}`);
  } else throw new Error('Usage: node scripts/release-artifacts.mjs check | collect <macos-arm64|windows-x64> | assemble <downloaded-artifacts>');
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch(error => { console.error(error.message); process.exitCode = 1; });
}
