#!/usr/bin/env bash
# Off-screen timer CPU: cull-ON (only visible band's timers run) vs --all-timers
# (every card's timer runs, no gating). RELEASE build — gate perf in release
# (memory: terminal-slice-1c-executed). Idle, no scroll interaction; the delta
# is the culling saving.
set -euo pipefail
cd "$(dirname "$0")"

SECS="${1:-10}"
BIN=../../target/release/board-container

echo "building release…"
cargo build -q --release -p board-container

sample() {              # $1 = label, $2.. = extra args
  local label="$1"; shift
  "$BIN" "$@" >/dev/null 2>&1 &
  local pid=$!
  sleep 2                                   # window warm-up
  local sum=0 n=0
  for _ in $(seq 1 "$SECS"); do
    local c
    c=$(ps -o %cpu= -p "$pid" 2>/dev/null | tr -d ' ' || echo 0)
    [ -z "$c" ] && c=0
    sum=$(echo "$sum + $c" | bc)
    n=$((n + 1))
    sleep 1
  done
  kill "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  echo "$label: avg ${label:+}$(echo "scale=1; $sum / $n" | bc)% CPU over ${n}s"
}

echo "=== board-container off-screen CPU (idle, ${SECS}s samples) ==="
sample "cull-ON  (visible band only)"
sample "all-timers (no gating)      " --all-timers
echo "note: idle window has a compositor floor; the DELTA is the culling saving."
