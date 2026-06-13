#!/bin/bash
# Start TEE VM with AMD SEV-SNP
# Usage: ./start-vm.sh --ovmf PATH --kernel PATH --initrd PATH
#                     [--katana-args CSV] [--chain-dir DIR] [--no-start] [...]
#
# This script:
# 1. Starts QEMU with the TEE boot components, passing Katana's CLI args via
#    fw_cfg and the optional chain config dir as a read-only virtio-blk ext2
#    disk packed from --chain-dir
# 2. Creates and attaches a data disk as /dev/sda
# 3. Optionally starts Katana asynchronously via a virtio-serial control channel
# 4. Forwards RPC port to host
#
# ==============================================================================
# LAUNCH MEASUREMENT INPUTS
# ==============================================================================
# The following parameters are used by QEMU/OVMF to compute the SEV-SNP launch
# measurement. Verifiers must use the same values to reproduce the measurement.
#
# Boot components (hashed when kernel-hashes=on):
#   OVMF_FILE      - OVMF.fd firmware image
#   KERNEL_FILE    - vmlinuz kernel image
#   INITRD_FILE    - initrd.img initial ramdisk
#   KERNEL_CMDLINE - "console=ttyS0" plus (for sealed storage):
#                      KATANA_EXPECTED_LUKS_UUID=<uuid>
#                    Sealed and unsealed variants produce different
#                    measurements. Verifiers must pin the expected
#                    cmdline variant.
#
# SEV-SNP guest configuration:
#   GUEST_POLICY      - 0x30000 (SMT allowed, debug disabled)
#   VCPU_COUNT        - 1
#   GUEST_FEATURES    - 0x1 (SNP active)
#
# CPU and platform:
#   CPU_TYPE          - EPYC-v4
#   CBITPOS           - 51 (C-bit position for memory encryption)
#   REDUCED_PHYS_BITS - 1
#
# Katana launch configuration (CLI args + chain config dir) is delivered via
# QEMU fw_cfg entries under opt/org.katana/ and is NOT part of the launch
# measurement — fw_cfg blobs are read by the guest at runtime, not hashed
# into the launch digest. The guest treats them as untrusted operator input
# and strips flags it owns (--db-*, --data-dir, --chain) before use.
#
# To compute expected measurement, use snp-digest from snp-tools:
#   cargo build -p snp-tools
#   ./target/debug/snp-digest --ovmf=OVMF.fd --kernel=vmlinuz --initrd=initrd.img \
#       --append="console=ttyS0" --vcpus=1 --cpu=epyc-v4 --vmm=qemu --guest-features=0x1
#
# ==============================================================================

set -euo pipefail

usage() {
    echo "Usage: $0 --ovmf PATH --kernel PATH --initrd PATH"
    echo "          [--katana-args CSV] [--chain-dir DIR] [--no-start]"
    echo "          [--data-disk PATH] [--luks-uuid UUID] [--unsealed]"
    echo ""
    echo "Starts a SEV-SNP VM and launches Katana asynchronously via control channel."
    echo "Sealed storage is the default — the disk is wrapped in LUKS2 + dm-integrity"
    echo "and unlocked inside the guest via SNP_GET_DERIVED_KEY."
    echo ""
    echo "Required boot components (each pinned by the SEV-SNP launch measurement):"
    echo "  --ovmf PATH           OVMF firmware file (.fd)"
    echo "  --kernel PATH         Linux kernel (vmlinuz)"
    echo "  --initrd PATH         Initrd image (.img)"
    echo ""
    echo "Options:"
    echo "  --katana-args CSV     Comma-separated Katana CLI args, delivered to the"
    echo "                        guest via QEMU fw_cfg (opt/org.katana/args). NOT part"
    echo "                        of the launch measurement."
    echo "  --chain-dir DIR       Directory with chain config files. Packed into a"
    echo "                        small ext2 image at boot and attached as a read-only"
    echo "                        virtio-blk disk; the guest mounts it and passes the"
    echo "                        mount point to Katana as --chain. fw_cfg is not used"
    echo "                        for the chain dir because its port-I/O sysfs read"
    echo "                        path is prohibitively slow under SEV-SNP for multi-MB"
    echo "                        blobs (e.g., a Cartridge-Controller-enabled genesis"
    echo "                        ~18 MB). NOT part of the launch measurement."
    echo "  --no-start            Boot VM without sending Katana start command"
    echo "  --data-disk PATH      Persistent data disk file attached as /dev/sda"
    echo "                        (default: ~/.katana/data.img, auto-created if absent)"
    echo "                        A user-specified PATH must already exist."
    echo "                        Env var: KATANA_DATA_DISK"
    echo "  --luks-uuid UUID      Override the LUKS UUID used for sealed storage."
    echo "                        Default: read from ~/.katana/luks-uuid, auto-generated"
    echo "                        with uuidgen on first run and reused after. Stable per"
    echo "                        host so the launch measurement is stable per host."
    echo "                        Env var: KATANA_LUKS_UUID"
    echo "  --unsealed            Skip sealed storage — boot with plain ext4 on /dev/sda."
    echo "                        Used for dev iteration and CI runs without Docker. The"
    echo "                        kernel cmdline does NOT carry KATANA_EXPECTED_LUKS_UUID,"
    echo "                        which produces a different (and pinnable) launch"
    echo "                        measurement from the canonical sealed boot."
    echo "  -h, --help            Show this help"
}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# Boot components are required and named explicitly via --ovmf / --kernel /
# --initrd. The SEV-SNP launch measurement pins each file by content hash; the
# script doesn't infer them from a directory anymore because doing so encoded
# a hidden contract on filenames + colocation that didn't survive a real deploy
# (artifacts often land on different filesystems, or audits want to swap one
# file against an otherwise-pinned set).
OVMF_FILE=""
KERNEL_FILE=""
INITRD_FILE=""
KATANA_ARGS_CSV="--http.addr,0.0.0.0,--http.port,5050,--tee,sev-snp"
CHAIN_DIR=""
AUTO_START_KATANA=1
DATA_DISK="${KATANA_DATA_DISK:-}"
DATA_DISK_DEFAULT="${HOME}/.katana/data.img"
LUKS_UUID="${KATANA_LUKS_UUID:-}"
LUKS_UUID_FILE="${HOME}/.katana/luks-uuid"
UNSEALED=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        --ovmf)
            [[ $# -ge 2 ]] || {
                echo "Error: --ovmf requires a value"
                exit 1
            }
            OVMF_FILE="$2"
            shift 2
            ;;

        --kernel)
            [[ $# -ge 2 ]] || {
                echo "Error: --kernel requires a value"
                exit 1
            }
            KERNEL_FILE="$2"
            shift 2
            ;;

        --initrd)
            [[ $# -ge 2 ]] || {
                echo "Error: --initrd requires a value"
                exit 1
            }
            INITRD_FILE="$2"
            shift 2
            ;;

        --katana-args)
            [[ $# -ge 2 ]] || {
                echo "Error: --katana-args requires a value"
                exit 1
            }
            KATANA_ARGS_CSV="$2"
            shift 2
            ;;

        --chain-dir)
            [[ $# -ge 2 ]] || {
                echo "Error: --chain-dir requires a value"
                exit 1
            }
            CHAIN_DIR="$2"
            shift 2
            ;;

        --no-start)
            AUTO_START_KATANA=0
            shift
            ;;

        --data-disk)
            [[ $# -ge 2 ]] || {
                echo "Error: --data-disk requires a value"
                exit 1
            }
            DATA_DISK="$2"
            shift 2
            ;;

        --luks-uuid)
            [[ $# -ge 2 ]] || {
                echo "Error: --luks-uuid requires a value"
                exit 1
            }
            LUKS_UUID="$2"
            shift 2
            ;;

        --unsealed)
            UNSEALED=1
            shift
            ;;

        -h|--help)
            usage
            exit 0
            ;;

        -*)
            echo "Error: Unknown option: $1"
            echo ""
            usage
            exit 1
            ;;

        *)
            echo "Error: Unexpected positional argument: $1"
            echo "       Boot components are specified via --ovmf / --kernel / --initrd."
            echo ""
            usage
            exit 1
            ;;
    esac
done

# Boot components are required — every measurement-relevant input must be
# explicitly named by the operator so a typo or missing artifact fails loudly
# at the script level rather than producing a silently-wrong launch digest.
missing=()
[[ -z "$OVMF_FILE" ]]   && missing+=(--ovmf)
[[ -z "$KERNEL_FILE" ]] && missing+=(--kernel)
[[ -z "$INITRD_FILE" ]] && missing+=(--initrd)
if (( ${#missing[@]} > 0 )); then
    echo "Error: missing required boot component flag(s): ${missing[*]}"
    echo ""
    usage
    exit 1
fi

# Sealed storage is canonical. Resolve the LUKS UUID unless the operator
# explicitly opted out with --unsealed. Default: read from ~/.katana/luks-uuid,
# generating with uuidgen on first run. This keeps the measurement stable per
# host across boots while different operators get distinct UUIDs.
#
# UUIDs are normalised to lowercase before any persist/print/cmdline use:
# macOS uuidgen emits uppercase, cryptsetup luksUUID always returns lowercase
# at runtime, and the init does an exact string compare.
if [[ "$UNSEALED" -eq 0 ]]; then
    if [[ -z "$LUKS_UUID" ]]; then
        if [[ -f "$LUKS_UUID_FILE" ]]; then
            LUKS_UUID="$(tr -d '[:space:]' < "$LUKS_UUID_FILE")"
            LUKS_UUID="$(printf '%s' "$LUKS_UUID" | tr '[:upper:]' '[:lower:]')"
            echo "Reusing LUKS UUID from $LUKS_UUID_FILE: $LUKS_UUID"
        else
            command -v uuidgen >/dev/null 2>&1 \
                || { echo "Error: uuidgen not found and no UUID provided"; exit 1; }
            LUKS_UUID="$(uuidgen | tr '[:upper:]' '[:lower:]')"
            mkdir -p "$(dirname "$LUKS_UUID_FILE")"
            printf '%s\n' "$LUKS_UUID" > "$LUKS_UUID_FILE"
            echo "Generated new LUKS UUID and stored at $LUKS_UUID_FILE: $LUKS_UUID"
        fi
    else
        LUKS_UUID="$(printf '%s' "$LUKS_UUID" | tr '[:upper:]' '[:lower:]')"
    fi

    if [[ ! "$LUKS_UUID" =~ ^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$ ]]; then
        echo "Error: LUKS UUID must be canonical (e.g. from uuidgen): got '$LUKS_UUID'"
        exit 1
    fi
elif [[ -n "$LUKS_UUID" ]]; then
    echo "Error: --unsealed and --luks-uuid are mutually exclusive"
    exit 1
fi

# ------------------------------------------------------------------------------
# Launch measurement inputs (must match values documented above)
# ------------------------------------------------------------------------------

# Boot components are set by the --ovmf / --kernel / --initrd flag handlers
# above and validated to be non-empty + readable before we get here.
KERNEL_CMDLINE="console=ttyS0"

# SEV-SNP guest configuration
GUEST_POLICY="0x30000"
VCPU_COUNT=1
CBITPOS=51
REDUCED_PHYS_BITS=1

# VM resources
MEMORY="512M"
CPU_TYPE="EPYC-v4"

# Networking
KATANA_RPC_PORT=5050
HOST_RPC_PORT=15051

# Katana control channel
CONTROL_PORT_NAME="org.katana.control.0"
CONTROL_SOCKET="/tmp/katana-tee-vm-control.$$.sock"
# How long to wait for both (a) the guest's virtio-serial control port to be
# ready and (b) Katana to report `running` after `start`. AMD's OVMF SEV fork
# is built with DEBUG verbosity, and the serial-to-file backend serialises
# every log line — clearing OVMF alone can take 60-90s on a cold boot before
# the kernel even loads. First-boot sealed mode adds luksFormat on top.
# Override via $KATANA_CONTROL_TIMEOUT for slow hosts or the rare hang.
CONTROL_TIMEOUT="${KATANA_CONTROL_TIMEOUT:-300}"

# VM data disk (required by init script). Persistent on host; NOT cleaned up
# on exit. Sealed-mode guests treat the disk as LUKS-encrypted; unsealed
# guests treat it as plain ext4.
if [[ -z "$DATA_DISK" ]]; then
    DATA_DISK="$DATA_DISK_DEFAULT"
    CREATE_DEFAULT_DISK=1
else
    CREATE_DEFAULT_DISK=0
fi
DISK_IMAGE="$DATA_DISK"
DISK_SIZE_MB=1024

# Logs
SERIAL_LOG="$(mktemp /tmp/katana-tee-vm-serial.XXXXXX.log)"

# ------------------------------------------------------------------------------
# Katana launch configuration: fw_cfg (args) + virtio-blk ext2 disk (chain dir)
# ------------------------------------------------------------------------------
# Neither channel is part of the SEV-SNP launch measurement — both are
# operator-supplied at boot. See the header comment.
#
# fw_cfg carries the CLI args (one per line at opt/org.katana/args). Small
# payload, port-I/O cost is negligible.
#
# The chain config dir is delivered via a read-only virtio-blk disk built
# here from --chain-dir. fw_cfg's sysfs read path uses port I/O byte-by-byte,
# and the upstream Linux qemu_fw_cfg driver re-reads the whole blob on every
# sysfs read() call — so cp(1)'ing an 18 MB genesis costs O(blob_size^2)
# port I/O and stalls the guest indefinitely under SEV-SNP. virtio-blk goes
# through DMA, finishes the same copy in milliseconds, and uses the same
# trust posture (host-supplied bytes, guest validates via Katana's parser).
KATANA_ARGS_FILE="$(mktemp /tmp/katana-tee-vm-args.XXXXXX)"
printf '%s' "$KATANA_ARGS_CSV" | tr ',' '\n' > "$KATANA_ARGS_FILE"
FW_CFG_OPTS=(-fw_cfg "name=opt/org.katana/args,file=$KATANA_ARGS_FILE")

CHAIN_IMG=""
CHAIN_DRIVE_OPTS=()
if [[ -n "$CHAIN_DIR" ]]; then
    if [[ ! -d "$CHAIN_DIR" ]]; then
        echo "Error: --chain-dir is not a directory: $CHAIN_DIR"
        exit 1
    fi
    if ! find "$CHAIN_DIR" -mindepth 1 -maxdepth 1 -type f | grep -q .; then
        echo "Error: --chain-dir contains no regular files: $CHAIN_DIR"
        exit 1
    fi
    if ! command -v mkfs.ext2 >/dev/null 2>&1; then
        echo "Error: mkfs.ext2 not found on host; required to pack --chain-dir into the guest disk."
        echo "       Install via: apt-get install -y e2fsprogs"
        exit 1
    fi

    # Size the image as 2 * dir contents + 16 MB headroom, rounded up to MB.
    # mkfs.ext2 itself needs ~5% overhead; doubling is generous and keeps room
    # for future genesis growth without re-running this code.
    CHAIN_DIR_MB=$(du -sm "$CHAIN_DIR" | awk '{print $1}')
    CHAIN_IMG_MB=$(( CHAIN_DIR_MB * 2 + 16 ))
    CHAIN_IMG="$(mktemp /tmp/katana-tee-vm-chain.XXXXXX.img)"
    truncate -s "${CHAIN_IMG_MB}M" "$CHAIN_IMG"
    # -F: force-create on a regular file. -d: populate from a host directory at
    # format time. -L katana-chain: stable label for debug; the guest mounts by
    # device path, not label. -E no_copy_xattrs: keep deterministic content
    # regardless of the host's xattr config.
    mkfs.ext2 -q -F -d "$CHAIN_DIR" -L katana-chain -E no_copy_xattrs "$CHAIN_IMG" \
        || { echo "Error: mkfs.ext2 failed to pack $CHAIN_DIR into $CHAIN_IMG"; exit 1; }

    # Attach as a read-only virtio-blk device. The guest sees it at /dev/vda
    # and mounts it read-only at $KATANA_CHAIN_DIR.
    CHAIN_DRIVE_OPTS=(
        -drive "file=$CHAIN_IMG,format=raw,if=none,id=chaincfg,readonly=on"
        -device 'virtio-blk-pci,drive=chaincfg,serial=katana-chain'
    )
fi

show_serial_tail() {
    echo ""
    echo "=== Serial output (last 80 lines) ==="
    tail -80 "$SERIAL_LOG" 2>/dev/null || echo "(no output)"
}

send_control_command() {
    local cmd="$1"
    local response

    # Keep stdin open for a window after the command. socat closes the write
    # side of the unix socket as soon as stdin EOFs, and QEMU treats that as
    # a full chardev disconnect — the guest's reply written to the virtio-
    # serial port is then dropped before it can flow back. The delay gives
    # the guest's read → handle → respond round-trip time to land before
    # socat tears the connection down. 2s comfortably covers `start`, where
    # the handler exec's /bin/katana in the background before responding.
    response="$( { printf '%s\n' "$cmd"; sleep 2; } | socat -t 2 -T 4 - UNIX-CONNECT:"$CONTROL_SOCKET" 2>/dev/null | head -n1 || true)"
    [[ -n "$response" ]] || return 1
    echo "$response"
}

# Cleanup function
QEMU_PID=""
cleanup() {
    local exit_code=$?

    echo ""
    echo "=== Cleanup ==="

    if [[ -n "$QEMU_PID" ]] && kill -0 "$QEMU_PID" 2>/dev/null; then
        # Prefer a graceful guest shutdown over killing QEMU: the `stop`
        # control command runs the guest's teardown (katana TERM, sync,
        # unmount, luksClose, poweroff), so writes still in the guest page
        # cache reach the sealed data disk. Killing QEMU is a power cut —
        # crash-safe for the LUKS/dm-integrity layers but not for database
        # state above them. Falls through to the kill path if the guest
        # doesn't answer (wedged, or an old initrd without `stop`).
        if [[ -S "$CONTROL_SOCKET" ]] && command -v socat >/dev/null 2>&1; then
            echo "Requesting graceful guest shutdown..."
            { printf 'stop\n'; sleep 2; } | socat -t 2 -T 4 - UNIX-CONNECT:"$CONTROL_SOCKET" >/dev/null 2>&1 || true
            for _ in $(seq 1 30); do
                if ! kill -0 "$QEMU_PID" 2>/dev/null; then
                    echo "Guest powered off gracefully."
                    break
                fi
                sleep 1
            done
        fi
    fi

    if [[ -n "$QEMU_PID" ]] && kill -0 "$QEMU_PID" 2>/dev/null; then
        echo "Stopping QEMU (PID $QEMU_PID)..."
        kill "$QEMU_PID" 2>/dev/null || true
        for _ in $(seq 1 10); do
            if ! kill -0 "$QEMU_PID" 2>/dev/null; then
                break
            fi
            sleep 0.5
        done
        if kill -0 "$QEMU_PID" 2>/dev/null; then
            echo "Force killing QEMU..."
            kill -9 "$QEMU_PID" 2>/dev/null || true
        fi
        wait "$QEMU_PID" 2>/dev/null || true
    fi

    [[ -f "$SERIAL_LOG" ]] && rm -f "$SERIAL_LOG"
    [[ -n "${KATANA_ARGS_FILE:-}" && -f "$KATANA_ARGS_FILE" ]] && rm -f "$KATANA_ARGS_FILE"
    [[ -n "${CHAIN_IMG:-}" && -f "$CHAIN_IMG" ]] && rm -f "$CHAIN_IMG"
    [[ -S "$CONTROL_SOCKET" ]] && rm -f "$CONTROL_SOCKET"
    # NOTE: $DISK_IMAGE is persistent — not cleaned up.

    echo "=== Cleanup complete ==="
    exit "$exit_code"
}
trap cleanup EXIT INT TERM

# Check for root/sudo (needed for KVM and disk formatting)
if [[ "$EUID" -ne 0 ]]; then
    echo "This script requires root privileges for KVM and disk setup."
    echo "Please run with: sudo $0 $*"
    exit 1
fi

for cmd in qemu-system-x86_64 mkfs.ext4 dd; do
    if ! command -v "$cmd" >/dev/null 2>&1; then
        echo "Error: Required command not found: $cmd"
        exit 1
    fi
done

if [[ "$AUTO_START_KATANA" -eq 1 ]]; then
    if ! command -v socat >/dev/null 2>&1; then
        echo "Error: Required command not found: socat"
        echo "Install socat or run with --no-start."
        exit 1
    fi
fi

# Verify files exist
echo "Checking TEE boot components..."
for file in "$OVMF_FILE" "$KERNEL_FILE" "$INITRD_FILE"; do
    if [[ ! -f "$file" ]]; then
        echo "Error: Missing $file"
        exit 1
    fi
    echo "  Found: $file ($(ls -lh "$file" | awk '{print $5}'))"
done

# Prepare data disk for /dev/sda.
# ┌─ path exists? ─┬─ sealed mode? ─┬─ action ───────────────────────────┐
# │  yes           │  yes / no      │  attach as-is                       │
# │  no            │  no            │  dd + mkfs.ext4 (only for default) │
# │  no            │  yes           │  dd only (guest luksFormat's it)   │
# │  no + explicit │  any           │  error (operator must provision)   │
# └────────────────┴────────────────┴─────────────────────────────────────┘
echo ""
echo "Preparing VM data disk..."
if [[ ! -f "$DISK_IMAGE" ]]; then
    if [[ "$CREATE_DEFAULT_DISK" -ne 1 ]]; then
        echo "Error: --data-disk path does not exist: $DISK_IMAGE"
        echo "  Provision the disk explicitly before booting, or drop --data-disk to use the default."
        exit 1
    fi
    mkdir -p "$(dirname "$DISK_IMAGE")"
    dd if=/dev/zero of="$DISK_IMAGE" bs=1M count="$DISK_SIZE_MB" status=none
    if [[ -z "$LUKS_UUID" ]]; then
        # Unsealed: put an ext4 fs on the raw disk directly.
        mkfs.ext4 -F -q "$DISK_IMAGE"
        echo "  Created: $DISK_IMAGE (${DISK_SIZE_MB}MB, plain ext4)"
    else
        # Sealed: leave raw; guest will luksFormat on first boot.
        echo "  Created: $DISK_IMAGE (${DISK_SIZE_MB}MB, raw — guest will luksFormat on first boot)"
    fi
else
    echo "  Reusing existing disk: $DISK_IMAGE"
fi

# Build the effective measured kernel command line. Adding the UUID produces
# a different launch measurement from the unsealed boot; verifiers pin the
# exact variant they expect. The sealed cmdline format is defined in
# sealed-cmdline.sh — shared with the release workflow and verify-build.sh
# so the measurement is reproducible byte-for-byte.
if [[ -n "$LUKS_UUID" ]]; then
    . "${SCRIPT_DIR}/scripts/sealed-cmdline.sh"
    KERNEL_CMDLINE="$(build_sealed_cmdline "$LUKS_UUID")"
fi

echo ""
echo "Starting TEE QEMU VM..."
echo "  OVMF:           $OVMF_FILE"
echo "  Kernel:         $KERNEL_FILE"
echo "  Initrd:         $INITRD_FILE"
echo "  Cmdline:        $KERNEL_CMDLINE"
echo "  Policy:         $GUEST_POLICY"
echo "  vCPUs:          $VCPU_COUNT"
echo "  Memory:         $MEMORY"
echo "  Serial:         $SERIAL_LOG"
echo "  Control socket: $CONTROL_SOCKET"
echo "  Katana args:    $KATANA_ARGS_CSV (via fw_cfg, unmeasured)"
if [[ -n "$CHAIN_DIR" ]]; then
    echo "  Chain dir:      $CHAIN_DIR -> $CHAIN_IMG (${CHAIN_IMG_MB}M ext2, ro virtio-blk, unmeasured)"
else
    echo "  Chain dir:      <none>"
fi
echo "  RPC:            localhost:$HOST_RPC_PORT -> VM:$KATANA_RPC_PORT"
echo ""
echo "To compute expected launch measurement:"
echo "  snp-digest --ovmf=$OVMF_FILE --kernel=$KERNEL_FILE --initrd=$INITRD_FILE \\"
echo "      --append='$KERNEL_CMDLINE' --vcpus=$VCPU_COUNT --cpu=epyc-v4 --vmm=qemu --guest-features=0x1"

qemu-system-x86_64 \
    -enable-kvm \
    -cpu "$CPU_TYPE" \
    -smp "$VCPU_COUNT" \
    -m "$MEMORY" \
    -machine q35,confidential-guest-support=sev0,vmport=off \
    -object memory-backend-memfd,id=ram1,size="$MEMORY",share=true,prealloc=false \
    -machine memory-backend=ram1 \
    -object sev-snp-guest,id=sev0,policy="$GUEST_POLICY",cbitpos="$CBITPOS",reduced-phys-bits="$REDUCED_PHYS_BITS",kernel-hashes=on \
    -nographic \
    -serial "file:$SERIAL_LOG" \
    -bios "$OVMF_FILE" \
    -kernel "$KERNEL_FILE" \
    -initrd "$INITRD_FILE" \
    -append "$KERNEL_CMDLINE" \
    -device virtio-serial-pci,id=virtio-serial0 \
    -chardev socket,id=katanactl,path="$CONTROL_SOCKET",server=on,wait=off \
    -device virtserialport,chardev=katanactl,name="$CONTROL_PORT_NAME" \
    "${FW_CFG_OPTS[@]}" \
    "${CHAIN_DRIVE_OPTS[@]}" \
    -device virtio-scsi-pci,id=scsi0 \
    -drive file="$DISK_IMAGE",format=raw,if=none,id=disk0,cache=none \
    -device scsi-hd,drive=disk0,bus=scsi0.0 \
    -netdev user,id=net0,hostfwd=tcp::${HOST_RPC_PORT}-:${KATANA_RPC_PORT} \
    -device virtio-net-pci,disable-legacy=on,iommu_platform=true,netdev=net0,romfile= \
    &

QEMU_PID=$!
echo "QEMU started with PID $QEMU_PID"

# Wait for serial log file to be created
echo ""
echo "Waiting for serial log file..."
while [[ ! -f "$SERIAL_LOG" ]]; do
    if ! kill -0 "$QEMU_PID" 2>/dev/null; then
        echo "Error: QEMU process died before creating serial log"
        show_serial_tail
        exit 1
    fi
    sleep 0.1
done
echo "Serial log file created"

if [[ "$AUTO_START_KATANA" -eq 1 ]]; then
    echo ""
    echo "Waiting for control socket..."
    waited=0
    while [[ ! -S "$CONTROL_SOCKET" ]]; do
        if ! kill -0 "$QEMU_PID" 2>/dev/null; then
            echo "Error: QEMU process died before control socket became ready"
            show_serial_tail
            exit 1
        fi

        sleep 1
        waited=$((waited + 1))
        if [[ "$waited" -ge "$CONTROL_TIMEOUT" ]]; then
            echo "Error: Timeout waiting for control socket: $CONTROL_SOCKET"
            show_serial_tail
            exit 1
        fi
    done
    echo "Control socket ready"

    # The host-side socket appears the moment QEMU starts, but the GUEST
    # control loop is only ready after firmware → kernel → initrd boot.
    # Probe with `status` until the guest answers (any non-empty response
    # qualifies — `stopped exit=never` is the expected pre-start reply),
    # then send `start`. Without this loop, the start command would race
    # the boot and almost always lose, especially with sealed-mode first
    # boot adding luksFormat on top of OVMF DEBUG verbosity.
    echo ""
    echo "Waiting for guest control loop..."
    waited=0
    while true; do
        if ! kill -0 "$QEMU_PID" 2>/dev/null; then
            echo "Error: QEMU process died before guest control loop was ready"
            show_serial_tail
            exit 1
        fi
        if [[ -n "$(send_control_command "status" || true)" ]]; then
            echo "  Guest control loop is responding"
            break
        fi
        sleep 1
        waited=$((waited + 1))
        if [[ "$waited" -ge "$CONTROL_TIMEOUT" ]]; then
            echo "Error: Timeout waiting for guest control loop"
            show_serial_tail
            exit 1
        fi
    done

    echo ""
    echo "Sending async Katana start command..."
    # Bare `start` — launch config was already delivered via fw_cfg.
    START_RESPONSE="$(send_control_command "start" || true)"
    if [[ -z "$START_RESPONSE" ]]; then
        echo "Error: No response from guest control channel"
        show_serial_tail
        exit 1
    fi
    echo "  Start response: $START_RESPONSE"

    case "$START_RESPONSE" in
        ok\ started*|err\ already-running*)
            ;;
        *)
            echo "Error: Unexpected start response from guest"
            show_serial_tail
            exit 1
            ;;
    esac

    echo ""
    echo "Waiting for Katana running status..."
    waited=0
    while true; do
        if ! kill -0 "$QEMU_PID" 2>/dev/null; then
            echo "Error: QEMU process died while waiting for Katana"
            show_serial_tail
            exit 1
        fi

        STATUS_RESPONSE="$(send_control_command "status" || true)"
        if [[ "$STATUS_RESPONSE" == running\ * ]]; then
            echo "  Status: $STATUS_RESPONSE"
            break
        fi

        sleep 1
        waited=$((waited + 1))
        if [[ "$waited" -ge "$CONTROL_TIMEOUT" ]]; then
            echo "Error: Timeout waiting for Katana to report running"
            echo "  Last status: ${STATUS_RESPONSE:-<none>}"
            show_serial_tail
            exit 1
        fi
    done
else
    echo ""
    echo "Katana auto-start disabled (--no-start)."
    echo "Use the control socket to send commands manually (launch config was"
    echo "already delivered via fw_cfg; start takes no arguments):"
    echo "  printf 'start\n' | socat - UNIX-CONNECT:$CONTROL_SOCKET"
    echo "  printf 'status\n' | socat - UNIX-CONNECT:$CONTROL_SOCKET"
    echo "  printf 'stop\n'   | socat - UNIX-CONNECT:$CONTROL_SOCKET   # graceful shutdown"
fi

echo ""
echo "=== Following serial output (Ctrl+C to exit) ==="
tail -f "$SERIAL_LOG"
