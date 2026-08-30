#!/bin/bash
# Usage: scripts/run-ios.sh [Debug|Release]   FLUID_DEVICE overrides the reference device.
set -euo pipefail
cd "$(dirname "$0")/.."
config="${1:-Debug}"
device="${FLUID_DEVICE:-1B834EFE-A784-5F98-9B7A-CF6D83E2123A}"
scripts/build-ios.sh "$config"
xcrun devicectl device install app --device "$device" "platforms/ios/build/Build/Products/$config-iphoneos/FluidApp.app"
xcrun devicectl device process launch --device "$device" com.screendead.FluidApp
