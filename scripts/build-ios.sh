#!/bin/bash
# Usage: scripts/build-ios.sh [Debug|Release] [device|sim]
set -euo pipefail
cd "$(dirname "$0")/../platforms/ios"
config="${1:-Debug}"
case "${2:-device}" in
  device) destination="generic/platform=iOS"; signing=(); sdk=iphoneos ;;
  sim) destination="generic/platform=iOS Simulator"; signing=(CODE_SIGNING_ALLOWED=NO); sdk=iphonesimulator ;;
  *) echo "unknown target: $2" >&2; exit 2 ;;
esac
xcodegen generate --quiet
xcodebuild -project FluidApp.xcodeproj -scheme FluidApp -configuration "$config" \
  -destination "$destination" -derivedDataPath build \
  -allowProvisioningUpdates -allowProvisioningDeviceRegistration ${signing[@]+"${signing[@]}"} \
  build | grep -E 'error:|warning:|BUILD ' || true
[[ -x "build/Build/Products/$config-$sdk/FluidApp.app/FluidApp" ]]
