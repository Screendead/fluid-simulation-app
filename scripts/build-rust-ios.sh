#!/bin/bash
# Xcode runs this as a pre-build phase. Its environment points host builds at
# the iOS SDK, so cargo gets a clean one.
set -euo pipefail
cd "$(dirname "$0")/.."
profile=()
[[ "${CONFIGURATION:-Release}" == Release ]] && profile=(--release)
exec env -i HOME="$HOME" PATH="$HOME/.cargo/bin:/usr/bin:/bin" \
  cargo build -p fluid-ffi --target aarch64-apple-ios "${profile[@]}"
