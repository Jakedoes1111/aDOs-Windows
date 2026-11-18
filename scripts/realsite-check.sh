#!/usr/bin/env bash
# Real-site browser check on the laptop (the cloud has no internet for Servo). Uses the invisible rig display.
# realsite-check.sh [sites-file]   -> screenshots in target/test-out/realsite/<n>-<host>.png
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"; export XSPACE_NAME=adosreal
RIG="$ROOT/scripts/rig.sh"; OUT="$ROOT/target/test-out/realsite"; mkdir -p "$OUT"
SITES=${1:-$ROOT/scripts/realsite-sites.txt}
"$RIG" up || exit 1
"$RIG" act mousemove 195 506 click 1; sleep 8   # open the Research layout (has a Browser)
n=0
while read -r url; do
  [ -z "$url" ] && continue; n=$((n+1)); host=$(echo "$url" | sed -E 's#https?://([^/]+).*#\1#')
  "$RIG" act key ctrl+l; sleep 0.5; "$RIG" act key ctrl+a; "$RIG" act type --delay 15 "$url"; "$RIG" act key Return
  sleep "${WAIT:-14}"; "$RIG" shot "$OUT/$(printf %02d $n)-$host.png"
  "$RIG" status | tail -2 | tr '\n' ' '; echo
done < "$SITES"
"$RIG" down
