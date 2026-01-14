#!/bin/bash
#
# Katana TEE Setup Script
# Connects to remote host, starts SNP VM in public mode, and returns RPC URL
#

set -e

# Load environment
if [ -f .env ]; then
    # Parse .env file, handling comments and empty lines
    while IFS='=' read -r key value; do
        # Skip empty lines and comments
        [[ -z "$key" || "$key" =~ ^[[:space:]]*# ]] && continue
        # Remove leading/trailing whitespace from key
        key=$(echo "$key" | xargs)
        # Skip if key is empty after trimming
        [[ -z "$key" ]] && continue
        # Export only if value is non-empty
        if [[ -n "$value" ]]; then
            export "$key=$value"
        fi
    done < .env
fi

# Defaults
TEE_HOST="${TEE_HOST:?Error: TEE_HOST not set in .env}"
TEE_SSH_PORT="${TEE_SSH_PORT:-22}"
TEE_SSH_USER="${TEE_SSH_USER:-ubuntu}"
VM_SSH_PORT="${VM_SSH_PORT:-2224}"
VM_SSH_PASSWORD="${VM_SSH_PASSWORD:-ubuntu123}"
RPC_PORT="${RPC_PORT:-5050}"

SSH_OPTS="-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o ConnectTimeout=10"

# SSH command builder
if [ -n "$TEE_SSH_KEY" ]; then
    HOST_SSH="ssh $SSH_OPTS -i $TEE_SSH_KEY -p $TEE_SSH_PORT $TEE_SSH_USER@$TEE_HOST"
else
    HOST_SSH="ssh $SSH_OPTS -p $TEE_SSH_PORT $TEE_SSH_USER@$TEE_HOST"
fi

echo "=============================================="
echo " Katana TEE Setup"
echo "=============================================="
echo ""
echo "Host: $TEE_HOST"
echo ""

# Function to run command on host
run_on_host() {
    $HOST_SSH "$1"
}

# Function to wait for condition
wait_for() {
    local msg=$1
    local check_cmd=$2
    local max_attempts=${3:-60}
    
    echo -n "$msg"
    for i in $(seq 1 $max_attempts); do
        if eval "$check_cmd" 2>/dev/null; then
            echo " ✓"
            return 0
        fi
        echo -n "."
        sleep 2
    done
    echo " ✗ (timeout)"
    return 1
}

case "${1:-start}" in
    start)
        echo "[1/4] Starting SNP VM in PUBLIC mode..."
        # Run in background with nohup to avoid blocking on console login prompt
        run_on_host "nohup ./snp-vm-helper.sh start-public > /tmp/snp-vm-start.log 2>&1 &" || true
        
        echo ""
        echo "[2/4] Waiting for VM to boot..."
        wait_for "  SSH" "nc -z $TEE_HOST $VM_SSH_PORT" 90
        
        # Additional wait for VM to fully initialize
        sleep 10
        
        echo ""
        echo "[3/4] Starting Katana with TEE..."
        # Run katana start in background as well
        run_on_host "nohup ./snp-vm-helper.sh start-katana > /tmp/katana-start.log 2>&1 &" || true
        
        sleep 5
        
        echo ""
        echo "[4/4] Verifying RPC..."
        if nc -z "$TEE_HOST" "$RPC_PORT" 2>/dev/null; then
            echo "  RPC is accessible ✓"
        else
            echo "  Warning: RPC port not accessible from client (firewall?)"
        fi
        
        echo ""
        echo "=============================================="
        echo " Setup Complete!"
        echo "=============================================="
        echo ""
        echo "RPC URL: http://${TEE_HOST}:${RPC_PORT}"
        echo ""
        echo "Test command:"
        echo "  curl -s http://${TEE_HOST}:${RPC_PORT} -X POST \\"
        echo "    -H 'Content-Type: application/json' \\"
        echo "    -d '{\"jsonrpc\":\"2.0\",\"method\":\"tee_generateQuote\",\"params\":[],\"id\":1}'"
        echo ""
        
        # Export for use in other scripts
        echo "http://${TEE_HOST}:${RPC_PORT}" > .katana-rpc-url
        echo "RPC URL saved to .katana-rpc-url"
        ;;
    
    stop)
        echo "Stopping SNP VM..."
        run_on_host "./snp-vm-helper.sh stop"
        rm -f .katana-rpc-url
        echo "Done"
        ;;
    
    status)
        echo "Checking status..."
        run_on_host "./snp-vm-helper.sh status"
        ;;
    
    test)
        RPC_URL=$(cat .katana-rpc-url 2>/dev/null || echo "http://${TEE_HOST}:${RPC_PORT}")
        echo "Testing TEE attestation at $RPC_URL ..."
        RESPONSE=$(curl -s --max-time 10 "$RPC_URL" -X POST \
            -H 'Content-Type: application/json' \
            -d '{"jsonrpc":"2.0","method":"tee_generateQuote","params":[],"id":1}')
        
        if [ -z "$RESPONSE" ]; then
            echo "Error: No response from RPC (is Katana running?)"
            exit 1
        fi
        
        echo "$RESPONSE" | python3 -m json.tool 2>/dev/null || echo "$RESPONSE"
        
        # Save response for later use
        echo "$RESPONSE" > .katana-quote-response.json
        echo ""
        echo "Response saved to .katana-quote-response.json"
        ;;
    
    url)
        # Just output the RPC URL (for piping to other commands)
        echo "http://${TEE_HOST}:${RPC_PORT}"
        ;;
    
    *)
        echo "Usage: $0 {start|stop|status|test|url}"
        echo ""
        echo "Commands:"
        echo "  start   - Start SNP VM and Katana, get RPC URL"
        echo "  stop    - Stop the SNP VM"
        echo "  status  - Check VM and Katana status"
        echo "  test    - Test TEE attestation endpoint"
        echo "  url     - Print RPC URL"
        ;;
esac
