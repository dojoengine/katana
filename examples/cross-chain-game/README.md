# Cross-Chain Game Store

A minimal end-to-end demo of **L1 → L2 messaging** with Katana. You buy a game on a
*settlement* chain; Katana relays the message to an *appchain*, which mints the game;
the UI reacts live to the new appchain state.

It exercises the three things a cross-chain app needs:

1. **Act on the settlement layer ("L1").** Clicking *Purchase Game* sends a tx that
   calls `send_message_to_appchain(...)` on a messaging contract, emitting a
   `MessageSent` event.
2. **Katana picks up the message.** The appchain node runs with `--messaging.enabled`
   pointed at the settlement node. Its collector sees `MessageSent` and submits an
   `L1HandlerTx` that invokes the `mint_game` `#[l1_handler]`.
3. **The UI reacts to L2 state.** `mint_game` mutates the appchain contract's storage;
   the React app polls a view function and flips the purchase **pending → confirmed**
   while the "total minted" counter climbs.

Both roles ("L1" and "L2") are Katana instances — the settlement layer is a second
Katana acting as a Starknet settlement chain via the piltover messaging mock.

```
┌─────────────────────────┐   send_message_to_appchain   ┌─────────────────────────┐
│  Settlement Katana :5050 │ ───────────────────────────► │   Appchain Katana :5051  │
│  messaging mock contract │      MessageSent event       │   game_minter contract   │
│      (the "L1")          │ ◄─── relayed by Katana ────  │       (the "L2")         │
└─────────────────────────┘     as an L1 handler tx       └─────────────────────────┘
            ▲                                                          │
            │ purchase (signed tx)                  poll total_minted()│
            └──────────────────────── React app :3001 ◄───────────────┘
```

## Prerequisites

- The `katana` binary built in this repo (`cargo build --release`), or `katana` on `PATH`.
- [`scarb`](https://docs.swmansion.com/scarb/) (compiles the appchain contract).
- [`bun`](https://bun.sh/) (deploy scripts + frontend).

`starkli`/foundry are **not** needed — contracts are declared/deployed with starknet.js.

## Run it

```bash
cd examples/cross-chain-game
./up.sh
```

`up.sh` builds the contract, starts both Katana nodes, deploys the contracts, writes
`app/src/deployments.json`, and serves the frontend. Open **http://localhost:3001** and
click *Purchase Game*. Press Ctrl-C to stop the nodes (or run `./down.sh`).

Both nodes run with `--explorer`, so each serves Katana's block explorer at its own
`/explorer` path (settlement: http://localhost:5050/explorer, appchain:
http://localhost:5051/explorer). Every transaction hash in the UI is a link: the **L1 tx**
opens the settlement explorer (the `send_message_to_appchain` invoke) and the **L2 tx**
opens the appchain explorer (the `mint_game` L1-handler) — the two halves of the round trip.

## What's where

| Path | Role |
| --- | --- |
| `cairo/src/lib.cairo` | Appchain (`L2`) contract: `mint_game` `#[l1_handler]` + view fns |
| `scripts/deploy-settlement.ts` | Declare/deploy the messaging mock on the settlement node |
| `scripts/deploy-appchain.ts` | Declare/deploy `game_minter` on the appchain node |
| `scripts/lib.ts` | Shared starknet.js helpers, RPC URLs, dev account |
| `app/` | React + Vite + TS frontend (Tailwind v4 + [shadcn/ui](https://ui.shadcn.com) components) |
| `up.sh` / `down.sh` | Start / stop the whole demo |

## How the message flows (the important details)

- The settlement contract's `send_message_to_appchain(to_address, selector, payload)`
  emits `MessageSent`. The appchain's messaging collector turns it into an
  `L1HandlerTx` with calldata `[from_address, ...payload]` — the settlement-side
  **caller** is always prepended as `from_address`.
- So `mint_game(ref self, from_address: felt252, game_id: felt252)` receives the buyer
  (the account that signed on the settlement chain) as `from_address`, and the payload
  `game_id` after it.
- The appchain node is wired to the settlement node purely via CLI flags — no chain
  spec file:

  ```
  katana --dev --dev.no-fee --http.port 5051 --http.cors_origins '*' --explorer \
    --messaging.enabled \
    --settlement.chain starknet \
    --settlement.rpc-url http://localhost:5050 \
    --settlement.core-contract <messaging-mock-address> \
    --messaging.from-block 0
  ```

## Notes

- `--dev.no-fee` keeps purchases frictionless. Both dev nodes use the default seed, so
  they share the same predeployed accounts; the demo signs with account #0. Its private
  key is a throwaway local dev key — **never** reuse this pattern with real funds.
- `--http.cors_origins '*'` lets the browser app read the RPC. Scope it down for
  anything beyond local development.
- `app/src/deployments.json` is regenerated on every `up.sh` run.
