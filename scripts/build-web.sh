#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build -p fluid-web --target wasm32-unknown-unknown --release
wasm-bindgen --target web --no-typescript --out-dir platforms/web/pkg \
  target/wasm32-unknown-unknown/release/fluid_web.wasm
