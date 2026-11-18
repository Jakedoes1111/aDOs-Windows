#!/usr/bin/env bash
# (stage3-int: the assistant is opened with Ctrl+5, since Alt+Space opens the xfwm4 window menu in the cloud rig.)
# Focus matrix: drives the rig with real X events and saves evidence (screenshots + steps.tsv). No pass/fail: a reader judges.
# usage: [ENGINE=Chromium] RIG=scripts/cloud-rig.sh scripts/focus-matrix.sh [case ...]    cases: enter leave click overlays steal window launch engine rapid
# Each combination starts from a fresh rig (down + up: test data deleted), so cells are independent.
# Needs the rig up (`$RIG up`). Geometry can be overridden by env (see below); calibrate with `$RIG shot`.
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"; cd "$ROOT"
RIG=${RIG:-scripts/cloud-rig.sh}
OUT=${OUT:-$ROOT/target/test-out/focus-matrix${ENGINE:+-$ENGINE}}
PAGES=${PAGES:-http://127.0.0.1:8765}
# geometry, relative to the aDOs window's top-left (1280x800 window, Research layout + Browser):
# rail icons, address bar, link/field/button on focus-probe.html (Chromium shows an infobar that pushes the page down)
RAIL_X=${RAIL_X:-35}; RAIL_Y0=${RAIL_Y0:-78}; RAIL_DY=${RAIL_DY:-55}
if [ "${ENGINE:-Servo}" = Chromium ]; then DY=36; else DY=0; fi
ST=${ST:-36}  # stage4-tabs: the Browser's page tab strip pushes its toolbar and page down
ADDR=${ADDR:-"358 $((122+ST))"}; LINK=${LINK:-"136 $((279+DY+ST))"}; FIELD=${FIELD:-"298 $((260+DY+ST))"}; BTN=${BTN:-"501 $((279+DY+ST))"}
RELOAD=${RELOAD:-"183 $((122+ST))"}; SETTINGS=${SETTINGS:-"35 765"}; LAYOUT_MENU=${LAYOUT_MENU:-"158 23"}; LAYOUT_RESEARCH=${LAYOUT_RESEARCH:-"168 69"}
PAGE_BODY=${PAGE_BODY:-"378 $((560+ST))"}; TABCLICK=${TABCLICK:-"153 84"}
# module -> Ctrl+number (Home1 Notes2 Terminal3 Browser4 Assistant5 Sessions6 Tasks7 Calendar8)
declare -A NUM=([home]=1 [notes]=2 [terminal]=3 [browser]=4 [sessions]=6 [tasks]=7 [calendar]=8)
MODS=(home notes terminal browser calendar sessions tasks)  # rail order
act() { $RIG act "$@" >/dev/null 2>&1; }
key() { act key --delay 40 "$@"; }
typ() { act type --delay 60 "$1"; }
win() { $RIG act search --onlyvisible --name '^aDOs$' 2>/dev/null | head -1; }
click() { local w; w=$(win); act mousemove --window "$w" $1 $2 click 1; }
railclick() { local i=0 m; for m in "${MODS[@]}"; do [ "$m" = "$1" ] && break; i=$((i+1)); done; click $RAIL_X $((RAIL_Y0+i*RAIL_DY)); }
CASE=""; N=0
begin() { CASE=$1; N=0; mkdir -p "$OUT/$CASE"; : >> "$OUT/$CASE/steps.tsv"; }
step() { N=$((N+1)); local f; f=$(printf '%s/%s/%02d-%s.png' "$OUT" "$CASE" $N "$(echo "$1" | tr -c 'A-Za-z0-9\n' '-' | cut -c1-40)")
  sleep 0.6; $RIG shot "$f" >/dev/null 2>&1; printf '%s\t%02d\t%s\t%s\n' "$CASE" $N "$1" "${f#$ROOT/}" >> "$OUT/$CASE/steps.tsv"; }
goto() { key ctrl+k; sleep .4; typ "$1"; key Return; sleep .8; }          # load URL via palette-free route: see openurl
openurl() { key ctrl+l; sleep .3; typ "$1"; key Return; sleep 2; }       # address bar must be reachable: Ctrl+L in browser
to_browser_page() { key ctrl+4; sleep 1; openurl "$PAGES/focus-probe.html"; click $PAGE_BODY; sleep .5; }
to_browser_addr() { key ctrl+4; sleep 1; key ctrl+l; sleep .4; }
# put the keyboard in a module's primary input
prime() { case $1 in
  notes) key ctrl+2;; terminal) key ctrl+3;; tasks) key ctrl+7; sleep .3; key ctrl+f;;
  calendar) key ctrl+8;; sessions) key ctrl+6; sleep .3; key ctrl+f;; home) key ctrl+1;;
  assistant) key ctrl+5;; page) to_browser_page;; addr) to_browser_addr;; esac; sleep .8; }
switch_to() { # how target
  case $1 in
    ctrl) key ctrl+${NUM[$2]};;
    rail) railclick $2;;
    palette) key ctrl+k; sleep .4; typ "Open ${2^}"; sleep .4; key Return;;
    tab) click $TABCLICK;;
    layout) click $LAYOUT_MENU; sleep .5; click $LAYOUT_RESEARCH;;   # layout menu > Research (the layout that holds the Browser)
  esac; sleep 1; }

case_enter() { begin enter
  local from how
  for from in notes terminal tasks calendar sessions home assistant; do
    for how in ctrl rail palette tab layout; do
      setup; prime $from; step "$from-start"; switch_to $how browser; typ abc; step "$from-via-$how-typed-abc"
    done; done; }
case_leave() { begin leave
  local from to how
  for from in page addr; do for to in notes terminal tasks calendar sessions home; do for how in ctrl click; do
    setup; prime $from; step "$from-start"
    if [ $how = ctrl ]; then key ctrl+${NUM[$to]}; else railclick $to; fi; typ xyz; step "$from-to-$to-$how-typed-xyz"
    key ctrl+4; sleep .8; step "$from-to-$to-$how-back-in-browser"
  done; done; done; }
case_click() { begin click
  local prev
  for prev in addr toolbar module overlay; do for tgt in LINK FIELD; do
    setup; to_browser_page
    case $prev in addr) key ctrl+l;; toolbar) click $RELOAD;; module) key ctrl+2;; overlay) key ctrl+k;; esac
    sleep .6; step "$prev-before-$tgt"; click ${!tgt}; sleep 1; typ k; step "$prev-click-$tgt-typed-k"
  done; done; }
case_overlays() { begin overlays
  local from ov
  for from in page addr; do for ov in palette capture settings layout assistant comment; do
    setup; prime $from; step "$from-$ov-start"
    case $ov in palette) key ctrl+k;; capture) key ctrl+shift+k;; settings) click $SETTINGS;; layout) click $LAYOUT_MENU;;
      assistant) key ctrl+5;; comment) key ctrl+shift+m;; esac
    sleep .8; step "$from-$ov-open"; key Escape; sleep .6; typ q; step "$from-$ov-closed-typed-q"
  done; done; }
case_steal() { begin steal
  local ctx
  for ctx in notes palette; do
    setup; to_browser_page; key ctrl+2; sleep .5; [ $ctx = palette ] && key ctrl+k; sleep .5; step "$ctx-before"
    openurl_bg "$PAGES/focus-probe.html?autofocus=1"; sleep 3; typ n; step "$ctx-autofocus-loaded-typed-n"
    key Escape; done; }
openurl_bg() { key ctrl+4; sleep .8; key ctrl+l; typ "$1"; key Return; sleep .3; key ctrl+2; }
case_window() { begin window
  local mode
  for mode in page addr; do
    setup; prime $mode; step "$mode-start"
    other_window; sleep 1.5; step "$mode-other-window-up"; sleep 4; typ w; step "$mode-other-closed-typed-w"
    prime $mode; act search --name '^aDOs$' windowminimize %1; sleep 1; step "$mode-minimised"
    act search --name '^aDOs$' windowmap %1 windowactivate %1; sleep 1.5; typ w; step "$mode-restored-typed-w"
  done; }
# a short-lived foreign X window that takes focus, then closes and gives it back
other_window() { if [[ $RIG == *cloud-rig* ]]; then DISPLAY=:99 xterm -e sleep 4 >/dev/null 2>&1 &
  else XSPACE_NAME=${XSPACE_NAME:-adosrig} xspace run xterm -e sleep 4 >/dev/null 2>&1; fi; }
case_launch() { begin launch
  local s
  for s in page addr notes; do
    setup; prime $s; sleep 1; $RIG relaunch >/dev/null 2>&1; sleep 3; step "$s-relaunched"; typ l; step "$s-relaunched-typed-l"
  done; }
case_engine() { begin engine
  to_browser_page; step start
  local k
  for k in ctrl+t ctrl+n ctrl+w ctrl+shift+t F12 ctrl+p F11; do key $k; sleep 1; step "$k"
    # Ctrl+W is also the aDOs "hide module" chord: bring the Browser back so the next key is tested on the page
    [ $k = ctrl+w ] && { key ctrl+shift+z; sleep 1; key ctrl+4; sleep 1; click $PAGE_BODY; step "after-ctrl+w-restore"; }; done
  key Escape; key ctrl+l; typ "$PAGES/second.html"; key Return; sleep 2; step "load-url-after"; }
case_rapid() { begin rapid
  local i m
  for i in $(seq 10); do for m in 1 2 3 4 5 6 7 8; do key ctrl+$m; sleep .08; done; done
  key ctrl+4; sleep .8; typ r; step "ended-on-browser-typed-r"
  for i in $(seq 10); do for m in 1 2 3 4 5 6 7 8; do key ctrl+$m; sleep .08; done; done
  key ctrl+2; sleep .8; typ r; step "ended-on-notes-typed-r"; }

CASES=("$@"); [ ${#CASES[@]} -eq 0 ] && CASES=(enter leave click overlays steal window launch engine rapid)
# the rig's test data dir (cloud-rig: target/test-home/rig; rig.sh: target/test-home/<XSPACE_NAME minus "ados">)
if [[ $RIG == *cloud-rig* ]]; then RIG_HOME=${RIG_HOME:-$ROOT/target/test-home/rig}
else _n=${XSPACE_NAME:-adosrig}; _n=${_n#ados}; RIG_HOME=${RIG_HOME:-$ROOT/target/test-home/${_n:-rig}}; fi
# every combination starts from a fresh data dir, Browser on the probe page, Notes active
# ENGINE=Servo|Chromium picks the classic engine: seeded into the fresh data dir's browser state
ENGINE=${ENGINE:-Servo}
setup() { $RIG quit >/dev/null 2>&1; rm -rf "$RIG_HOME"; mkdir -p "$RIG_HOME/state"
  printf '{"url":"%s/focus-probe.html","engine":"%s"}' "$PAGES" "$ENGINE" > "$RIG_HOME/state/browser.json"
  $RIG start >/dev/null 2>&1; sleep 2; key ctrl+4; sleep 1; openurl "$PAGES/focus-probe.html"; key ctrl+2; sleep .8; }
# the cloud rig's xfwm4 exits without a D-Bus session; without a window manager there is no X focus
# management and minimise does nothing, so start one here if it is missing
if [[ $RIG == *cloud-rig* ]] && ! pgrep -x xfwm4 >/dev/null; then
  DISPLAY=:99 setsid nohup dbus-run-session -- xfwm4 --compositor=on >/dev/null 2>&1 < /dev/null & sleep 3; fi
for c in "${CASES[@]}"; do echo "== $c"; setup; "case_$c"; done
echo "evidence: $OUT (see */steps.tsv)"
