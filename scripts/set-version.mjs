#!/usr/bin/env node
// 单一版本入口：同步 package.json、Tauri、Cargo、Cargo.lock 和官网 Release 链接。
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { releaseInfo } from './release-artifacts.mjs';

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const version = process.argv[2];

function usage() {
  console.error('Usage: node scripts/set-version.mjs <semver>');
  console.error('Example: node scripts/set-version.mjs 0.1.2');
  process.exit(1);
}

if (!version || !/^\d+\.\d+\.\d+(?:-[a-zA-Z0-9.-]+)?$/.test(version)) usage();

const tag = `v${version}`;
const read = file => fs.readFileSync(path.join(repo, file), 'utf8');
const write = (file, text) => fs.writeFileSync(path.join(repo, file), text);

function updateJson(file, edit) {
  const full = path.join(repo, file);
  const json = JSON.parse(fs.readFileSync(full, 'utf8'));
  edit(json);
  fs.writeFileSync(full, `${JSON.stringify(json, null, 2)}\n`);
}

function replaceOnce(file, pattern, replacement, label) {
  const text = read(file);
  if (!pattern.test(text)) throw new Error(`${label} not found in ${file}`);
  const next = text.replace(pattern, replacement);
  write(file, next);
}

updateJson('package.json', json => { json.version = version; });
updateJson('src-tauri/tauri.conf.json', json => { json.version = version; });

replaceOnce('src-tauri/Cargo.toml', /^version\s*=\s*"[^"]+"/m, `version = "${version}"`, 'Cargo package version');
replaceOnce('src-tauri/Cargo.lock', /(\[\[package\]\]\r?\nname = "agentpulse"\r?\nversion = ")[^"]+"/, `$1${version}"`, 'Cargo.lock agentpulse version');

for (const file of ['site/index.html', 'site/guide/index.html']) {
  write(file, read(file).replace(/releases\/tag\/v\d+\.\d+\.\d+(?:-[a-zA-Z0-9.-]+)?/g, `releases/tag/${tag}`));
}

// 文档里的示例目录跟随当前版本，避免下次照着旧路径操作。
write('README.md', read('README.md').replace(/release-assets\/v\d+\.\d+\.\d+(?:-[a-zA-Z0-9.-]+)?/g, `release-assets/${tag}`));

console.log(JSON.stringify(releaseInfo(repo)));
