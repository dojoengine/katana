#!/usr/bin/env bash
# Stop any Katana nodes started by this demo (ports 5050 and 5051).
set -uo pipefail

stopped=0
for port in 5050 5051; do
  pids=$(pgrep -f "katana --dev --dev.no-fee --http.port $port" || true)
  if [[ -n "$pids" ]]; then
    echo "→ stopping katana on :$port (pid $pids)"
    kill $pids 2>/dev/null || true
    stopped=1
  fi
done
[[ "$stopped" -eq 0 ]] && echo "no demo Katana nodes running."
echo "done."
