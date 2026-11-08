#!/bin/zsh
# usage: run.sh <mode> <seconds> [ENV=VAL ...]
mode=$1; secs=$2; shift 2
cd "$(dirname "$0")/.."
mkdir -p logs shots
env S1_AUTOPILOT=1 S1_QUIT=1 S1_STAGE_EXEMPT=1 S1_NO_ACTIVATE=1 S1_WINCAP=$PWD/wincap S1_SHOTS=shots/$mode "$@" ./target/debug/s1 $mode > logs/$mode.log 2>&1 &
pid=$!
for i in $(seq 1 $((secs*10))); do
  kill -0 $pid 2>/dev/null || break
  sleep 0.1
done
if kill -0 $pid 2>/dev/null; then echo "TIMEOUT - killing"; kill $pid; sleep 0.5; kill -9 $pid 2>/dev/null; fi
pkill -f "target/debug/s1" 2>/dev/null
cat logs/$mode.log | grep -v "^\s*$" | tail -80
