# TODOS

## Bootstrap TUI

- **Bounded concurrency in refresh task.** The Settings-change refresh fires
  one RPC call per Done item via `FuturesUnordered`. Currently unbounded.
  **Why:** a 50+ item manifest flipping RPC fires 50+ concurrent HTTP calls,
  which could be rate-limited by some nodes.
  **Pros:** graceful degradation for large manifests.
  **Cons:** adds bounded-semaphore code for the common case (2-10 items)
  where it does not matter.
  **Context:** introduced by refactor/bootstrap. See
  `crates/bootstrap/src/tui.rs` refresh task, and the eng-review test plan.
  Revisit when a user reports rate-limit errors, or when the manifest size
  routinely exceeds 20 items. Target: cap at 8 concurrent in-flight requests.
  **Depends on / blocked by:** none.

- **Refresh trigger on Account change (with UDC unique/non-unique nuance).**
  Today, refresh triggers only on RPC URL change. Account change also invalidates
  Done status for UDC deploys where `unique = true` (address depends on caller).
  For `unique = false`, address is salt-only, so Done status remains valid.
  **Why:** users changing signer mid-session may see stale Done badges.
  **Pros:** correctness for all UDC deploy modes.
  **Cons:** needs per-item inspection of UDC unique flag; two-pass logic.
  **Context:** `crates/bootstrap/src/plan.rs` DeployStep carries the unique
  flag. Refresh task would need to branch on it: unique=true requires
  recomputing address against new account, unique=false uses existing address.
  **Depends on / blocked by:** the initial refactor landing.
