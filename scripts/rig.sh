#!/usr/bin/env bash
# UI test rig for aDOs on `xspace` (the author's virtual-display tool). Without xspace use scripts/cloud-rig.sh (Xvfb), same subcommands.
# All input and screenshots go through this; it can only reach the invisible test display.
# rig.sh up | down | shot <file.png> | act <xdotool args...> | relaunch | status
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# One rig per agent: XSPACE_NAME picks the display, data dir and binary copy.
N=${XSPACE_NAME:-adosrig}; N=${N#ados}; N=${N:-rig}
export XSPACE_NAME=ados$N XSPACE_WM='xfwm4 --compositor=on'
HOME_DIR="$ROOT/target/test-home/$N"; BIN="$ROOT/target/test-bin/ados-$N"; EXE=$(basename "$BIN" | cut -c1-15)
start_app() { ADOS_HOME="$HOME_DIR" xspace run "$BIN" >/dev/null 2>&1; xspace wait-window 'aDOs' >/dev/null; sleep 3; }
stop_app() { for p in $(pgrep -x "$EXE"); do kill "$p"; done; sleep 1; }
case "${1:-}" in
  up)
    avail=$(free -m | awk 'NR==2{print $7}')
    [ "$avail" -lt 1200 ] && { echo "refusing: only ${avail} MB available"; exit 1; }
    mkdir -p "$ROOT/target/test-bin"; cp "$ROOT/target/debug/ados" "$BIN" || exit 1
    xspace start >/dev/null 2>&1; start_app; echo "rig up (data: $HOME_DIR)";;
  relaunch) stop_app; start_app; echo relaunched;;
  quit) stop_app; echo "app stopped (display still up)";;
  start) start_app; echo started;;
  down) stop_app; xspace stop >/dev/null 2>&1; rm -rf "$HOME_DIR"; echo "rig down, test data deleted";;
  shot) xspace shot "$2" >/dev/null && echo "$2";;
  act) shift; xspace keys "$@";;
  status) xspace status; pgrep -x "$EXE" >/dev/null && echo "app running" || echo "app NOT running"; free -m | awk 'NR==2{print "avail MB",$7}';;
  *) echo "usage: rig.sh up|down|relaunch|quit|start|shot FILE|act XDOTOOL...|status"; exit 2;;
esac
