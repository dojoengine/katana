# Self-hosted Cartridge Controller keychain — local fork changes

To drive a **Cartridge Controller against a local custom appchain** (the cross-chain
demos), the hosted keychain at `x.cartridge.gg` isn't enough — it's built for
Cartridge-known chains, and the deployed build also lags `main`. We run a
**self-hosted keychain** from the open `cartridge-gg/controller` repo with a little
local config. This directory records that config durably (the clone is throwaway,
e.g. `/tmp/controller-ref`).

- **Patch:** [`keychain.patch`](./keychain.patch) — the **local self-hosting config
  only** (`vite.config.ts` + `.env.dev`). `git apply` it on top of a controller
  checkout. Two behavioral fixes that used to live here were **merged upstream**
  (see below), so they're no longer in the patch.
- **Use a controller checkout at/after the upstream fixes** — commit `00344102`
  (the `#2609` merge, 2026-06-03) or later `main`. That base is a descendant of
  `4357514` (== SDK `0.13.12`, which the demo apps bundle), so the keychain↔dapp wire
  protocol still lines up.

## In the patch (local self-hosting config — you must apply these)

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

## Merged upstream (now on controller `main` — NOT in the patch)

Both went in via **`cartridge-gg/controller#2609`** (merge commit `00344102`,
2026-06-03). A controller checkout at/after that has them already.

3. **`ExecutionContainer.tsx`** — re-run the fee estimate when the **controller
   (chain) changes**, not only when the calls change. On a chain switch a new
   `Controller` (new RPC) is created a render later; without this the Review screen
   keeps a stale "Contract not found" fee error from the previous chain.

4. **`use-simulate.ts`** — the balance-change preview simulates a tx **from the
   controller account**. If the account isn't deployed on the target chain yet (it
   deploys on first execute), the sim's sender doesn't exist → starknet.js rejects
   with "Contract not found" → a red **"Simulation Error"** even though the real
   execute (which deploys the account first) succeeds. Fix: when the sim fails, check
   whether the account is deployed; if not, skip the preview instead of erroring.

> Also relied on, but already in `main` (not ours): `switchChain.ts` rebuilds
> `window.controller` with the new RPC on a chain switch — this is why we self-host
> from `main` rather than use the older deployed `x.cartridge.gg` keychain.

## Not in the patch (regenerate / context)

- **`packages/keychain/.certs/`** — mkcert TLS certs (gitignored, machine-specific).
  Regenerate: `CAROOT=~/.vite-plugin-mkcert ~/.vite-plugin-mkcert/mkcert \
  -cert-file .certs/localhost.pem -key-file .certs/localhost-key.pem localhost 127.0.0.1 ::1`
  (reuses the already-trusted mkcert CA).
- **`packages/keychain/src/components/provider/tokens.tsx`** — a chain-aware
  appchain-fee-token hardcode was tried, then **reverted**: it's no longer needed
  once the appchain hosts STRK at the canonical address (katana
  `feat/rollup-canonical-strk-fee-token`, in this demo branch). Don't re-add it.

## Setup from scratch

```bash
git clone https://github.com/cartridge-gg/controller /tmp/controller-ref
cd /tmp/controller-ref && git checkout 00344102   # the #2609 merge, or later main
git apply /path/to/keychain-fork/keychain.patch   # local self-hosting config
# regenerate .certs (see above)
pnpm install && pnpm build:deps
pnpm keychain dev          # → https://localhost:3010
```

Then point the demo app at it: `app/.env.local` → `VITE_KEYCHAIN_URL=https://localhost:3010`.

See the demo's `docs/client.md` "Current known blockers" and the agent memory
`project_controller_on_appchain_setup` for the full picture.
