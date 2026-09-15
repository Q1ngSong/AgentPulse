#!/bin/sh
# 打包 AgentPulse.app / .dmg：先编译 Hook 并放到 Tauri 的 externalBin 目录，再 tauri build
set -e
cd "$(dirname "$0")/.."
. scripts/env.sh
triple="$(rustc -vV | sed -n 's/^host: //p')"
mkdir -p src-tauri/binaries
# tauri-build 要求 externalBin 先存在，而它正是下面要编的 Hook：先放占位文件
placeholder="src-tauri/binaries/agentpulse-hook-$triple"
[ -s "$placeholder" ] || { printf '#!/bin/sh\n' > "$placeholder"; chmod +x "$placeholder"; }
cargo build --release --manifest-path src-tauri/Cargo.toml --bin agentpulse-hook
cp "src-tauri/target/release/agentpulse-hook" "src-tauri/binaries/agentpulse-hook-$triple"
pnpm tauri build "$@"
echo "产物：src-tauri/target/release/bundle/"
