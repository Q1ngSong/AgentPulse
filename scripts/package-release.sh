#!/bin/bash
# 从已编译的 App 创建 DMG；--installer-only 不生成自动更新包。不会自动上传。
set -euo pipefail
cd "$(dirname "$0")/.."
version="$(node -p 'require("./package.json").version')"
app="$(pwd)/src-tauri/target/release/bundle/macos/AgentPulse.app"
release_dir="$(pwd)/release-assets/v${version}/macos-arm64"
installer_only=false
case "${1:-}" in
  --installer-only) installer_only=true ;;
  '') ;;
  *) echo 'Usage: scripts/package-release.sh [--installer-only]' >&2; exit 1 ;;
esac
test -d "$app"
if ! "$installer_only"; then
  test -n "${TAURI_SIGNING_PRIVATE_KEY:-}" || { echo 'Set TAURI_SIGNING_PRIVATE_KEY to the local key path first.' >&2; exit 1; }
fi
architecture="$(lipo -archs "$app/Contents/MacOS/agentpulse")"
test "$architecture" = arm64 || { echo "Unsupported release architecture: $architecture" >&2; exit 1; }
test "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$app/Contents/Info.plist")" = "$version"
mkdir -p "$release_dir"
staging="$(mktemp -d "$release_dir/staging.XXXXXX")"
trap 'rm -rf "$staging"' EXIT
cp -R "$app" "$staging/AgentPulse.app"
ln -s /Applications "$staging/Applications"
# 只清理发布副本的扩展属性；图标和其他资源保持原始字节。
xattr -cr "$staging/AgentPulse.app"
codesign --force --deep --sign - "$staging/AgentPulse.app"
codesign --verify --deep --strict --verbose=2 "$staging/AgentPulse.app"
dmg="$release_dir/AgentPulse_${version}_aarch64.dmg"
hdiutil create -volname AgentPulse -srcfolder "$staging" -ov -format UDZO "$dmg"
shasum -a 256 "$dmg"
if "$installer_only"; then
  node scripts/release-artifacts.mjs collect macos-arm64
  exit 0
fi
# 更新包包含同一份经过验证的 App，不含 DMG 的 Applications 快捷方式。
COPYFILE_DISABLE=1 tar -czf "$release_dir/AgentPulse_aarch64.app.tar.gz" -C "$staging" AgentPulse.app
if [ -f "$TAURI_SIGNING_PRIVATE_KEY" ]; then
  env -u TAURI_SIGNING_PRIVATE_KEY pnpm tauri signer sign -f "$TAURI_SIGNING_PRIVATE_KEY" "$release_dir/AgentPulse_aarch64.app.tar.gz"
else
  pnpm tauri signer sign "$release_dir/AgentPulse_aarch64.app.tar.gz"
fi
node --input-type=module - "$release_dir" "$version" <<'JS'
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
const [out, version] = process.argv.slice(2);
const archive = 'AgentPulse_aarch64.app.tar.gz';
const manifest = {
  version,
  notes: 'AgentPulse：Claude Code / Codex 的本机提醒与延迟远程推送。',
  pub_date: new Date().toISOString(),
  platforms: { 'darwin-aarch64': {
    signature: fs.readFileSync(path.join(out, `${archive}.sig`), 'utf8').trim(),
    url: `https://github.com/Q1ngSong/AgentPulse/releases/download/v${version}/${archive}`,
  } },
};
fs.writeFileSync(path.join(out, 'latest.json'), JSON.stringify(manifest, null, 2) + '\n');
const files = [`AgentPulse_${version}_aarch64.dmg`, archive, `${archive}.sig`, 'latest.json'];
fs.writeFileSync(path.join(out, 'SHA256SUMS.txt'), files.map(file =>
  `${crypto.createHash('sha256').update(fs.readFileSync(path.join(out, file))).digest('hex')}  ${file}\n`
).join(''));
console.log(`Release artifacts: ${out}`);
JS
node scripts/verify-release.mjs "$release_dir"
