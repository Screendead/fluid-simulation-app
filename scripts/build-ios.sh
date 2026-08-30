#!/bin/bash
# Usage: scripts/build-ios.sh [Debug|Release]
set -euo pipefail
cd "$(dirname "$0")/../platforms/ios"
config="${1:-Debug}"
xcodegen generate --quiet
xcodebuild -project FluidApp.xcodeproj -scheme FluidApp -configuration "$config" \
  -destination "id=${FLUID_DEVICE:-1B834EFE-A784-5F98-9B7A-CF6D83E2123A}" \
  -derivedDataPath build -allowProvisioningUpdates -allowProvisioningDeviceRegistration \
  build | grep -E 'error:|warning:|BUILD ' || true
[[ -d "build/Build/Products/$config-iphoneos/FluidApp.app" ]]
