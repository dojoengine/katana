#!/usr/bin/env bash
# Stop the dungeon demo: the appchain Katana (:5070), the saya-tee sidecar, and
# both torii indexers (:8091/:8092). The settlement layer is remote Sepolia —
# nothing to stop there.
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

kill_match "appchain katana (:5070)" "katana .*--http.port 5070"
kill_match "saya-tee" "saya-tee tee start"
kill_match "torii (score :8091)" "torii .*--http.port 8091"
kill_match "torii (game :8092)" "torii .*--http.port 8092"

[[ "$stopped" -eq 0 ]] && echo "no demo processes running."
echo "done."
