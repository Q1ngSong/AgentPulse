# 项目内工具链：source scripts/env.sh 后即可用 cargo / rustc / pnpm tauri
# Rust 工具链装在主工作目录的 .toolchain/ 下，所有 worktree 共用
_main="$(git worktree list --porcelain 2>/dev/null | sed -n '1s/^worktree //p')"
[ -n "$_main" ] || _main="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"
export RUSTUP_HOME="$_main/.toolchain/rustup"
export CARGO_HOME="$_main/.toolchain/cargo"
export PATH="$CARGO_HOME/bin:$PATH"
unset _main
