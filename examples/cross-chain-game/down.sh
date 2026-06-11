#!/usr/bin/env bash
# Stop the demo: both Katana nodes (ports 5050/5051), the saya-tee sidecar, and
# both torii indexers (ports 8081/8082).
set -uo pipefail

stopped=0
kill_match() {
  local label="$1" pat="$2"
  local pids
  pids=$(pgrep -f "$pat" || true)
  if [[ -n "$pids" ]]; then
    echo "→ stopping $label (pid $pids)"
    kill $pids 2>/dev/null || true
    stopped=1
  fi
}

kill_match "settlement katana (:5050)" "katana .*--http.port 5050"
kill_match "appchain katana (:5051)" "katana .*--http.port 5051"
kill_match "saya-tee" "saya-tee tee start"
kill_match "torii (score :8081)" "torii .*--http.port 8081"
kill_match "torii (game :8082)" "torii .*--http.port 8082"

[[ "$stopped" -eq 0 ]] && echo "no demo processes running."
echo "done."
