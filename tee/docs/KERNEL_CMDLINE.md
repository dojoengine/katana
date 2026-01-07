# Kernel Command Line Parameters

This document explains the kernel command line parameters used in `/tee/configs/cmdline.txt` for the Katana TEE VM image.

## Current Configuration

```
console=ttyS0 loglevel=7 quiet
```

## Parameter Breakdown

### `console=ttyS0`

**Purpose:** Specifies where the Linux kernel should send console output and receive input.

**What it means:**
- `ttyS0` is the first serial port (COM1 in DOS/Windows terminology)
- This redirects all kernel messages and system console to the serial port
- Essential for VMs because:
  - VMs typically don't have a "real" display/keyboard
  - Serial console works with QEMU, cloud VMs, and hypervisors
  - Allows viewing boot messages and accessing the system remotely

**Why we use it:**
- TEE VMs run headless (no GUI)
- Serial console is the standard way to interact with cloud VMs
- Works with `virsh console`, QEMU `-serial`, and cloud serial console features
- GCP Confidential VMs provide serial console access via this

**Alternatives:**
- `console=hvc0` - For Xen paravirtualized console
- `console=tty0` - For standard VGA text console (not useful in cloud VMs)
- `console=ttyS0,115200n8` - Same but explicitly sets baud rate (115200), no parity (n), 8 data bits

### `loglevel=7`

**Purpose:** Sets the kernel log level for messages printed to the console.

**Log Levels (0-7):**
```
0 - KERN_EMERG   - System is unusable (panic, etc.)
1 - KERN_ALERT   - Action must be taken immediately
2 - KERN_CRIT    - Critical conditions
3 - KERN_ERR     - Error conditions
4 - KERN_WARNING - Warning conditions
5 - KERN_NOTICE  - Normal but significant
6 - KERN_INFO    - Informational
7 - KERN_DEBUG   - Debug-level messages (most verbose)
```

**Current setting: `7` (KERN_DEBUG)**
- Shows ALL kernel messages including debug output
- Useful during development and testing
- Helps diagnose boot issues, driver problems, hardware detection

**Why we use it:**
- **Development phase:** We want maximum visibility into what's happening
- **TEE debugging:** Helps verify secure boot chain and attestation setup
- **Troubleshooting:** If the VM fails to boot, we see detailed error messages

**Production considerations:**
- For production, consider `loglevel=4` (errors and warnings only)
- Reduces console noise and potential information leakage
- Improves boot time slightly (less I/O to serial console)

**Trade-offs:**
- Higher log level = more output = slower boot (serial console is slow)
- Higher log level = more debugging information = easier to troubleshoot
- Lower log level = faster boot = less information if something goes wrong

### `quiet`

**Purpose:** Reduces the amount of informational messages during boot.

**What it does:**
- Suppresses most non-critical boot messages
- Overrides some verbose driver output
- Makes boot appear "cleaner" with fewer messages

**Interaction with `loglevel`:**
- `quiet` primarily affects userspace init messages and some driver verbosity
- It does NOT completely silence kernel messages if `loglevel` is high
- There's some overlap/conflict: `loglevel=7` shows everything, `quiet` tries to hide things
- In practice with both: You'll see kernel debug messages but fewer informational messages

**Why we have it:**
- Historical/conventional - often included by default
- Reduces clutter from well-known harmless messages
- Makes it easier to spot actual errors in the output

**Current combination analysis:**
```
loglevel=7 quiet
     ↓        ↓
  Show all + Hide some  = Mostly verbose but cleaner
```

This is a bit contradictory. For development, you might want to **remove `quiet`** to see everything.

## Recommended Configurations

### Development / Testing (current)
```
console=ttyS0 loglevel=7
```
**Remove `quiet`** - Get all possible debug output for troubleshooting.

### Production
```
console=ttyS0 loglevel=4 quiet
```
Show only warnings and errors, suppress informational noise.

### Minimal / Security-Focused
```
console=ttyS0 loglevel=3
```
Only critical errors and alerts. No `quiet` needed since loglevel is already restrictive.

## Additional Useful Parameters

You may want to add these to `cmdline.txt` depending on your needs:

### `init=/bin/katana`
**Purpose:** Specifies the first program to run after kernel boots.

**Example:**
```
console=ttyS0 loglevel=7 init=/bin/katana
```

**Benefits:**
- Skips all init systems (systemd, initramfs, etc.)
- Kernel directly launches Katana as PID 1
- Absolutely minimal - no other processes
- Fastest boot time

**Drawbacks:**
- Katana must handle being PID 1 (reaping zombies, signal handling)
- No process management or supervision
- If Katana crashes, kernel panics (no way to restart)

**Current approach:**
Our initrd has an `init` script that handles the PID 1 responsibilities and then launches Katana. This is more robust.

### `ro`
**Purpose:** Mount root filesystem read-only.

**Benefits:**
- Security: Prevents tampering with system files
- Integrity: Ensures measured boot state doesn't change
- Durability: Protects against accidental corruption

**When to use:**
- Production TEE deployments
- When you want immutable infrastructure
- Add to cmdline: `console=ttyS0 loglevel=4 ro`

### `panic=0`
**Purpose:** Controls kernel behavior on panic.

**Values:**
- `panic=0` - Halt forever (default, good for debugging)
- `panic=10` - Reboot after 10 seconds (good for production auto-recovery)
- `panic=-1` - Reboot immediately

### `selinux=0` or `apparmor=0`
**Purpose:** Disable SELinux or AppArmor.

**When to use:**
- If not using these security frameworks
- Slightly reduces boot time and memory usage
- Simplifies debugging (no policy denials)

### `mitigations=off`
**Purpose:** Disable CPU vulnerability mitigations (Spectre, Meltdown, etc.).

**WARNING:** Security trade-off!
- **Performance gain:** 5-30% in some workloads
- **Security loss:** Vulnerable to CPU side-channel attacks
- **TEE context:** The TEE hardware (SEV-SNP) provides isolation, so host-based attacks may be less relevant
- **Recommendation:** Only disable if you understand the risks and have measured performance benefits

### `nosmap` / `nosmep`
**Purpose:** Disable Supervisor Mode Access/Execution Prevention.

**WARNING:** Major security impact! Only for debugging hardware issues.

## SEV-SNP Specific Considerations

### Why cmdline matters for measurements

The kernel command line is **included in the SEV-SNP measurement**:

```
MEASUREMENT = SHA384(OVMF || kernel_hash_table || VMSAs)
                              ↑
                              includes cmdline hash
```

**Implication:** Any change to cmdline.txt changes the measurement!

**Example:** Changing `loglevel=7` to `loglevel=4` produces a completely different measurement.

**Best practices:**
1. **Lock down cmdline early** - Don't change it after calculating measurements
2. **Document cmdline** - Users need exact parameters to verify measurements
3. **Minimal changes** - Each cmdline change requires recalculating and republishing measurements

### Kernel options that DON'T affect measurement

These can be set at boot time without changing the measurement:
- **None in this model** - Our cmdline is in the initrd/kernel hash table

In some configurations (like SEV-SNP with SVSM), you can pass runtime options that aren't measured, but our current approach bakes everything into the measurement.

## Debugging Boot Issues

If the VM fails to boot:

1. **Increase verbosity:**
   ```
   console=ttyS0 loglevel=8 debug
   ```
   (`debug` is even more verbose than `loglevel=7`)

2. **Add early console:**
   ```
   console=ttyS0 earlyprintk=serial,ttyS0,115200
   ```
   Shows messages before regular console is initialized

3. **Disable quiet mode:**
   Remove `quiet` to see all messages

4. **Add init debug:**
   ```
   init=/bin/sh
   ```
   Drops to a shell instead of running init (useful for debugging initrd)

## Example Configurations

### Current (Development)
```
console=ttyS0 loglevel=7 quiet
```

### Recommended for Development
```
console=ttyS0 loglevel=7
```
(Remove `quiet` to see everything)

### Production TEE
```
console=ttyS0 loglevel=3 ro panic=10
```
- Errors only
- Read-only root
- Auto-reboot on panic

### Maximum Security
```
console=ttyS0 loglevel=0 ro panic=0 init=/bin/katana
```
- No console output (prevents information leakage)
- Read-only root
- Halt on panic (prevents reboot attacks)
- Direct init to Katana

### Maximum Performance
```
console=ttyS0 loglevel=3 mitigations=off
```
- Minimal logging
- CPU mitigations disabled (⚠️ security trade-off)

## References

- [Linux Kernel Parameters](https://www.kernel.org/doc/html/latest/admin-guide/kernel-parameters.html) - Official documentation
- [Serial Console HOWTO](https://www.kernel.org/doc/html/latest/admin-guide/serial-console.html)
- [SEV-SNP Measurement](https://www.amd.com/system/files/TechDocs/56860.pdf) - AMD SNP ABI specification

## Recommendation for Katana TEE

For the current development phase, I recommend:

```
console=ttyS0 loglevel=7
```

**Rationale:**
- Remove `quiet` - We want ALL messages during development
- Keep `loglevel=7` - Maximum debug visibility
- Keep `console=ttyS0` - Essential for VM console access

For production, switch to:
```
console=ttyS0 loglevel=3 ro
```

**Rationale:**
- `loglevel=3` - Only critical errors and alerts
- `ro` - Read-only root for integrity
- Remove `quiet` - Not needed with low loglevel
