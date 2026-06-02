# Self-hosted Cartridge Controller keychain — local fork changes

To drive a **Cartridge Controller against a local custom appchain** (the cross-chain
demos), the hosted keychain at `x.cartridge.gg` isn't enough — it's built for
Cartridge-known chains. We run a **self-hosted keychain** from the open
`cartridge-gg/controller` repo with a few local modifications. Those modifications
live in a throwaway clone (e.g. `/tmp/controller-ref`), so this directory records
them durably.

- **Base:** `cartridge-gg/controller` @ `4357514` ("feat: auto detect SMS country
  code (#2608)") — this is keychain/SDK version **0.13.12**, matching the
  `@cartridge/controller` the demo apps bundle, so the wire protocol lines up.
- **Patch:** [`keychain.patch`](./keychain.patch) — `git apply` it on top of `4357514`.

## What each change does

1. **`packages/keychain/vite.config.ts`** — dev server on **port 3010** over
   **trusted HTTPS** (mkcert certs, required: WebAuthn needs a secure context and
   the keychain is loaded cross-origin as an iframe). Also adds a vite **proxy**
   `/__cartridge` → `https://api.cartridge.gg` (`changeOrigin`, and spoofs the
   `Origin` header to `https://x.cartridge.gg`). This is the **CORS workaround**:
   `api.cartridge.gg` only sends CORS headers for Cartridge-owned origins, so a
   localhost keychain can't call it from the browser — the proxy makes the browser
   talk same-origin to vite, which forwards server-side.

2. **`packages/keychain/.env.dev`** — points the keychain at our setup:
   - `VITE_ORIGIN="https://localhost:3010"`
   - `VITE_CARTRIDGE_API_URL="https://localhost:3010/__cartridge"` (the proxied path)
   - `VITE_RPC_SEPOLIA="http://localhost:5050"` (the local settlement node; the
     demo's settlement runs with chain id `SN_SEPOLIA`)
   - `VITE_RP_ID` stays `localhost` (WebAuthn RP). Note: this means a **fresh local
     Controller** (a `localhost`-scoped passkey), not your `cartridge.gg` account.

3. **`packages/keychain/src/components/ExecutionContainer.tsx`** — re-run the fee
   estimate when the **controller (chain) changes**, not only when the calls change.
   On a chain switch a new `Controller` (new RPC) is created a render later; without
   this, the Review screen keeps a stale "Contract not found" fee error from the
   previous chain.

4. **`packages/keychain/src/components/simulation/use-simulate.ts`** — the
   balance-change preview simulates a tx **from the controller account**. If the
   controller isn't deployed on the target chain yet (it deploys on first execute,
   e.g. on an appchain), the sim's sender doesn't exist and starknet.js rejects with
   "Contract not found" → a red **"Simulation Error"** on the Review screen even
   though the real execute (which deploys the controller first) succeeds. The fix:
   in the `.catch`, check whether the controller is deployed; if not, skip the
   preview (`setIsError(false)`) instead of flagging an error. Genuine reverts
   (deployed sender) still surface.

## Not in the patch (regenerate / context)

- **`packages/keychain/.certs/`** — mkcert TLS certs (gitignored, machine-specific).
  Regenerate: `CAROOT=~/.vite-plugin-mkcert ~/.vite-plugin-mkcert/mkcert \
  -cert-file .certs/localhost.pem -key-file .certs/localhost-key.pem localhost 127.0.0.1 ::1`
  (reuses the already-trusted mkcert CA).
- **`packages/keychain/src/components/provider/tokens.tsx`** — a chain-aware
  appchain-fee-token hardcode was tried, then **reverted**: it's no longer needed
  once the appchain hosts STRK at the canonical address (katana branch
  `feat/rollup-canonical-strk-fee-token`, now in this demo branch). Don't re-add it.

## Setup from scratch

```bash
git clone https://github.com/cartridge-gg/controller /tmp/controller-ref
cd /tmp/controller-ref && git checkout 4357514
git apply /path/to/keychain-fork/keychain.patch
# regenerate .certs (see above)
pnpm install && pnpm build:deps
pnpm keychain dev          # → https://localhost:3010
```

Then point the demo app at it: `app/.env.local` → `VITE_KEYCHAIN_URL=https://localhost:3010`.

## Upstreaming

These are local workarounds. The ones worth upstreaming to `cartridge-gg/controller`:
the `ExecutionContainer` re-estimate-on-chain-change and the `use-simulate`
not-yet-deployed tolerance (both make the keychain behave on a chain where the
Controller deploys lazily). The HTTPS/proxy/env bits are demo-host config, not
upstream changes. See the demo's `docs/client.md` "Current known blockers" and the
agent memory `project_controller_on_appchain_setup` for the full picture.
