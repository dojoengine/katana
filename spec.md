# Katana Avnu Paymaster + VRF Integration Spec

## Background
Katana is migrating paymaster support to the Avnu paymaster service and the new VRF account model
used by controller-rs. We no longer ship custom `katana-paymaster`/`katana-vrf` binaries; Katana
should spawn the real `paymaster-service` and `vrf-server` binaries when configured for sidecar
mode, and otherwise talk to external services.

## Goals
- Support Avnu paymaster flows for sponsored and self-funded transactions from Controller accounts.
- Optionally run Avnu paymaster and VRF as sidecar processes when enabled by CLI/config.
- Use Katana’s existing keypair material for VRF proofs and account signing.
- Ensure required classes are declared and contracts are deployed on Katana when running locally.
- Proxy paymaster JSON-RPC through Katana’s RPC server when paymaster is active.
- Provide `cartridge_addExecuteFromOutside` compatibility on top of the Avnu flow.

## Non-goals
- Implement paymaster policy management or quota systems inside Katana.
- Replace the Controller account model or redefine `OutsideExecution` wire formats.
- Provide UI/Explorer features for paymaster or VRF management.

## Requirements
### Functional
- CLI/config toggles for paymaster and VRF with `disabled | sidecar | external` modes.
- Sidecar mode spawns `paymaster-service` and `vrf-server` (PATH or explicit `--*.bin`).
- On-chain bootstrap: declare classes; deploy Avnu forwarder, VRF account, VRF consumer;
  set VRF public key; whitelist relayer in forwarder.
- Generate a paymaster profile JSON and launch `paymaster-service` via `PAYMASTER_PROFILE`.
- VRF uses a Katana-derived keypair for proof generation and outside-execution signing.
- RPC proxying of paymaster requests through Katana when paymaster is active.
- Backwards compatible `cartridge_addExecuteFromOutside` method mapped to Avnu flow.

### Non-functional
- Default behavior is unchanged when paymaster/VRF is disabled.
- Sidecar startup has timeouts and clear error reporting.
- Docker image includes `paymaster-service` and `vrf-server` without running them by default.

## Design
### Components
- Paymaster client: JSON-RPC client to the Avnu paymaster service.
- VRF client: HTTP client for `vrf-server` (`/info`, `/stark_vrf`).
- VRF wrapper: Katana logic to compute seed, request proof, and build/sign VRF outside executions.
- Sidecar manager: spawns, monitors, and shuts down optional sidecar processes.
- Bootstrapper: ensures classes are declared and contracts are deployed when local.
- RPC proxy: forwards `paymaster_*` methods through Katana’s JSON-RPC server.

### Configuration model
Modes are explicit and independent for paymaster and VRF:
- `disabled`: feature off, no sidecar or external calls.
- `sidecar`: Katana spawns local service(s) and uses local URLs.
- `external`: Katana uses provided URLs and does not spawn services.

Suggested CLI flags (names can map to existing `--cartridge.*` for compatibility):
- `--paymaster.mode <disabled|sidecar|external>`
- `--paymaster.url <URL>` (required for `external`)
- `--paymaster.prefunded-index <N>` (relayer N, gas tank N+1, estimate N+2)
- `--paymaster.bin <PATH>` (defaults to `paymaster-service`)
- `--paymaster.price-api-key <KEY>` (optional, for Avnu price oracle)
- `--vrf.mode <disabled|sidecar|external>`
- `--vrf.url <URL>` (required for `external`)
- `--vrf.key-source <prefunded|sequencer>`
- `--vrf.prefunded-index <N>`
- `--vrf.bin <PATH>` (defaults to `vrf-server`)

### Startup + bootstrap flow (sidecar)
1. Resolve modes, URLs, and sidecar binaries from CLI/config.
2. Derive paymaster accounts from prefunded index (relayer, gas tank, estimate).
3. Declare required classes in genesis (Controller, Avnu forwarder, VRF account/consumer).
4. Deploy forwarder if missing (owner = relayer, gas fees recipient = gas tank).
5. Whitelist relayer in forwarder.
6. Deploy VRF account and consumer if missing; set VRF public key.
7. Generate a paymaster profile JSON; launch `paymaster-service` via `PAYMASTER_PROFILE`.
8. Launch `vrf-server --secret-key <u64>` and wait for readiness.

### Key management for VRF
Katana derives the VRF key from an existing keypair:
- Default: use the prefunded account at `vrf.prefunded_index`.
- Optional: use the sequencer keypair if available in chain spec/config.
- The derived public key determines the VRF account address.

### Paymaster RPC proxying
When paymaster is active:
- Katana exposes a `paymaster_*` namespace via its JSON-RPC server.
- Requests in that namespace are forwarded to the configured paymaster URL.
- Errors from the paymaster service are passed through verbatim.

### `cartridge_addExecuteFromOutside` compatibility
The compatibility method is implemented in the `cartridge` namespace:
- Inputs: `address`, `outside_execution`, `signature` (same as today).
- Build an `execute_from_outside` call for the Controller account.
- If VRF `request_random` is present, wrap calls with VRF submit + execute_from_outside.
- Use Avnu paymaster to sponsor (or self-fund) and submit the tx.

### VRF execution flow
When VRF is enabled and a `request_random` call is present:
- Decode `request_random` calldata to get `caller` + `source`.
- Compute the seed using on-chain state (pedersen + poseidon, per VRF contract rules).
- Call `vrf-server /stark_vrf` to fetch the proof.
- Build a `submit_random` call for the VRF account.
- Wrap `submit_random` + original `execute_from_outside` call inside a new
  `OutsideExecutionV2` signed by the VRF account.

## Docker distribution
- Install `vrf-server` and `paymaster-service` using versions from `.tool-versions`.
- Copy the binaries into the final image alongside `katana`.
- Sidecar processes are started only when CLI/config enables them.

## Testing
- Unit tests for VRF wrapping helpers (submit_random calldata, request_random detection).
- Integration tests for paymaster-sponsored and self-funded flows (Controller + Avnu).
- Integration tests for VRF wrapping when `request_random` is present.

## Rollout
- Keep legacy Cartridge RPCs as wrappers during migration.
- Add deprecation warnings for legacy flags once the new flow is stable.
