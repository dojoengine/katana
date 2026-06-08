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

## Deployed contracts

From a fresh deploy (`FRESH=1 CONTROLLER=1 ./up.sh`) on **2026-06-08**. Settlement is real
**Starknet Sepolia**; the appchain is the local `DUNGEON` rollup. The Sepolia contracts
(piltover, tokens, worlds) are **redeployed on every `FRESH=1`** — the always-current source
is `app/src/deployments.json`. The appchain world/system and the TEE-registry mock are
derived from fixed seeds/salts, so they're stable across redeploys.

### Settlement — Starknet Sepolia ([Voyager](https://sepolia.voyager.online))

| Contract | Address |
| --- | --- |
| piltover (rollup settlement core) | [`0x14ca1ec4f958c163afb5fb07c247074a53a3f736ac8e4a4b55196649da1bd4e`](https://sepolia.voyager.online/contract/0x14ca1ec4f958c163afb5fb07c247074a53a3f736ac8e4a4b55196649da1bd4e) |
| TEE registry (mock attestation) | [`0x37189b1807f1358074b70b3dc8ab79167bbf72cff1296286052f6dfe31c8f15`](https://sepolia.voyager.online/contract/0x37189b1807f1358074b70b3dc8ab79167bbf72cff1296286052f6dfe31c8f15) |
| GAME token (entry credit) | [`0x4c1ddd5c68e6797721c707565552e7d53e4bcd5a9be06d8943bb48a608d4b26`](https://sepolia.voyager.online/contract/0x4c1ddd5c68e6797721c707565552e7d53e4bcd5a9be06d8943bb48a608d4b26) |
| GOLD token (winnings) | [`0x7bf13b3acf30ec60de476dc1f7ef0f75dd2a9d8588212ecfa8b2f6f2d689165`](https://sepolia.voyager.online/contract/0x7bf13b3acf30ec60de476dc1f7ef0f75dd2a9d8588212ecfa8b2f6f2d689165) |
| bank world | [`0x20079c4e09dce36eb6d0e5b571485181a5a0cf1ef387830d648f10c8cbc9cbc`](https://sepolia.voyager.online/contract/0x20079c4e09dce36eb6d0e5b571485181a5a0cf1ef387830d648f10c8cbc9cbc) |
| bank system (consumes withdrawals → mints GOLD) | [`0x582023a0e94bc90363ee1dc40877c773da21c4b69c52a2074845253603990ba`](https://sepolia.voyager.online/contract/0x582023a0e94bc90363ee1dc40877c773da21c4b69c52a2074845253603990ba) |
| Entry (charge GAME + L1→L2 enter) | [`0x3113d4c637c135fe325e745fc7ecb72e17fefa152c2e8ca07fdb91841cdec87`](https://sepolia.voyager.online/contract/0x3113d4c637c135fe325e745fc7ecb72e17fefa152c2e8ca07fdb91841cdec87) |
| TokenSale (USDC→GAME — inert: `usdc` unset, UI uses Dev-mint) | [`0x32dc683d0c2216648c89fdc3a206c602f5f80c88231b88d8d31cb2c7c208013`](https://sepolia.voyager.online/contract/0x32dc683d0c2216648c89fdc3a206c602f5f80c88231b88d8d31cb2c7c208013) |

### Appchain — local `DUNGEON` rollup (`http://localhost:5070`)

| Contract | Address |
| --- | --- |
| game world | `0x7f6c1c800301783a1a5a9378a6c3cdf237ad9ae21bb715c0bf5e408a450ab6e` |
| game system (move / attack / loot / use / extract / withdraw) | `0x6d3d2eab82c4b17ee17eeae58f9981db04a8e9beeaa887b355ce7e57f085e97` |

### Cartridge Controller account classes (declared on the appchain)

All bundled versions are declared so a Controller auto-deploys at the version it was created
with (see `scripts/declare-controller-class.ts`).

| Version | Class hash |
| --- | --- |
| v1.0.9 (latest) | `0x743c83c41ce99ad470aa308823f417b2141e02e04571f5c0004e743556e7faf` |
| v1.0.8 | `0x511dd75da368f5311134dee2356356ac4da1538d2ad18aa66d57c47e3757d59` |
| v1.0.7 | `0x3e0a04bab386eaa51a41abe93d8035dccc96bd9d216d44201266fe0b8ea1115` |
| v1.0.6 | `0x59e4405accdf565112fe5bf9058b51ab0b0e63665d280b816f9fe4119554b77` |
| v1.0.5 | `0x32e17891b6cc89e0c3595a3df7cee760b5993744dc8dfef2bd4d443e65c0f40` |
| v1.0.4 | `0x24a9edbfa7082accfceabf6a92d7160086f346d622f28741bf1c651c412c9ab` |

### Accounts

| Role | Address |
| --- | --- |
| Operator (settlement deploys + dev signer) | [`0x00ddeE62091d2F9De6FF534a951a6202372Bfe1f3803ae5c1a73010F6AF4248f`](https://sepolia.voyager.online/contract/0x00ddeE62091d2F9De6FF534a951a6202372Bfe1f3803ae5c1a73010F6AF4248f) |
| saya (piltover operator — settles appchain state) | [`0x0639956bAB912477F04fa7b9189d014E785092E795b3B57E9481f89642cde52B`](https://sepolia.voyager.online/contract/0x0639956bAB912477F04fa7b9189d014E785092E795b3B57E9481f89642cde52B) |
| Appchain dev account (default play signer) | `0xdcbeb1f415c0c3e8ae300f3550ff9d649c03c2aeea5ec15f9862139ac3857b` |
