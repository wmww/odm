#!/usr/bin/env bash
# Screenshot the ODM viewer (or run anything Wayland) inside a private headless
# compositor. Each run gets its own XDG_RUNTIME_DIR, so concurrent agents never
# clobber each other, and everything is torn down on exit.
#
#   scripts/ui-shot.sh [-o OUT] [-s WxH] [-d SECS] [-k KEYS] [PROJECT]
#   scripts/ui-shot.sh --session CMD [ARGS...]
#
#   -o OUT     png path (default /tmp/odm-ui-shot.png)
#   -s WxH     output size (default 1280x720)
#   -d SECS    extra settle delay after the frame stops changing (default 0)
#   -k KEYS    keystrokes to send with wtype before the shot, e.g. -k 'f'
#   PROJECT    project dir to open (default examples/hello-bracket)
#   --session  run CMD inside the compositor instead; $WAYLAND_DISPLAY and
#              $XDG_RUNTIME_DIR are set, so plain `grim out.png` works.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
out=/tmp/odm-ui-shot.png
size=1280x720
delay=0
keys=
session_mode=0

if [[ ${1-} == --session ]]; then
  session_mode=1
  shift
  [[ $# -gt 0 ]] || { echo "--session needs a command" >&2; exit 2; }
else
  while getopts ':o:s:d:k:h' opt; do
    case $opt in
      o) out=$OPTARG ;;
      s) size=$OPTARG ;;
      d) delay=$OPTARG ;;
      k) keys=$OPTARG ;;
      h) sed -n '2,15p' "${BASH_SOURCE[0]}"; exit 0 ;;
      *) echo "bad flag -$OPTARG" >&2; exit 2 ;;
    esac
  done
  shift $((OPTIND - 1))
  project=${1:-$repo/examples/hello-bracket}
  # Ask cargo for the target dir rather than assuming ./target.
  target=$(cd "$repo" && cargo metadata --format-version 1 --no-deps 2>/dev/null |
    sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
  target=${target:-$repo/target}
  engine=${ODM_ENGINE_BIN:-$target/debug/odm-engine}
  cli=${ODM_CLI_BIN:-$target/debug/odm}
  [[ -x $engine ]] || { echo "no engine binary at $engine (cargo build?)" >&2; exit 1; }
  [[ -d $project ]] || { echo "no project dir $project" >&2; exit 1; }
  project=$(cd "$project" && pwd)
fi

# Job control: every background job lands in its own process group, so cleanup
# can take out whatever it spawned (a `--session` script's children included).
set -m

export XDG_RUNTIME_DIR
XDG_RUNTIME_DIR=$(mktemp -d "${TMPDIR:-/tmp}/odm-wl-XXXXXXXX")
chmod 700 "$XDG_RUNTIME_DIR"
export WAYLAND_DISPLAY=wayland-0
runtime=$XDG_RUNTIME_DIR
comp_pid=
app_pid=

group_alive() { [[ -n $1 ]] && kill -0 -- "-$1" 2>/dev/null; }
kill_group() { [[ -n $1 ]] && kill "-$2" -- "-$1" 2>/dev/null || true; }

# Record what we own so a later run can reap us if we are SIGKILLed.
note_owner() { echo "$$ $comp_pid $app_pid" >"$runtime/owner"; }
note_owner

cleanup() {
  local rc=$?
  kill_group "$app_pid" TERM
  kill_group "$comp_pid" TERM
  for _ in $(seq 1 15); do
    group_alive "$app_pid" || group_alive "$comp_pid" || break
    sleep 0.2
  done
  kill_group "$app_pid" KILL
  kill_group "$comp_pid" KILL
  rm -rf "$runtime"
  return $rc
}

# Reap sessions whose script died without running cleanup (SIGKILL, crash).
# A session is stale only if its owning script is gone.
reap_stale() {
  local dir opid c a
  for dir in "${TMPDIR:-/tmp}"/odm-wl-*; do
    [[ -d $dir && $dir != "$runtime" ]] || continue
    if ! read -r opid c a <"$dir/owner" 2>/dev/null; then
      # No owner yet: either a run that started microseconds ago, or debris.
      [[ -n $(find "$dir" -maxdepth 0 -mmin +5 2>/dev/null) ]] && rm -rf "$dir" 2>/dev/null
      continue
    fi
    if [[ -n $opid ]] && grep -qa ui-shot "/proc/$opid/cmdline" 2>/dev/null; then
      continue # still running
    fi
    kill_group "$a" KILL
    kill_group "$c" KILL
    rm -rf "$dir" 2>/dev/null || true
  done
}
reap_stale
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP

# labwc's headless backend needs no seat, GPU or /dev/input.
WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 \
  labwc >"$runtime/labwc.log" 2>&1 &
comp_pid=$!
note_owner

for _ in $(seq 1 100); do
  [[ -S $runtime/$WAYLAND_DISPLAY ]] && break
  group_alive "$comp_pid" || { cat "$runtime/labwc.log" >&2; exit 1; }
  sleep 0.1
done
[[ -S $runtime/$WAYLAND_DISPLAY ]] || { echo "compositor never came up" >&2; cat "$runtime/labwc.log" >&2; exit 1; }

wlr-randr --output HEADLESS-1 --custom-mode "$size" >/dev/null 2>&1 || true

if (( session_mode )); then
  "$@" &
  app_pid=$!
  note_owner
  wait "$app_pid"
  exit $?
fi

"$engine" "$project" >"$runtime/engine.log" 2>&1 &
app_pid=$!
note_owner

# `odm build` blocks until the scene is built, so we don't screenshot a viewer
# that is still showing an empty generation.
if [[ -x $cli ]]; then
  for _ in $(seq 1 100); do
    [[ -S $project/.odm/engine.sock ]] && break
    sleep 0.1
  done
  "$cli" --project "$project" build >"$runtime/build.log" 2>&1 || true
fi

# Wait for the window to appear and stop changing: two identical frames in a
# row that differ from the bare desktop. Falls through after ~15s.
bare=$(grim - 2>/dev/null | sha256sum)
prev=
for _ in $(seq 1 75); do
  sleep 0.2
  group_alive "$app_pid" || { echo "engine exited early:" >&2; cat "$runtime/engine.log" >&2; exit 1; }
  cur=$(grim - 2>/dev/null | sha256sum) || continue
  [[ $cur == "$bare" ]] && continue
  [[ $cur == "$prev" ]] && break
  prev=$cur
done

[[ $delay != 0 ]] && sleep "$delay"
if [[ -n $keys ]]; then
  wtype "$keys"
  sleep 0.5
fi

grim "$out"
# The socket path is fixed per project, so two runs on one project share it: the
# second engine still shows its viewer, but `odm` talks to the first one.
grep -q 'another engine is already running' "$runtime/engine.log" 2>/dev/null &&
  echo "warning: another engine owns $project/.odm/engine.sock; this viewer is not the one \`odm\` talks to" >&2
echo "$out"
