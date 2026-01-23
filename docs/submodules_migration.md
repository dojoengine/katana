## Submodule Migration Plan (Future)

Goal: remove vendored `crates/*` submodules once development stabilizes and replace them with versioned dependencies.

1. Replace path-based Cargo deps with git/registry deps in `Cargo.toml`.
2. Update any local tooling/scripts that assume `crates/*` directories exist.
3. Remove submodule entries from `.gitmodules` and delete submodule directories.
4. Update `THIRD_PARTY_NOTICES.md` to reflect new dependency sources.
