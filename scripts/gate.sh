#!/bin/bash
# The whole CI gate. .github/workflows/ci.yml runs these same steps.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p fluid-web --target wasm32-unknown-unknown -- -D warnings
cargo test --workspace
cargo build -p fluid-web --target wasm32-unknown-unknown
generated=$(mktemp)
cbindgen --config crates/fluid-ffi/cbindgen.toml --quiet --output "$generated" crates/fluid-ffi
diff -u crates/fluid-ffi/include/fluid_ffi.h "$generated"
[[ "$(uname)" == Darwin ]] && cargo build -p fluid-ffi --target aarch64-apple-ios --target aarch64-apple-ios-sim
echo "gate: green"
