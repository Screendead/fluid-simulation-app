#!/bin/bash
# Usage: scripts/run-sim.sh [Debug|Release]
# Proves the shell compiles, links and launches. The simulator has no motion
# sensors, so it proves nothing else; measure on the reference device.
set -euo pipefail
cd "$(dirname "$0")/.."
config="${1:-Debug}"
scripts/build-ios.sh "$config" sim
sim=$(xcrun simctl list devices available -j | python3 -c '
import json, sys
devices = json.load(sys.stdin)["devices"]
print(next(d["udid"] for runtime, ds in devices.items() if "iOS" in runtime for d in ds if "iPhone" in d["name"]))')
xcrun simctl boot "$sim" 2>/dev/null || true
open -a Simulator
xcrun simctl install "$sim" "platforms/ios/build/Build/Products/$config-iphonesimulator/FluidApp.app"
xcrun simctl launch "$sim" com.screendead.FluidApp
