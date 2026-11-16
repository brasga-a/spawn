#!/usr/bin/env bash
# Local check command (ADR-0007): runs the same checks as the Linux CI job.
# CI runs only on pull requests, so run this before pushing directly.
set -euo pipefail
cd "$(dirname "$0")/.."

run() { echo "==> $*"; "$@"; }

run cargo fmt --all --check
run cargo clippy --workspace --all-targets -- -D warnings
run cargo test --workspace
RUSTDOCFLAGS="-D warnings" run cargo doc --workspace --no-deps

if command -v cargo-deny >/dev/null 2>&1; then
    run cargo deny check
else
    echo "==> cargo-deny not installed; skipping license/advisory check (runs in CI)."
fi

echo "All checks passed."
