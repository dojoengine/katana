#!/usr/bin/env bash
# React + Vite frontend (HTTPS by default via mkcert; HTTP=1 for plain http — handled
# by the vite config). Reads contract addresses from app/src/deployments.json.
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_common.sh"

free_port "$FRONTEND_PORT"
echo "→ frontend (vite) on :$FRONTEND_PORT"
cd "$DEMO_DIR/app"
exec bun run dev
