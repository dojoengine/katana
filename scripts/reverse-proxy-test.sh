#!/bin/bash

# This script is for running a test to make sure the `/explorer` endpoint can still be accessible
# as normal when Katana is put behind a reverse proxy.
#
# This script is meant to be run at the repo's root directory

set -e

# Use environment variables for binary paths, or default to bare commands

KATANA_BIN="${KATANA_BIN:-katana}"
CADDY_BIN="${CADDY_BIN:-caddy}"

# Cleanup background processes
cleanup() {
	echo "Cleaning up processes..."
	if [ ! -z "$KATANA_PID" ]; then
		kill $KATANA_PID 2>/dev/null || true
	fi
	if [ ! -z "$CADDY_PID" ]; then
		kill $CADDY_PID 2>/dev/null || true
	fi
}

# Wait for a service to be ready
#
# Arguments:
#   $1 - service_name: The name of the service to wait for
#   $2 - url: The health check URL of the service to check for service availability
wait_for_service() {
	local service_name=$1
	local url=$2

	for i in {1..30}; do
		if curl -s -o /dev/null -w "%{http_code}" "$url" | grep -q "200"; then
			echo "${service_name} is running at ${url}"
			return 0
		fi

		if [ $i -eq 30 ]; then
			echo "Failed to start ${service_name}"
			exit 1
		fi

		sleep 1
	done
}

# Perofrm background processes cleanup on exit
trap cleanup EXIT

${KATANA_BIN} --http.port 6060 --explorer > katana.log 2>&1 &
KATANA_PID=$!
echo "Katana starting at port 6060..."
wait_for_service "Katana" "http://localhost:6060/"

${CADDY_BIN} run --config ./tests/fixtures/Caddyfile > caddy.log 2>&1 &
CADDY_PID=$!
echo "Caddy starting at port 9090..."
wait_for_service "Caddy" "https://localhost:9090/health-check"

if ! cargo run -p reverse-proxy-test; then
	echo
	echo -e "\033[31mtest failed\033[0m"
	echo
	echo "=== Last 50 lines of katana.log ==="
	echo
	tail -n 50 katana.log
	echo
	echo "=== Last 50 lines of caddy.log ==="
	echo
	tail -n 50 caddy.log
	exit 1
fi
