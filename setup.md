# Katana TEE RPC Client Setup

Instructions for connecting to a remote Katana node running inside an AMD SEV-SNP protected VM.

## Prerequisites

- SSH access to the TEE host server
- `sshpass` installed locally (for automated SSH): `apt install sshpass` or `brew install hudochenkov/sshpass/sshpass`

---

## Step 1: Create Environment File

Create a `.env` file in your project root:

```bash
# .env - Katana TEE Connection Settings

# Remote host running the SNP VM
TEE_HOST=185.26.9.157

# SSH port for the host (not the VM)
TEE_SSH_PORT=22

# SSH credentials for the host
TEE_SSH_USER=ubuntu
TEE_SSH_KEY=~/.ssh/id_rsa  # Or use TEE_SSH_PASSWORD

# VM SSH port (forwarded from host)
VM_SSH_PORT=2224
VM_SSH_PASSWORD=ubuntu123

# RPC port (when in public mode)
RPC_PORT=5050
```

---

## Step 2: Create the Setup Script

Create `katana-tee-setup.sh`:

```bash
#!/bin/bash
#
# Katana TEE Setup Script
# Connects to remote host, starts SNP VM in public mode, and returns RPC URL
#

set -e

# Load environment
if [ -f .env ]; then
    export $(grep -v '^#' .env | xargs)
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
        run_on_host "./snp-vm-helper.sh start-public" 2>/dev/null || true
        
        echo ""
        echo "[2/4] Waiting for VM to boot..."
        wait_for "  SSH" "nc -z $TEE_HOST $VM_SSH_PORT" 90
        
        # Additional wait for VM to fully initialize
        sleep 10
        
        echo ""
        echo "[3/4] Starting Katana with TEE..."
        run_on_host "./snp-vm-helper.sh start-katana" 2>/dev/null
        
        sleep 3
        
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
        curl -s "$RPC_URL" -X POST \
            -H 'Content-Type: application/json' \
            -d '{"jsonrpc":"2.0","method":"tee_generateQuote","params":[],"id":1}' | python3 -m json.tool
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
```

---

## Step 3: Usage

```bash
# Make executable
chmod +x katana-tee-setup.sh

# Start everything and get RPC URL
./katana-tee-setup.sh start

# Check status
./katana-tee-setup.sh status

# Test TEE attestation
./katana-tee-setup.sh test

# Get just the URL (for scripts)
RPC_URL=$(./katana-tee-setup.sh url)

# Stop when done
./katana-tee-setup.sh stop
```

---

## Output

After running `./katana-tee-setup.sh start`, you will get:

```
RPC URL: http://185.26.9.157:5050
```

The URL is also saved to `.katana-rpc-url` for use by other scripts.

---

## Using the RPC

### Get TEE Attestation Quote

```bash
curl -s http://185.26.9.157:5050 -X POST \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"tee_generateQuote","params":[],"id":1}'
```

### Response Structure

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "quote": "0x...",       // SEV-SNP attestation report (hex)
    "stateRoot": "0x...",   // Blockchain state root
    "blockHash": "0x...",   // Block hash
    "blockNumber": 0        // Block number
  }
}
```

---

## Notes

- The VM takes ~2 minutes to boot
- The RPC port (5050) must be accessible through any firewalls
- The attestation quote binds `Poseidon(stateRoot, blockHash)` to the hardware measurement
