# Release Reproducibility

Katana release binaries use a vendored Cargo dependency archive to reduce non-determinism from network/package source drift.

## Vendored Artifacts

- `third_party/cargo/vendor.tar.gz.part-*`
- `third_party/cargo/vendor.tar.gz.sha256`
- `third_party/cargo/VENDOR_MANIFEST.lock`

The archive is stored as split parts to remain below GitHub's per-file blob limit.

The manifest records:

- SHA-256 of `Cargo.lock`
- SHA-256 of the dependency archive
- archive path metadata used by CI/release validation

## Updating Vendored Dependencies

After any dependency update (`cargo add`, `cargo update`, manifest edits), refresh vendored artifacts:

```bash
make vendor-refresh
```

Commit all of the following in the same PR:

- `Cargo.lock`
- `third_party/cargo/vendor.tar.gz.part-*`
- `third_party/cargo/vendor.tar.gz.sha256`
- `third_party/cargo/VENDOR_MANIFEST.lock`

## Validation

Run local verification:

```bash
make vendor-verify
```

Verification checks:

- archive checksum matches `vendor.tar.gz.sha256`
- manifest archive hash matches the archive
- manifest lock hash matches `Cargo.lock`
- archive extract sanity checks (`cargo-home` layout)

CI enforces this for pull requests and blocks drift.

## Release Workflow Behavior

`release.yml` now:

- validates vendored artifacts before building
- builds Katana with `--locked --offline --frozen` via `scripts/release/build-katana-vendored.sh`
- runs a reproducibility gate (two clean Linux amd64 builds with matching SHA-256)
