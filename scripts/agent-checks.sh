#!/usr/bin/env bash
set -euo pipefail

worktree_root=$(git rev-parse --show-toplevel)
cd "$worktree_root"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
export RUST_TEST_THREADS="${RUST_TEST_THREADS:-2}"
export CARGO_TARGET_DIR="$worktree_root/target"
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo run -p knowmesh --locked -- version
git diff --check
