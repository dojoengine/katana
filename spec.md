# Katana Avnu Paymaster + VRF Integration Spec

## Background
Katana currently ships a Cartridge-specific paymaster flow and an in-process VRF helper. We are
migrating paymaster support to the Avnu paymaster service and the new VRF service used by
`controller-rs` integration tests. Katana should support these services locally (sidecar) and in
production (external), expose compatible RPCs, and include the services in the Docker
distribution.

## Goals
- Support Avnu paymaster flows for sponsored and self-funded transactions from Controller
  accounts.
- Optionally run Avnu paymaster and VRF as sidecar processes when enabled by CLI/config.
- Use Katana's existing keypair material for VRF proofs instead of a new dedicated key.
- Ensure required classes are declared and contracts are deployed on Katana when running locally.
- Proxy paymaster JSON-RPC through Katana's RPC server when paymaster is active.
- Provide `cartridge_addExecuteFromOutside` compatibility on top of the Avnu sponsored flow.

## Non-goals
- Implement paymaster policy management or quota systems inside Katana.
- Replace the Controller account model or redefine `OutsideExecution` wire formats.
- Provide new UI/Explorer features for paymaster or VRF management.

## Requirements
### Functional
- CLI/config toggles for paymaster and VRF with `disabled | sidecar | external` modes.
- Sidecar processes started and supervised by Katana when enabled.
- On-chain bootstrap for dev chains: declare required classes and deploy paymaster/VRF contracts.
- RPC proxying of paymaster requests through Katana when paymaster is active.
- Backwards compatible `cartridge_addExecuteFromOutside` method mapped to Avnu flow.

### Non-functional
- Default behavior is unchanged when paymaster/VRF is disabled.
- VRF proofs are derived from a stable, existing Katana keypair.
- Sidecar startup has timeouts and clear error reporting.
- Docker image includes sidecar binaries without running them by default.

## Design
### Components
- Paymaster client: thin JSON-RPC client to the Avnu paymaster service.
- VRF client: HTTP client for the VRF service (`/info`, `/stark_vrf`).
- Sidecar manager: spawns, monitors, and shuts down optional sidecar processes.
- Bootstrapper: ensures classes are declared and contracts are deployed when local.
- RPC proxy: forwards paymaster JSON-RPC requests through Katana when active.

### Configuration model
Modes are explicit and independent for paymaster and VRF:
- `disabled`: feature off, no sidecar or external calls.
- `sidecar`: Katana spawns local service(s) and uses local URLs.
- `external`: Katana uses provided URLs and does not spawn services.

Suggested CLI flags (names can map to existing `--cartridge.*` for compatibility):
- `--paymaster.mode <disabled|sidecar|external>`
- `--paymaster.url <URL>` (required for `external`)
- `--paymaster.prefunded-index <N>` (default `0`)
- `--vrf.mode <disabled|sidecar|external>`
- `--vrf.url <URL>` (required for `external`)
- `--vrf.key-source <prefunded|sequencer>`
- `--vrf.prefunded-index <N>` (default `0`)

Config file sketch:
```toml
[paymaster]
mode = "sidecar"
url = "http://127.0.0.1:8081"
prefunded_index = 0

[vrf]
mode = "sidecar"
url = "http://127.0.0.1:3000"
key_source = "prefunded"
prefunded_index = 0

[sidecar]
paymaster_bin = "/usr/local/bin/avnu-paymaster"
vrf_bin = "/usr/local/bin/vrf"
startup_timeout_secs = 10
```

### Startup + bootstrap flow (sidecar)
1. Resolve mode, URLs, and sidecar binaries from CLI/config.
2. Select prefunded account index and read its private key from genesis.
3. Declare required classes in genesis (Controller, paymaster contract, VRF provider).
4. Deploy paymaster and VRF provider contracts if missing (UDC flow).
5. Spawn sidecar services with RPC URL, chain id, account key, and contract addresses.
6. Wait for sidecar readiness before enabling RPC proxy.

### Key management for VRF
Katana derives the VRF key from an existing keypair:
- Default: use the prefunded account at `vrf.prefunded_index`.
- Optional: use the sequencer keypair if available in chain spec/config.
- The derived public key determines the VRF provider contract address.

### Paymaster RPC proxying
When paymaster is active:
- Katana exposes a `paymaster_*` namespace via its JSON-RPC server.
- Requests in that namespace are forwarded to the configured paymaster URL.
- Errors from the paymaster service are passed through verbatim.

### `cartridge_addExecuteFromOutside` compatibility
The compatibility method is implemented in the `cartridge` namespace:
- Inputs: `address`, `outside_execution`, `signature` (same as today).
- Build an `execute_from_outside` call for the Controller account.
- If VRF `request_random` is present, wrap calls with VRF submit/assert.
- Use Avnu paymaster to sponsor (or return self-funded) and submit the tx.
- Keep `cartridge_addExecuteOutsideTransaction` as an alias or redirect.

### VRF execution flow
When VRF is enabled and the first call is `request_random`:
- Compute the seed per the VRF contract rules.
- Ask the VRF service to generate the proof and random value.
- Inject `submit_random` and `assert_consumed` around the outer call.
- Ensure the VRF provider contract is deployed at the derived address.

## Docker distribution
- Build the Avnu paymaster and VRF binaries in the Docker pipeline.
- Copy binaries into the final image alongside `katana`.
- Sidecar processes are started only when CLI/config enables them.

## Testing
- Add Katana integration tests mirroring `controller-rs` sponsored/self-funded flows.
- Add VRF tests for `request_random` wrapping and proof generation.
- Add RPC proxy tests that validate pass-through behavior and errors.

## Rollout
- Introduce new paymaster/VRF modes behind feature flags.
- Keep current Cartridge RPCs as wrappers during migration.
- Add deprecation warnings for legacy flags once the new flow is stable.

## Open questions
- Exact Avnu paymaster RPC method set and required config parameters.
- Whether to default VRF to the paymaster account or the sequencer key.
- How to persist paymaster/VRF contract addresses for non-dev chains.
