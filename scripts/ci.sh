#!/usr/bin/env bash
# Runs every check CI would run, locally. Usage: pnpm verify  (or scripts/ci.sh)
set -euo pipefail
cd "$(dirname "$0")/.."

step() { printf '\n\033[1m▶ %s\033[0m\n' "$1"; }

step "Rust: format"
cargo fmt --all --check
step "Rust: clippy"
# ts-rs prints harmless "failed to parse serde attribute" notes; only show output on failure.
if ! clippy_out=$(cargo clippy --workspace --all-targets --locked --quiet -- -D warnings 2>&1); then
  echo "$clippy_out"
  exit 1
fi
step "Rust: tests"
cargo test --workspace --locked --quiet
step "Generated code is up to date"
cargo run -q -p xtask -- codegen --check 2>/dev/null
step "TypeScript: install"
pnpm install --frozen-lockfile --silent
step "TypeScript: lint"
pnpm lint
step "TypeScript: build"
pnpm build
step "TypeScript: typecheck"
pnpm typecheck
step "TypeScript: tests"
pnpm test

printf '\n\033[1;32m✔ all checks passed\033[0m\n'
