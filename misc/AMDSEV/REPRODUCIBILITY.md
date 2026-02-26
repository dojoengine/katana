# AMD SEV-SNP Build Reproducibility Policy

## Scope

This policy targets byte-identical outputs for the following artifacts when built with the same:
- source tree revision
- `build-config` pins
- `SOURCE_DATE_EPOCH`
- toolchain/runtime environment

Artifacts:
- `OVMF.fd`
- `vmlinuz`
- `initrd.img`
- `katana`

## Required Inputs

- `SOURCE_DATE_EPOCH` must be explicitly set and fixed.
- `OVMF_COMMIT` must be pinned.
- Package versions and SHA256 values in `build-config` must remain pinned.
- `BUILD_CONTAINER_IMAGE_DIGEST` should be set when using a containerized CI pipeline.
- For katana, prefer passing a prebuilt pinned binary via `--katana`. If auto-building, set `KATANA_STRICT_REPRO=1` with vendored dependencies.

## Stronger Package Source Determinism

To avoid host apt source drift, set:
- `APT_SNAPSHOT_URL`
- `APT_SNAPSHOT_SUITE`
- `APT_SNAPSHOT_COMPONENTS`

If unset, build scripts use host apt sources and reproducibility guarantees are weaker.

## Validation

- Use `./misc/AMDSEV/build.sh --repro-check` to run a double-build and hash comparison.
- Use `./misc/AMDSEV/verify-build.sh --compare DIR_A DIR_B` for explicit directory comparisons.

## Provenance Files

Each build emits:
- `build-info.txt` with pinned inputs and output checksums
- `materials.lock` with immutable input and artifact hashes
