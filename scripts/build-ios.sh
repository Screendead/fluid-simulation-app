#!/bin/bash
# Usage: scripts/build-ios.sh [Debug|Release] [device|sim]
set -euo pipefail
cd "$(dirname "$0")/../platforms/ios"
config="${1:-Debug}"
case "${2:-device}" in
  device) destination="id=${FLUID_DEVICE:-1B834EFE-A784-5F98-9B7A-CF6D83E2123A}"; signing=(); sdk=iphoneos ;;
  sim) destination="generic/platform=iOS Simulator"; signing=(CODE_SIGNING_ALLOWED=NO); sdk=iphonesimulator ;;
  *) echo "unknown target: $2" >&2; exit 2 ;;
esac
xcodegen generate --quiet
xcodebuild -project FluidApp.xcodeproj -scheme FluidApp -configuration "$config" \
  -destination "$destination" -derivedDataPath build \
  -allowProvisioningUpdates -allowProvisioningDeviceRegistration "${signing[@]}" \
  build | grep -E 'error:|warning:|BUILD ' || true
[[ -d "build/Build/Products/$config-$sdk/FluidApp.app" ]]
