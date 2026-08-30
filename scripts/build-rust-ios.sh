#!/bin/bash
# Xcode runs this as a pre-build phase and exports FLUID_RUST_TARGET and
# FLUID_RUST_PROFILE from project.yml. Its environment points host builds at
# the iOS SDK, so cargo gets a clean one.
set -euo pipefail
cd "$(dirname "$0")/.."
profile=()
[[ "$FLUID_RUST_PROFILE" == release ]] && profile=(--release)
exec env -i HOME="$HOME" PATH="$HOME/.cargo/bin:/usr/bin:/bin" \
  cargo build -p fluid-ffi --target "$FLUID_RUST_TARGET" "${profile[@]}"
