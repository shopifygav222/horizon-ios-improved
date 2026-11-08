#!/bin/bash
# The app's start in the iOS Simulator, checked before any phone sees it (IOS-SYNC.md): it is installed
# in a fresh iPhone simulator and launched, and after a while its logs must show Starling's own start
# (its log opened, the iOS layer's start line, the renderer starting or saying why it can't), with no
# panic in stderr and no crash report. GitHub's macOS machines may give the Simulator no Metal device:
# then the renderer's "No Metal graphics device" is what the start reaches, which still proves every
# step before it.
#   tools/ci/ios_simulator_smoke.sh path/to/Starling.app
set -euo pipefail
app=$1
bundle=dev.horizon.starling
device=$(xcrun simctl list devices available -j | python3 -c '
import json, sys
devices = json.load(sys.stdin)["devices"]
phones = [d for runtime, list_ in devices.items() if "iOS" in runtime for d in list_ if d["name"].startswith("iPhone")]
print(phones[-1]["udid"] if phones else "")
')
test -n "$device" || { echo '::error::no iPhone simulator on this machine'; exit 1; }
echo "Simulator: $device"
xcrun simctl boot "$device" || true
xcrun simctl bootstatus "$device" -b
xcrun simctl install "$device" "$app"
started=$(date +%s)
xcrun simctl launch "$device" "$bundle"
sleep 120
data=$(xcrun simctl get_app_container "$device" "$bundle" data)
logs="$data/Documents/logs"
echo "::group::stderr.log"; cat "$logs/stderr.log" 2>/dev/null || true; echo "::endgroup::"
echo "::group::latest.log"; cat "$logs/latest.log" 2>/dev/null || true; echo "::endgroup::"
failed=0
if grep -q 'panicked at' "$logs/stderr.log" 2>/dev/null; then
  echo '::error::the app panicked in the Simulator (stderr.log above)'
  failed=1
fi
if ! grep -q 'Horizon: iOS, game folder' "$logs/latest.log" 2>/dev/null; then
  echo "::error::the game's start did not reach its log (latest.log above)"
  failed=1
fi
if ! grep -qE 'Horizon renderer:|No Metal graphics device' "$logs/latest.log" 2>/dev/null; then
  echo "::error::the game did not reach its renderer's start (latest.log above)"
  failed=1
fi
crashes=$(find "$HOME/Library/Logs/DiagnosticReports" -newermt "@$started" -name 'Starling*' 2>/dev/null || true)
if [ -n "$crashes" ]; then
  for report in $crashes; do echo "::group::$report"; head -120 "$report"; echo "::endgroup::"; done
  echo '::error::the app crashed in the Simulator (reports above)'
  failed=1
fi
if xcrun simctl spawn "$device" launchctl list | grep -q "UIKitApplication:$bundle"; then
  echo 'The app is still running.'
else
  echo '::warning::the app is no longer running'
fi
xcrun simctl shutdown "$device" || true
exit $failed
