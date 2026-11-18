#!/usr/bin/env bash
# UI test rig for cloud sessions: same commands as scripts/rig.sh, on Xvfb :99 instead of xspace.
# rig: up | down | relaunch | quit | start | shot FILE.png | act XDOTOOL... | status | pages
# The sandbox's HTTPS proxy is not honoured by Servo, so web pages are served locally (`pages`), not from the internet.
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export DISPLAY=:99
HOME_DIR="$ROOT/target/test-home/rig"; BIN="$ROOT/target/test-bin/ados-rig"; OUT="$ROOT/target/test-out/cloud"
mkdir -p "$OUT"
start_app() { ADOS_HOME="$HOME_DIR" nohup "$BIN" >"$OUT/app.log" 2>&1 &
  for i in $(seq 60); do xdotool search --name '^aDOs$' >/dev/null 2>&1 && break; sleep 1; done; sleep 3; }
stop_app() { pkill -x ados-rig || true; pkill -x servoshell || true; sleep 1; }
case "${1:-}" in
  up)
    mkdir -p "$ROOT/target/test-bin"; cp "$ROOT/target/debug/ados" "$BIN" || exit 1
    pgrep -x Xvfb >/dev/null || { nohup Xvfb :99 -screen 0 1920x1080x24 >"$OUT/xvfb.log" 2>&1 & sleep 2; }
    pgrep -x xfwm4 >/dev/null || { nohup dbus-run-session xfwm4 --compositor=on >"$OUT/wm.log" 2>&1 & sleep 2; }  # plain xfwm4 dies here: "Xfconf could not be initialized"
    pgrep -f 'http.server 8765' >/dev/null || (cd "$ROOT/crates/ados-browser/fixtures" && nohup python3 -m http.server 8765 --bind 127.0.0.1 >"$OUT/pages.log" 2>&1 &)
    start_app; echo "rig up (data: $HOME_DIR, local pages: http://127.0.0.1:8765/)";;
  relaunch) stop_app; start_app; echo relaunched;;
  quit) stop_app; echo "app stopped";;
  start) start_app; echo started;;
  down) stop_app; pkill -x xfwm4 || true; pkill -x Xvfb || true; pkill -f 'http.server 8765' || true; rm -rf "$HOME_DIR"; echo "rig down, test data deleted";;
  shot) scrot -o "$2" && echo "$2";;
  act) shift; xdotool "$@";;
  pages) ls "$ROOT/crates/ados-browser/fixtures";;
  status) pgrep -x ados-rig >/dev/null && echo "app running" || echo "app NOT running"; free -m | awk 'NR==2{print "avail MB",$7}';;
  *) echo "usage: cloud-rig.sh up|down|relaunch|quit|start|shot FILE|act XDOTOOL...|status|pages"; exit 2;;
esac
