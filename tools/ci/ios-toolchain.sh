#!/bin/bash
# Fail before compiling Rust when the runner's Xcode cannot build for the phone.
set -euo pipefail
xcodebuild -version
sdk=$(xcrun --sdk iphoneos --show-sdk-version)
printf 'iPhoneOS SDK: %s\n' "$sdk"
major=${sdk%%.*}
# The app's minimum is iOS 16 (packaging/ios/project.yml); an older SDK can't build for it.
if ! [[ "$major" =~ ^[0-9]+$ ]] || (( major < 16 )); then
  echo '::error::The selected iPhoneOS SDK must be 16 or newer. Check DEVELOPER_DIR and xcode-select.' >&2
  exit 1
fi
