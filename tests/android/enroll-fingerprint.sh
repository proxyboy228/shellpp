#!/bin/bash
# Enrolls fingerprint <id> on the Android emulator through Settings, for the
# fingerprint unlock tests of scripts/verify-android.ps1 (D218). The
# emulator's screen lock PIN must be 1111. ADB names adb.exe.
#   bash tests/android/enroll-fingerprint.sh <id>
export MSYS_NO_PATHCONV=1
ADB=${ADB:-"$LOCALAPPDATA/Android/Sdk/platform-tools/adb.exe"}
FINGER=$1
screen() {
  "$ADB" shell uiautomator dump /sdcard/ui.xml >/dev/null 2>&1
  "$ADB" shell cat /sdcard/ui.xml
}
# The center of the first node whose text matches $1 (case-insensitive).
center_of() {
  python -I -c "
import sys,re
x=sys.stdin.read()
for m in re.finditer(r'<node [^>]*>', x):
    n=m.group(0)
    t=re.search(r' text=\"([^\"]*)\"',n).group(1)
    if re.fullmatch(sys.argv[1], t, re.I):
        a,b,c,d=map(int,re.findall(r'\d+',re.search(r'bounds=\"([^\"]*)\"',n).group(1)))
        print((a+c)//2,(b+d)//2); break
" "$1"
}
# A Settings task left open would be brought to the front as it was.
"$ADB" shell am force-stop com.android.settings
"$ADB" shell am start -a android.settings.FINGERPRINT_ENROLL >/dev/null
sleep 3
"$ADB" shell input text 1111
"$ADB" shell input keyevent 66
for step in $(seq 1 40); do
  sleep 1.5
  page=$(screen)
  if echo "$page" | grep -q 'Fingerprint added'; then
    xy=$(echo "$page" | center_of 'done')
    [ -n "$xy" ] && "$ADB" shell input tap $xy
    echo "fingerprint $FINGER enrolled"
    exit 0
  fi
  if echo "$page" | grep -qi 'Touch the sensor\|Lift'; then
    "$ADB" emu finger touch "$FINGER" >/dev/null
    sleep 0.8
    "$ADB" emu finger remove "$FINGER" >/dev/null 2>&1
    continue
  fi
  xy=$(echo "$page" | center_of 'more|i agree|agree|next|start|continue|got it')
  [ -n "$xy" ] && "$ADB" shell input tap $xy
done
echo "fingerprint $FINGER: enrollment did not finish" >&2
exit 1
