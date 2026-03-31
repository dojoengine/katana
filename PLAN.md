# Sidecar Binary Distribution Implementation Plan

## Overview

Bundle prebuilt `paymaster-service` and `vrf-server` binaries as separate artifacts in katana's GitHub releases, and add lazy auto-installation to katana so that users don't need a Rust toolchain or manual binary management. When katana needs a sidecar binary and can't find it, it prompts the user and downloads the correct version automatically.

## Goals

- Ship prebuilt `paymaster-service` and `vrf-server` binaries for all supported platforms as part of katana releases
- Katana lazily downloads and installs sidecar binaries on first use (with user confirmation)
- Version-locked sidecars: each katana release knows exactly which sidecar version it expects
- Checksum verification of downloaded binaries

## Non-Goals

- Updating the asdf installer to handle sidecar binaries (deferred — katana handles it)
- Building sidecar binaries with cairo-native (they don't use it)
- Auto-updating sidecars in the background without user interaction

## Assumptions and Constraints

- Paymaster and VRF repos (`cartridge-gg/paymaster`, `cartridge-gg/vrf`) can be built with standard Rust toolchains on all target platforms
- Sidecar binaries don't depend on LLVM/cairo-native, so one build per platform suffices
- `~/.katana/` is already an established directory convention (`~/.katana/db` exists)
- The sidecar binaries are versioned in lockstep with katana (released as assets on katana's GitHub release)

## Requirements

### Functional

- A `sidecar-versions.toml` file in the repo root pins the git repo URL and rev for each sidecar
- CI builds `paymaster-service` and `vrf-server` for all 4 platform targets (linux-amd64, linux-arm64, darwin-arm64, win32-amd64)
- Each sidecar binary is uploaded as a separate release artifact (e.g., `paymaster-service_{version}_{platform}_{arch}.tar.gz`)
- A `checksums.txt` file is generated and included in the release with SHA256 hashes of all artifacts
- Binary resolution order: `--paymaster.bin`/`--vrf.bin` flag > PATH > `~/.katana/bin/` > lazy download
- On lazy download: print what will be downloaded, prompt for user confirmation, download, verify checksum, install to `~/.katana/bin/`
- On version mismatch: detect installed sidecar version doesn't match expected, prompt user to re-download

### Non-Functional

- Sidecar builds should run in parallel with katana builds in CI (no added latency to release pipeline)
- Downloaded binaries must be verified via SHA256 checksum before execution
- Lazy install should work without any pre-existing toolchain (just needs network access)

## Technical Design

### Sidecar Version Pinning

New file `sidecar-versions.toml` at repo root:

```toml
[paymaster-service]
repo = "https://github.com/cartridge-gg/paymaster"
rev = "4748365"
package = "paymaster-service"

[vrf-server]
repo = "https://github.com/cartridge-gg/vrf.git"
rev = "6d1c0f60a53558f19618b2bff81c3da0849db270"
package = "vrf-server"
```

CI reads this file to determine what to build. The revs are the source of truth — updated via PRs when upgrading sidecar versions.

### Release Artifact Layout

For a release `v1.2.3`, the GitHub release would contain:

```
# Katana binaries (existing)
katana_v1.2.3_linux_amd64.tar.gz
katana_v1.2.3_linux_amd64_native.tar.gz
katana_v1.2.3_linux_arm64.tar.gz
katana_v1.2.3_linux_arm64_native.tar.gz
katana_v1.2.3_darwin_arm64.tar.gz
katana_v1.2.3_darwin_arm64_native.tar.gz
katana_v1.2.3_win32_amd64.zip

# Sidecar binaries (new)
paymaster-service_v1.2.3_linux_amd64.tar.gz
paymaster-service_v1.2.3_linux_arm64.tar.gz
paymaster-service_v1.2.3_darwin_arm64.tar.gz
paymaster-service_v1.2.3_win32_amd64.zip
vrf-server_v1.2.3_linux_amd64.tar.gz
vrf-server_v1.2.3_linux_arm64.tar.gz
vrf-server_v1.2.3_darwin_arm64.tar.gz
vrf-server_v1.2.3_win32_amd64.zip

# Checksums (new)
checksums.txt
```

### Binary Resolution Architecture

```
resolve_sidecar_binary(name, explicit_path)
  |
  ├─ explicit_path provided? → validate exists → use it
  |
  ├─ search PATH → found? → check version → match? → use it
  |                                         → mismatch? → prompt re-download
  |
  ├─ search ~/.katana/bin/ → found? → check version → match? → use it
  |                                                  → mismatch? → prompt re-download
  |
  └─ not found → prompt user to download
                  → download from GitHub release
                  → verify SHA256 checksum
                  → install to ~/.katana/bin/
                  → set executable permission
                  → use it
```

### Version Detection

Each sidecar binary should support a `--version` flag. Katana calls `<binary> --version` and compares the output against the expected version (derived from katana's own version or from `sidecar-versions.toml` embedded at build time).

If the sidecar binaries don't currently support `--version`, we may need to:
- Check if they already do (likely, as most Rust CLIs do)
- If not, fall back to a simple "file exists" check and skip version validation initially

### Download and Install Flow

```
1. Print: "paymaster-service not found. Download v1.2.3 for {platform}? [y/N]"
2. Wait for user confirmation (stdin)
3. Download: https://github.com/dojoengine/katana/releases/download/v1.2.3/paymaster-service_v1.2.3_{platform}_{arch}.tar.gz
4. Download: checksums.txt from same release
5. Verify SHA256 of downloaded archive against checksums.txt
6. Extract binary to ~/.katana/bin/
7. chmod +x (on Unix)
8. Proceed with sidecar startup
```

---

## Implementation Plan

### Serial Dependencies (Must Complete First)

#### Phase 0: Version Pinning File + Resolution Infrastructure

**Prerequisite for:** All subsequent phases

| Task | Description | Output |
|------|-------------|--------|
| 0.1 | Create `sidecar-versions.toml` in repo root with paymaster and vrf repo URLs and revs | `sidecar-versions.toml` |
| 0.2 | Create a `katana-sidecar` crate (or module in `katana-cli`) with: binary resolution logic (explicit path → PATH → `~/.katana/bin/`), version checking (`--version` call), download + checksum verification, user prompting, and install-to-disk | New crate/module with `resolve_or_install()` async fn |
| 0.3 | Embed the expected sidecar version into the katana binary at build time (e.g., via `build.rs` reading `sidecar-versions.toml`, or simply using katana's own version string) | Compile-time constant for expected sidecar versions |

---

### Parallel Workstreams

These workstreams can be executed independently after Phase 0.

#### Workstream A: CI — Build Sidecar Binaries

**Dependencies:** Phase 0 (needs `sidecar-versions.toml`)
**Can parallelize with:** Workstreams B, C

| Task | Description | Output |
|------|-------------|--------|
| A.1 | Add a `build-sidecars` job to `release.yml` that reads `sidecar-versions.toml` and builds `paymaster-service` and `vrf-server` via `cargo install` for each platform target (linux-amd64, linux-arm64, darwin-arm64, win32-amd64). Use the same runner matrix pattern as the existing `release` job but without LLVM/native setup. | New CI job producing sidecar binaries |
| A.2 | Archive sidecar binaries as separate artifacts following the naming convention: `{name}_{version}_{platform}_{arch}.tar.gz` (`.zip` for Windows) | Uploaded release artifacts |
| A.3 | Add a `generate-checksums` job that runs after both `release` and `build-sidecars` jobs, downloads all artifacts, generates `checksums.txt` with SHA256 hashes, and uploads it as a release artifact | `checksums.txt` artifact |
| A.4 | Update `create-draft-release` job to include sidecar artifacts and `checksums.txt` in the GitHub release | Updated release with all artifacts |

#### Workstream B: Runtime — Lazy Install Logic in Katana

**Dependencies:** Phase 0 (needs resolution module)
**Can parallelize with:** Workstreams A, C

| Task | Description | Output |
|------|-------------|--------|
| B.1 | Implement the download function: construct GitHub release URL from katana version + platform detection, download archive, download `checksums.txt`, verify SHA256, extract to `~/.katana/bin/`, set executable permissions | Download + verify + install functions |
| B.2 | Implement version mismatch detection: call `<binary> --version`, parse output, compare against expected version. Handle cases where binary doesn't support `--version` gracefully. | Version check function |
| B.3 | Implement user prompting: detect if stdin is a TTY (interactive), print download prompt, read y/N response. If not a TTY (e.g., CI/scripts), fail with a clear error message telling the user how to install manually. | Prompt function with TTY detection |
| B.4 | Wire the new resolution logic into the existing `bootstrap_paymaster()` and `bootstrap_vrf()` functions in `crates/cli/src/sidecar.rs`, replacing the current `resolve_executable()` in `crates/paymaster/src/lib.rs` | Updated sidecar bootstrap |

#### Workstream C: Platform Detection

**Dependencies:** Phase 0
**Can parallelize with:** Workstreams A, B

| Task | Description | Output |
|------|-------------|--------|
| C.1 | Implement platform detection function that maps the current OS and architecture to the release artifact naming convention (`linux`/`darwin`/`win32` + `amd64`/`arm64`). Use `std::env::consts::OS` and `std::env::consts::ARCH`. | `detect_platform() -> (platform, arch)` |

---

### Merge Phase

After parallel workstreams complete, these tasks integrate the work.

#### Phase N: Integration and Testing

**Dependencies:** Workstreams A, B, C

| Task | Description | Output |
|------|-------------|--------|
| N.1 | Integration test: verify that katana with `--paymaster` correctly resolves a binary from `~/.katana/bin/` in a test environment | Test in `crates/cli/` or integration tests |
| N.2 | Test the CI workflow on a feature branch by triggering a test release (or dry-run the workflow) to verify sidecar binaries are built and uploaded correctly | Successful test release |
| N.3 | Update documentation / CLI help text to mention auto-install behavior and `~/.katana/bin/` | Updated help strings |
| N.4 | Manual smoke test: fresh machine (or clean `~/.katana/bin/`), run `katana --paymaster`, verify prompt appears, download works, checksum passes, service starts | Verified end-to-end flow |

---

## Testing and Validation

- **Unit tests:** Resolution logic (PATH lookup, `~/.katana/bin/` fallback, version parsing)
- **Unit tests:** Checksum verification (correct hash passes, wrong hash fails)
- **Unit tests:** Platform detection returns correct values
- **Integration test:** Full sidecar bootstrap with binary in `~/.katana/bin/`
- **CI validation:** Trigger test release, verify all 8 sidecar artifacts + checksums.txt are present
- **Manual test:** End-to-end lazy install on clean environment

## Rollout and Migration

- **No breaking changes:** Existing `--paymaster.bin` and `--vrf.bin` flags continue to work as before (highest priority in resolution)
- **No migration needed:** Users who already have sidecars in PATH are unaffected
- **New behavior is additive:** The lazy download only triggers when no binary is found at all
- **Rollback:** If issues arise, users can always manually install binaries and use explicit `--*.bin` flags

## Verification Checklist

- [ ] `sidecar-versions.toml` exists and contains correct revs
- [ ] `cargo build -p katana` succeeds with the new resolution module
- [ ] `cargo nextest run -p katana-cli` passes (or whichever crate hosts the new logic)
- [ ] CI release workflow builds all 8 sidecar artifacts (4 platforms x 2 services)
- [ ] `checksums.txt` is generated and contains SHA256 hashes for all release artifacts
- [ ] Running `katana --paymaster` without binary installed prompts for download
- [ ] Running `katana --paymaster` with correct binary in `~/.katana/bin/` starts normally
- [ ] Running `katana --paymaster` with outdated binary prompts for re-download
- [ ] Running `katana --paymaster --paymaster.bin /path/to/binary` bypasses all resolution logic
- [ ] Non-TTY mode (piped stdin) fails with clear error message instead of hanging on prompt

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| Sidecar repos fail to cross-compile for some targets | Medium | High | Test builds early on all platforms; fall back to `cargo install` instructions in error message |
| Sidecar binaries don't support `--version` flag | Medium | Low | Fall back to existence check only; add version file alongside binary |
| GitHub release download blocked by corporate firewalls | Low | Medium | Clear error message with manual install instructions and direct download URL |
| Checksum mismatch due to release artifact corruption | Low | High | Fail loudly with instructions to retry or report issue |
| CI build time increase from building sidecars | Low | Low | Sidecar builds run in parallel with katana builds, no added latency |

## Open Questions

- [ ] Do `paymaster-service` and `vrf-server` support `--version` flags? If not, how should we detect installed versions? (Alternative: write a `.version` file next to the binary during install)
- [ ] Should we support `KATANA_HOME` env var to override `~/.katana/` base path?
- [ ] Cross-compilation: can the sidecar repos be built with `--target` on GitHub runners, or do they need native runners per platform? (The katana release workflow uses native runners for each platform, so this is likely fine)

## Decision Log

| Decision | Rationale | Alternatives Considered |
|----------|-----------|------------------------|
| Separate release artifacts (not bundled in katana tarball) | Keeps katana download small; sidecars are optional features | Bundled in same tarball — simpler but bloats download for users who don't need sidecars |
| Resolution: PATH → ~/.katana/bin/ → lazy download | PATH-first respects existing installs; ~/.katana/bin/ is a known convention; lazy download is last resort | Sibling directory (next to katana binary) — doesn't work well with package managers |
| Prompt before download | Avoids surprising network requests; important for CI/scripted environments | Auto-download silently — faster but could surprise users or break non-interactive flows |
| Non-native builds only for sidecars | Paymaster/VRF don't use Cairo execution | Build all variants — unnecessary complexity and build time |
| Version pinning via sidecar-versions.toml | Single source of truth, easy to update in PRs | Hardcoded in CI workflow — harder to review; Cargo.toml extraction — fragile parsing |
| Checksum verification | Security best practice for downloading and executing binaries | No verification — too risky for executable downloads |
| Version check + prompt on mismatch | Prevents subtle compatibility issues from stale binaries | Always use what's found — simpler but risky; overwrite silently — could break user's intentional setup |
| All platforms including Windows | Matches katana's platform coverage | Linux + macOS only — simpler but excludes Windows users |
