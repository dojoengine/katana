#!/usr/bin/env bash
# Stop the demo: both Katana nodes (ports 5050/5051). The frontend runs in the
# foreground under up.sh, so Ctrl-C there already stops it; this is for cleaning
# up nodes left behind by a detached run.
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

[[ "$stopped" -eq 0 ]] && echo "no demo processes running."
echo "done."
