#!/usr/bin/env bash
# Local check command (ADR-0007, gate G0): runs the Linux CI checks plus `cargo deny check`.
# CI runs only on pull requests, so run this before pushing directly.
#
# Exit status:
#   0  every mandatory check ran and passed
#   1  a check ran and failed
#   2  validation incomplete: a mandatory tool is missing, so not every check could run
set -euo pipefail
cd "$(dirname "$0")/.."

run() { echo "==> $*"; "$@"; }

# Mandatory tools are verified before running anything, so a missing tool
# can never be mistaken for a passing run.
# Subcommands are probed through cargo itself, because cargo also finds them in
# $CARGO_HOME/bin when that directory is not on PATH.
missing=()
if ! command -v cargo >/dev/null 2>&1; then
    missing+=("cargo")
else
    cargo fmt --version >/dev/null 2>&1 || missing+=("rustfmt")
    cargo clippy --version >/dev/null 2>&1 || missing+=("cargo-clippy")
    cargo deny --version >/dev/null 2>&1 || missing+=("cargo-deny")
fi
if ((${#missing[@]} > 0)); then
    echo "ERROR: validation incomplete: missing mandatory tool(s): ${missing[*]}" >&2
    for tool in "${missing[@]}"; do
        case "$tool" in
            cargo-deny) echo "  cargo-deny: install with 'cargo install --locked cargo-deny' or your distribution's package." >&2 ;;
            rustfmt | cargo-clippy) echo "  $tool: install the rustfmt/clippy components of the pinned toolchain (rust-toolchain.toml)." >&2 ;;
            *) echo "  $tool: install the Rust toolchain pinned in rust-toolchain.toml." >&2 ;;
        esac
    done
    echo "No checks were run. Gate G0 is NOT passed." >&2
    exit 2
fi

run cargo fmt --all --check
run cargo clippy --workspace --all-targets -- -D warnings
run cargo test --workspace
RUSTDOCFLAGS="-D warnings" run cargo doc --workspace --no-deps
# Native backends are off by default (ADR-0002); check them explicitly.
# They load their system libraries at runtime, so no system packages are needed.
run cargo clippy -p spawn-platform --features wayland --all-targets -- -D warnings
run cargo test -p spawn-platform --features wayland
RUSTDOCFLAGS="-D warnings" run cargo doc -p spawn-platform --features wayland --no-deps
run cargo deny check

echo "All mandatory checks ran and passed."
