# One Controller, both chains

[← client](./client.md) · [guide index](./README.md)

The demo runs on the **operator account** by default (no login). Optionally a single
**Cartridge Controller** can sign everything — buy / enter / bank on **real Sepolia**
*and* the dungeon play actions on the **local appchain** — at one address. This is the
same wiring as [cross-chain-game](../../cross-chain-game/docs/client.md), with one
extra wrinkle: the dungeon settles to **real Sepolia**, so the Controller needs a
little funding there.

## How it works

`app/src/wallet.tsx` gives the `ControllerConnector` both chains and exposes two
signers — `l1Account` (Sepolia) and `l2Account` (appchain). Each wraps the raw
`controller.account` and **switches the keychain's chain** around the call: `l2Account`
switches to `shortString("DUNGEON")`, executes the play action, then switches back to
Sepolia for the next L1 op. The **player** is the Controller address (same on both
chains), so a run entered on Sepolia is played and banked by that same Controller.
Details in [client.md](./client.md#wallets-operator-default-optional-controller).

Why this needs a **self-hosted keychain**: the hosted keychain (`x.cartridge.gg`) only
knows Cartridge chains, so it can't talk to a local appchain. We self-host the keychain
from the open `cartridge-gg/controller` repo and point the app at it. RP-id `localhost`
means a **fresh local Controller** (a localhost passkey), not your `cartridge.gg`
account.

## Setup

### 1. Self-hosted keychain (reuse the sibling fork)

The keychain config lives next door, in
[`../../cross-chain-game/keychain-fork/`](../../cross-chain-game/keychain-fork/README.md).
Follow that README to clone `cartridge-gg/controller` (`@00344102` or later), apply
`keychain.patch`, generate mkcert certs, and `pnpm keychain dev` (serves
`https://localhost:3010`).

**One dungeon-specific override:** the fork's `.env.dev` points `VITE_RPC_SEPOLIA` at
cross-chain-game's *local* settlement node. The dungeon settles to **real Sepolia**, so
set it to your real Sepolia RPC instead (the same URL as `SETTLEMENT_RPC_URL` in
`.env`, e.g. `https://api.cartridge.gg/x/starknet/sepolia/rpc/v0_9`). Keep
`VITE_RP_ID=localhost`, `VITE_ORIGIN=https://localhost:3010`, and the `/__cartridge`
API proxy.

### 2. Point the app at the keychain + start the stack

```bash
echo 'VITE_KEYCHAIN_URL=https://localhost:3010' > app/.env.local
CONTROLLER=1 ./up.sh
```

`CONTROLLER=1` appends `--paymaster --cartridge.paymaster --cartridge.controllers` to
the **appchain** node (the settlement side is real Sepolia — Cartridge knows it
natively, so only the appchain needs the middleware). The controller account classes
are already in the rollup genesis by default (katana #584), so the Controller
auto-deploys on the appchain at its canonical address on first play. The frontend is
HTTPS already (`mkcert`), required for the passkey login.

### 3. Login → Connect Controller, then fund it on Sepolia

Open `https://localhost:3002`, **Login → Connect Controller** (creates the localhost
passkey). The Controller signs the appchain for free (`--dev.no-fee`), but on **real
Sepolia it starts with nothing** — so before buy/enter/bank, give its address:

- **STRK** for gas — transfer some to the Controller's address (shown on connect).
- **GAME** to enter — once it has gas, hit **Dev-mint** (a session policy), or buy with
  USDC.

Then the full loop runs as the Controller: enter (Sepolia) → play (appchain) → withdraw
→ bank (Sepolia), all keyed on the one Controller address.

## Gotchas

- **Trusted cert is mandatory** — WebAuthn refuses an untrusted/`-k` cert. `mkcert
  -install` once.
- **Keychain up before you connect** — the dapp loads it as an iframe at
  `VITE_KEYCHAIN_URL`; if `:3010` is down, "Connect Controller" can't complete.
- **Chrome may block the iframe → localhost** (Private Network Access). If connect
  stalls, enable `chrome://flags/#local-network-access-check`.
- **Fund the Controller on Sepolia first** — unlike cross-chain-game (both chains local
  and free), the dungeon's Sepolia ops cost real gas; a fresh localhost Controller has
  none until you fund it.
- **Per-chain sessions** — the appchain session isn't pre-approved on connect, so the
  first play may show a confirm modal rather than being silent (same as cross-chain-game).
- **Ports** — keychain `:3010`, demo frontend `:3002`.

The default operator path needs none of this — `./up.sh` and play.

---

Back to the [client](./client.md) read/write layer, or the
[guide index](./README.md).
