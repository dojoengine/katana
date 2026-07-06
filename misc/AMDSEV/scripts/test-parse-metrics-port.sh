#!/bin/bash
# ==============================================================================
# TEST-PARSE-METRICS-PORT.SH - Unit tests for start-vm.sh's parse_metrics_port.
# ==============================================================================
#
# start-vm.sh derives the enclave's metrics port forward entirely from
# --katana-args: if the args carry --metrics.port PORT, that guest port is
# forwarded to the host; otherwise there is no metrics forward. parse_metrics_port
# is the function that extracts that port, and is the single source of truth for
# the forward — so a regression here silently drops the metrics endpoint.
#
# This extracts the function from start-vm.sh and exercises it directly — fast,
# no QEMU — the same pattern as test-strip-reserved-args.sh. The metrics path is
# otherwise only observable by booting the enclave and scraping /metrics.
#
# Run from anywhere; exits non-zero on any failed assertion.
# ==============================================================================

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
START_VM="${SCRIPT_DIR}/../start-vm.sh"
[ -f "$START_VM" ] || { echo "start-vm.sh not found at $START_VM" >&2; exit 1; }

FN="$(mktemp)"
trap 'rm -f "$FN"' EXIT

# Extract just the parse_metrics_port function definition (from its header to
# the first line that is a lone closing brace).
awk '/^parse_metrics_port\(\)/{f=1} f{print} f && /^}$/{exit}' "$START_VM" > "$FN"
grep -q '^parse_metrics_port()' "$FN" \
    || { echo "failed to extract parse_metrics_port from $START_VM" >&2; exit 1; }
# shellcheck disable=SC1090  # sourcing the extracted function by path
. "$FN"

FAILS=0
check() {
    # $1 description, $2 expected, $3 actual
    if [ "$3" = "$2" ]; then
        echo "ok   - $1"
    else
        echo "FAIL - $1"
        echo "         want: [$2]"
        echo "         got:  [$3]"
        FAILS=$((FAILS + 1))
    fi
}

# The script's own default args must carry a metrics port (metrics on by default).
check "default args carry --metrics.port 9100" \
    "9100" "$(parse_metrics_port "--http.addr,0.0.0.0,--http.port,5050,--tee,sev-snp,--metrics,--metrics.addr,0.0.0.0,--metrics.port,9100")"

# No metrics flags at all => no port => no forward.
check "no metrics flags => empty" \
    "" "$(parse_metrics_port "--http.port,5050,--tee,sev-snp")"

# The "--metrics.port=VALUE" single-token form is supported.
check "--metrics.port=VALUE form" \
    "9200" "$(parse_metrics_port "--tee,sev-snp,--metrics,--metrics.port=9200")"

# --metrics without an explicit --metrics.port => nothing to forward.
check "--metrics without a port => empty" \
    "" "$(parse_metrics_port "--tee,sev-snp,--metrics")"

# A non-default port is picked up verbatim.
check "custom port" \
    "7777" "$(parse_metrics_port "--metrics,--metrics.addr,0.0.0.0,--metrics.port,7777")"

# --metrics.port as the final token has no value => empty (not a stray flag).
check "--metrics.port as the last token => empty" \
    "" "$(parse_metrics_port "--tee,sev-snp,--metrics.port")"

# A glob value elsewhere in the args must not interfere with (or corrupt) the
# port — parse_metrics_port must not pathname-expand operator values.
check "glob value in other args does not interfere" \
    "9100" "$(cd / && parse_metrics_port "--http.cors-origins,*,--metrics,--metrics.port,9100")"

# If the args carry the port twice, the last one wins (matches Katana/clap).
check "last --metrics.port wins" \
    "9300" "$(parse_metrics_port "--metrics.port,9100,--tee,sev-snp,--metrics.port,9300")"

if [ "$FAILS" -ne 0 ]; then
    echo "parse_metrics_port: ${FAILS} assertion(s) failed" >&2
    exit 1
fi
echo "parse_metrics_port: all assertions passed"
