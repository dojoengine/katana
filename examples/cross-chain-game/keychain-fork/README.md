# Self-hosted Cartridge Controller keychain — local fork (now a fallback)

> **You probably don't need this anymore.** The hosted keychain at `x.cartridge.gg`
> now drives a local custom appchain directly: it redeployed from `main` right after
> the fixes merged (`#2609`, 2026-06-03), so it carries both the behavioral fixes and
> the `switchChain` rebuild. Just run `CONTROLLER=1 ./up.sh` (no `app/.env.local`),
> declare the controller class once (the `#584` runtime step — see `docs/client.md`),
> and **Connect Controller** uses your real `cartridge.gg` account. If a connect/roll
> stalls, the only gotcha is Chrome Private Network Access
> (`chrome://flags/#local-network-access-check`) — the hosted iframe reaching
> `localhost`.

This directory is kept as a **fallback** for when you need to run against an
*unreleased* keychain change (before it's deployed to `x.cartridge.gg`), or want a
fully offline setup. It runs a **self-hosted keychain** from the open
`cartridge-gg/controller` repo with a little local config, recorded here durably (the
clone is throwaway, e.g. `/tmp/controller-ref`).

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

## Set up a working self-hosted keychain

Prereqs: `pnpm` 10, Node, and `mkcert` (`brew install mkcert`) for **trusted** local
TLS — WebAuthn refuses an untrusted cert, and the keychain runs as a cross-origin
iframe so it must be HTTPS.

**1. Clone the keychain and apply the local config.** (Run all keychain commands
from the clone; `keychain.patch` is in *this* directory.)

```bash
git clone https://github.com/cartridge-gg/controller /tmp/controller-ref
cd /tmp/controller-ref
git checkout 00344102            # the #2609 merge (or any later main)
git apply /ABS/PATH/TO/examples/cross-chain-game/keychain-fork/keychain.patch
```

The patch adds the https :3010 server + `/__cartridge` API proxy in `vite.config.ts`
and the matching `.env.dev`.

**2. Generate trusted localhost certs** (the patch points vite at `.certs/`):

```bash
mkcert -install                  # one-time: trust the local CA (OS keychain prompt)
mkdir -p packages/keychain/.certs
mkcert -cert-file packages/keychain/.certs/localhost.pem \
       -key-file  packages/keychain/.certs/localhost-key.pem \
       localhost 127.0.0.1 ::1
```

(If you've already run the demo frontend once, `vite-plugin-mkcert` left a trusted CA
you can reuse instead: `CAROOT=~/.vite-plugin-mkcert ~/.vite-plugin-mkcert/mkcert …`.)

**3. Install, build deps, and run the keychain** (serves `https://localhost:3010`):

```bash
pnpm install
pnpm build:deps                  # the keychain imports built @cartridge/* packages
pnpm keychain dev
```

Verify it's healthy:

```bash
curl https://localhost:3010/                         # 200 WITHOUT -k → cert is trusted
curl -X POST https://localhost:3010/__cartridge/query \
     -H 'content-type: application/json' --data '{"query":"{ __typename }"}'
# → {"data":{"__typename":"Query"}}   (the API proxy reaches api.cartridge.gg)
```

**4. Point the demo at it and start the stack** (from `examples/cross-chain-game`):

```bash
echo 'VITE_KEYCHAIN_URL=https://localhost:3010' > app/.env.local
CONTROLLER=1 ./up.sh             # demo on https://localhost:3001, paymaster on both nodes
```

**5. Controller class on the appchain** (the #584 gap — see `docs/client.md`): the
canonical class the keychain deploys (`0x743c8…`) must be declared on the appchain.
**`CONTROLLER=1 ./up.sh` already does this** at boot
(`scripts/declare-controller-class.ts`), so there's nothing to do here. (Only if you
run the keychain against some *other* appchain without the demo's `up.sh` would you
declare `controller.latest` yourself — run that script against your node.)

**6. Open `https://localhost:3001` → Login → Connect Controller.** RP id is
`localhost`, so this creates a **fresh local Controller** (a localhost-scoped
passkey), not your `cartridge.gg` account. Then buy → roll → bank.

### Gotchas

- **Trusted cert is mandatory.** A self-signed / `-k` cert fails with *"WebAuthn is
  not supported on sites with TLS certificate errors"*. `mkcert -install` fixes it.
- **The keychain must be running before you connect** — the dapp loads it as an
  iframe at `VITE_KEYCHAIN_URL`; if `:3010` is down, "Connect Controller" can't
  complete.
- **Chrome may block the iframe reaching `localhost`** (Private Network Access). If
  connect stalls, enable `chrome://flags/#local-network-access-check`.
- **Ports:** keychain `:3010`, demo frontend `:3001` — keep them distinct.

See the demo's `docs/client.md` "Current known blockers" and the agent memory
`project_controller_on_appchain_setup` for the full picture.
