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
udids = [d["udid"] for runtime, ds in devices.items() if "iOS" in runtime for d in ds if "iPhone" in d["name"]]
print(udids[0] if udids else "")')
if [[ -z "$sim" ]]; then
  devtype=$(xcrun simctl list devicetypes | grep -oE 'com.apple.CoreSimulator.SimDeviceType.iPhone-1[5-9][^)]*' | tail -1)
  runtime=$(xcrun simctl list runtimes -j | python3 -c 'import json,sys; print(json.load(sys.stdin)["runtimes"][-1]["identifier"])')
  sim=$(xcrun simctl create FluidSim "$devtype" "$runtime")
fi
xcrun simctl boot "$sim" 2>/dev/null || true
open -a Simulator
xcrun simctl install "$sim" "platforms/ios/build/Build/Products/$config-iphonesimulator/FluidApp.app"
SIMCTL_CHILD_FLUID_PARTICLES="${FLUID_PARTICLES:-}" \
SIMCTL_CHILD_FLUID_RADIUS="${FLUID_RADIUS:-}" \
SIMCTL_CHILD_FLUID_BENCH="${FLUID_BENCH:-}" \
SIMCTL_CHILD_FLUID_SPACING="${FLUID_SPACING:-}" \
SIMCTL_CHILD_FLUID_SIM="${FLUID_SIM:-}" \
xcrun simctl launch "$sim" com.screendead.FluidApp
