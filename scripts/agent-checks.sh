#!/usr/bin/env bash
set -euo pipefail

worktree_root=$(git rev-parse --show-toplevel)
git_common_dir=$(git rev-parse --path-format=absolute --git-common-dir)
integration_root=$(dirname "$git_common_dir")
coordination_dir="$integration_root/.worktrees/.coordination"

command -v flock >/dev/null || {
  printf '%s\n' 'flock is required on this shared Linux development host.' >&2
  exit 1
}
mkdir -p "$coordination_dir"
exec 9>"$coordination_dir/full-check.lock"
printf '%s\n' 'Waiting for the shared full-check slot...' >&2
flock -x 9

cd "$worktree_root"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
export RUST_TEST_THREADS="${RUST_TEST_THREADS:-2}"
export CARGO_TARGET_DIR="$worktree_root/target"
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo run -p knowmesh --locked -- version
git diff --check
