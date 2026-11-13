#!/bin/bash
# The app's start in the iOS Simulator, checked before any phone sees it (IOS-SYNC.md): it is installed
# in a fresh iPhone simulator and launched, and after a while its logs must show Starling's own start
# (its log opened, the iOS layer's start line, the renderer starting or saying why it can't), with no
# panic in stderr and no crash report. GitHub's macOS machines may give the Simulator no Metal device:
# then the renderer's "No Metal graphics device" is what the start reaches, which still proves every
# step before it.
#   tools/ci/ios_simulator_smoke.sh path/to/HorizonPE.app
set -euo pipefail
app=$1
bundle=dev.horizon.pocketedition
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
# The Simulator's GPU is not of Apple's family 4, so wgpu gives it 15 inter-stage shader variables
# where every iPhone iOS 16 runs on (A11 and later) has 31: the renderer's pipelines stop there,
# in the Simulator only. Any other panic is the app's.
simulator_limit='max_inter_stage_shader_variables` limit (15)'
simulator_stop=0
if grep -qF "$simulator_limit" "$logs/stderr.log" 2>/dev/null; then
  simulator_stop=1
  echo "The Simulator's own shader-variable limit stopped the renderer's pipelines, as expected there."
elif grep -q 'panicked at' "$logs/stderr.log" 2>/dev/null; then
  echo '::error::the app panicked in the Simulator (stderr.log above)'
  failed=1
fi
if ! grep -q 'Horizon renderer: creating the pipelines' "$logs/latest.log" 2>/dev/null; then
  echo "::error::the game did not reach its renderer's pipelines (latest.log above)"
  failed=1
fi
if ! grep -qE 'Horizon: iOS, game folder' "$logs/latest.log" 2>/dev/null; then
  echo "::error::Starling's start did not reach its log (latest.log above)"
  failed=1
fi
crashes=$(find "$HOME/Library/Logs/DiagnosticReports" -newermt "@$started" -name 'Starling*' 2>/dev/null || true)
if [ -n "$crashes" ]; then
  for report in $crashes; do echo "::group::$report"; head -120 "$report"; echo "::endgroup::"; done
  # The Simulator-only stop above ends the app too.
  if [ $simulator_stop = 0 ]; then
    echo '::error::the app crashed in the Simulator (reports above)'
    failed=1
  fi
fi
if xcrun simctl spawn "$device" launchctl list | grep -q "UIKitApplication:$bundle"; then
  echo 'The app is still running.'
else
  echo '::warning::the app is no longer running'
fi
xcrun simctl shutdown "$device" || true
exit $failed
