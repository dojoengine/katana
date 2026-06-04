# One Controller, both chains

[← client](./client.md) · [guide index](./README.md)

The demo runs on the **operator account** by default (no login). Optionally a single
**Cartridge Controller** can sign everything — buy / enter / bank on **real Sepolia**
*and* the dungeon play actions on the **local appchain** — at one address.

## How it works

`app/src/wallet.tsx` gives the `ControllerConnector` both chains and exposes two
signers — `l1Account` (Sepolia) and `l2Account` (appchain). Each wraps the raw
`controller.account` and **switches the keychain's chain** around the call: `l2Account`
switches to `shortString("DUNGEON")`, executes the play action, then switches back to
Sepolia for the next L1 op. The **player** is the Controller address (same on both
chains), so a run entered on Sepolia is played and banked by that same Controller.
Details in [client.md](./client.md#wallets-operator-default-optional-controller).

## Setup (hosted keychain)

By default the app connects to the **hosted Cartridge keychain** (`x.cartridge.gg`) —
your **real Cartridge Controller account**, already deployed by Cartridge. Just:

```bash
CONTROLLER=1 ./up.sh
```

`CONTROLLER=1` appends `--paymaster --cartridge.paymaster --cartridge.controllers` to
the **appchain** node (the settlement side is real Sepolia — Cartridge knows it
natively, so only the appchain needs the middleware). The controller account classes are
in the rollup genesis (katana #584), so the Controller deploys on the appchain at its
canonical address on first play. The frontend serves HTTPS (`mkcert`), required for the
passkey login.

Open `https://localhost:3002`, **Login → Connect Controller** (your cartridge.gg
account), then play: enter (Sepolia) → play (appchain) → withdraw → bank (Sepolia), all
keyed on the one Controller address.

### Funding

Sepolia ops (buy / enter / bank) cost real gas; the appchain is free (`--dev.no-fee`).
Your Controller is a real account, so **fund it with a little STRK on Sepolia** (or rely
on Cartridge paymaster sponsorship if your app is set up for it). For **GAME** to enter,
hit **Dev-mint** (a session policy) once funded, or buy with USDC.

## Gotchas

- **Chrome may block the keychain → localhost appchain** (Private Network Access). The
  play actions need the keychain to reach `:5070`; if a roll stalls, enable
  `chrome://flags/#local-network-access-check`.
- **Per-chain sessions** — the appchain session isn't pre-approved on connect, so the
  first play may show a confirm modal rather than being silent.
- **HTTPS for WebAuthn** — `CONTROLLER=1 ./up.sh` serves `https://localhost:3002` via
  `mkcert`; passkey login refuses an untrusted cert.

The default operator path needs none of this — `./up.sh` and play.

## Self-hosted keychain (fully-local fallback)

For a fully-local Controller — a `localhost` passkey, no cartridge.gg account (e.g.
offline dev) — point the app at a self-hosted keychain instead:

1. Set up the keychain from the sibling fork
   [`../../cross-chain-game/keychain-fork/`](../../cross-chain-game/keychain-fork/README.md)
   (clone `cartridge-gg/controller`, apply `keychain.patch`, mkcert certs,
   `pnpm keychain dev` on `https://localhost:3010`). One dungeon override: set the fork's
   `VITE_RPC_SEPOLIA` to your **real Sepolia RPC** (`.env` `SETTLEMENT_RPC_URL`), not
   cross-chain-game's local node.
2. Point the app at it, then start:
   ```bash
   echo 'VITE_KEYCHAIN_URL=https://localhost:3010' > app/.env.local
   CONTROLLER=1 ./up.sh
   ```

A self-hosted Controller is a **fresh localhost passkey** (RP-id `localhost`), so its
Sepolia account starts undeployed and unfunded — send its address (shown on connect)
some STRK so it can self-deploy + pay. Keep the keychain (`:3010`) up before connecting.

---

Back to the [client](./client.md) read/write layer, or the
[guide index](./README.md).
