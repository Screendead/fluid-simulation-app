#!/bin/bash
# The whole CI gate. .github/workflows/ci.yml runs these same steps.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo fmt --all --check
# fluid-ffi is Apple-only glue (CAMetalLayer surfaces); off Apple the
# lint and test sweep covers the core, and the ios CI job compiles the
# FFI for both iOS targets.
if [[ "$(uname)" == Darwin ]]; then
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    cargo test --workspace
else
    cargo clippy -p fluid-core --all-targets --all-features -- -D warnings
    cargo test -p fluid-core
fi
generated=$(mktemp)
cbindgen --config crates/fluid-ffi/cbindgen.toml --quiet --output "$generated" crates/fluid-ffi
diff -u crates/fluid-ffi/include/fluid_ffi.h "$generated"
[[ "$(uname)" == Darwin ]] && cargo build -p fluid-ffi --target aarch64-apple-ios --target aarch64-apple-ios-sim
echo "gate: green"
