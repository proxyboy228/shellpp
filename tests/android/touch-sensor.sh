#!/bin/bash
# Touches the Android emulator's fingerprint sensor with finger <id> every
# two seconds until <stop-file> exists (at most 10 minutes), so that every
# biometric prompt of the test running meanwhile passes (D218).
#   bash tests/android/touch-sensor.sh <id> <stop-file>
ADB=${ADB:-"$LOCALAPPDATA/Android/Sdk/platform-tools/adb.exe"}
for i in $(seq 1 300); do
  [ -e "$2" ] && exit 0
  "$ADB" emu finger touch "$1" >/dev/null 2>&1
  sleep 1
  "$ADB" emu finger remove "$1" >/dev/null 2>&1
  sleep 1
done
