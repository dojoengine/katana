# Cross-Chain Dungeon

A Katana appchain example that **settles to a real Starknet network** (Sepolia by
default, **mainnet supported** via `SETTLEMENT_NETWORK`) and **depends
on an external settlement-layer contract (USDC)**. It's a push-your-luck dungeon
roguelite with a **two-token economy**: buy **GAME** with USDC and spend it to enter,
descend with **one appchain transaction per action**, collect **GOLD**, and either
**extract** (bank the run's gold into your on-L2 vault) or **die** (forfeit the
in-progress haul). Then **bank once** — withdraw the whole vault to Sepolia, where
GOLD is minted on L1. The point: appchain value is provisional until you commit it
to the settlement layer.

It's the sibling of [`../cross-chain-game`](../cross-chain-game). Where that demo
runs two local Katanas, this one runs **one** (the appchain) and settles to a real
public chain — and adds a token economy on top of an external contract.

> New to the appchain architecture? Read the [guide](./docs/README.md) — it builds
> the mental model (worlds, messaging, saya, Torii) using this game as the example.

## What's different from cross-chain-game

| | cross-chain-game | **cross-chain-dungeon** |
| --- | --- | --- |
| Settlement layer | local Katana (`SN_SEPOLIA`) | **real Starknet (Sepolia default, mainnet supported)** |
| Local nodes | 2 Katanas | **1** (appchain only) |
| Economy | none | **two tokens: GAME (USDC→play) + GOLD (winnings, minted on bank)** |
| External dependency | — | **Circle USDC on Sepolia** |
| Gameplay | one roll | **a dungeon run, one tx per action; vault many runs, bank once** |
| Ports | 5050/5051/8081/8082/3001 | **5070/8091/8092/3002** |
| Controller | both chains | **both chains** (hosted keychain; funded on real Sepolia) |

## Prerequisites

This is *not* fully one-click — settling to a real chain needs real accounts.

1. **katana** built from this repo: `( cd ../../ && cargo build --release )`.
2. **Patched saya v0.4.0** (`saya-ops`, `saya-tee`) on PATH — the Poseidon L1→L2
   hash fix. See [`../cross-chain-game/saya-patch`](../cross-chain-game/saya-patch).
3. **Dojo toolchain** (`sozo`/`torii`/`scarb`) via `asdf install` (pinned in
   `.tool-versions`), and a sibling **dojo** checkout (the cairo packages depend on
   it by path, ref `sozo/v1.8.7`).
4. **Bun**.
5. A funded Sepolia **operator** account and a separate funded **saya** account,
   and a **USDC** address — all in `.env` (see below).

## Run it

```bash
cp .env.example .env     # fill in SEPOLIA_RPC_URL, operator + saya accounts, USDC
./up.sh                  # appchain :5070, saya → Sepolia, torii ×2, frontend :3002
```

`up.sh` deploys the mock TEE registry + piltover core on Sepolia, starts the
appchain and saya, deploys the economy + worlds (`scripts/deploy.ts`), starts both
Torii indexers, and serves the client. `./down.sh` stops the local processes.

Then open `http://localhost:3002`, **Dev-mint** some GAME (or **Buy** it with
USDC), start a **New Game** (each dive is its own run — you can keep several open and
continue any of them from the lobby), play, **Extract** to bank gold into your vault,
then on the **Bank** tab withdraw the vault to Sepolia to mint **GOLD**.

## Funding & costs

Every deploy and every `saya update_state` is a **real Sepolia transaction**:

- The **operator** pays for the TEE registry, piltover, the GAME/GOLD/sale/entry
  contracts, and the bank-world migration.
- **saya** pays for `update_state` on every settled batch (recurring) — give it a
  **dedicated** funded account, never shared with the operator (nonce contention
  stalls settlement).
- The **player** path: `Dev-mint` needs only Sepolia gas (no USDC); `Buy` needs
  real test **USDC**. The dev-mint faucet exists so the demo is playable without it.

## Using Controller (optional)

By default the client signs with the **operator account** on Sepolia and the **dev
account** on the appchain — no login. The header **login** button can swap in a
[Cartridge Controller](https://github.com/cartridge-gg/controller) that signs on
**both chains** as one identity: buy / enter / bank on real Sepolia *and* the dungeon
play actions on the local appchain, at the same address. It needs `CONTROLLER=1
./up.sh` (Controller-capable appchain + HTTPS via `mkcert`) and a Cartridge Controller
login — the **hosted keychain** (x.cartridge.gg) by default, with a self-hosted keychain
as a fully-local fallback. Fund the Controller with a little STRK on real Sepolia. Full
walkthrough: [docs/controller.md](./docs/controller.md).

## What's where

| Path | What |
| --- | --- |
| `cairo/game` | appchain dungeon world (`game` namespace) — run, actions, GOLD vault, leaderboard |
| `cairo/score` | settlement `bank` world (`bank` namespace) — mints GOLD when a withdrawal settles |
| `cairo/token` | `game_token` (GAME), `gold_token` (GOLD), `token_sale` (USDC→GAME), `entry` (charge + L1→L2) |
| `scripts/` | `deploy.ts` + `lib.ts` (deploy economy + migrate worlds) |
| `app/` | React + Vite terminal client (`app/src/chain.ts`, `App.tsx`, `wallet.tsx`) |
| `design/ui-mockup.html` | the standalone terminal-UI design mockup |
| `up.sh` / `down.sh` | one-command bring-up / teardown |
| `docs/` | the architecture guide |
| `PLAN.md` | the full implementation spec |
