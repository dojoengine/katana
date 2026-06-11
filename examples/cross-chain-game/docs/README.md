# Building a Katana appchain application

A practical guide to building an application that runs on its own **Katana
appchain** and settles to a Starknet layer below it. It explains the
architecture, why each service in the stack exists, how the contracts are
structured, and how a client reads on-chain state.

The guide is **generalized** — the patterns apply to any appchain app — but every
section is grounded in one concrete worked example: the **Cross-Chain Dice** demo
that lives in this directory (`../`). When you see a `path:line` reference, it
points at that demo's real code so you can see the pattern in action.

> New here? Run the demo first (`../README.md`) so the moving parts are concrete,
> then come back and read why it's built the way it is.

## What "an app as a Katana appchain" means

Instead of deploying your app's contracts onto a shared chain, you run your own
[Katana](https://github.com/dojoengine/katana) instance as a dedicated
**appchain** ("L2") and **settle** it onto a chain you trust ("L1"). You get:

- **Your own block space** — no competing for throughput or fee markets.
- **Custom execution** — fees off, custom genesis, custom block time.
- **A trust anchor** — state roots (and cross-chain messages) are settled onto
  the L1 by a prover, so the L1 — and anything on it — can verify what happened.

The cost is operational: an appchain is a small distributed system. This guide is
mostly about understanding that system.

## The stack at a glance

```
                         buy → mint  (L1 → L2)
   ┌───────────────────────────┐ ─────────────────────► ┌──────────────────────────┐
   │  Settlement Katana ("L1")  │  relayed as L1-handler  │   Appchain Katana ("L2")  │
   │  • piltover core           │                         │  (rollup, --tee mock)     │
   │  • your settlement world   │ ◄───────────────────── │  • your appchain world    │
   └───────────────────────────┘  msg (L2 → L1) + saya    └──────────────────────────┘
        │ Torii (indexer)                                      │ Torii (indexer)
        └──────────────────────── client / UI ◄────────────────┘
                          (reads models + events via Torii)
   saya  ── proves each appchain block, submits update_state to piltover ──┘
```

Five kinds of process, explained in [services.md](./services.md):

| Component | Role |
| --- | --- |
| **Katana** (settlement) | The chain you settle to — acts as "L1". Hosts the piltover core. |
| **Katana** (appchain) | Your app's chain — a rollup that settles to the piltover core. |
| **piltover core** | The settlement-side messaging contract: the L1↔L2 mailbox + settled state. |
| **saya** | Proves appchain blocks and submits state updates to piltover (enables L2→L1). |
| **Torii** | Indexes a chain's world into a queryable DB so the client can read it. |

## The spine: one action, end to end

Everything in the demo is a variation on this round trip. Follow it once and the
rest of the guide is just detail.

1. **Act on L1.** The client calls a settlement contract that, in turn, calls
   `piltover.send_message_to_appchain(...)` (the demo: *Insert coin* → the `store`
   world's `buy_game`). piltover emits `MessageSent`.
2. **Relay to L2.** The appchain Katana (`--messaging.enabled`) sees the message
   and relays it as an `L1HandlerTx` that runs your `#[l1_handler]` (the demo:
   `mint_game` adds a credit). → [contracts.md](./contracts.md)
3. **Act on L2.** The client calls a system on the appchain (the demo: `play_game`
   rolls a score). The system writes a model and, to talk back to L1, calls
   `send_message_to_l1`.
4. **Settle.** saya proves the appchain block and submits `update_state` to
   piltover, which **registers** the outbound message hash. → [services.md](./services.md)
5. **Consume on L1.** The client calls your settlement system (the demo:
   `claim_score`), which calls `piltover.consume_message_from_appchain(...)` —
   this only succeeds because step 4 registered the message.
6. **Index + render.** Every write updated a model / emitted an event; **Torii**
   indexes both. The client reads the new state from Torii and re-renders.
   → [client.md](./client.md)

## Read this in order

1. **[architecture.md](./architecture.md)** — the application model: worlds,
   models, systems, events, and the two-chain split.
2. **[services.md](./services.md)** — each service: why it's needed, where it
   sits, how it works.
3. **[contracts.md](./contracts.md)** — Dojo contracts and both messaging
   directions.
4. **[deployment.md](./deployment.md)** — build, migrate, and orchestrate the
   whole stack.
5. **[client.md](./client.md)** — how the client queries appchain state.

## Prerequisites for the worked example

The same as the demo (`../README.md`): the `katana` binary, `bun`, the Dojo
toolchain (`sozo`, `torii` — pinned in `../.tool-versions`), a sibling `dojo`
checkout, and patched `saya-tee`/`saya-ops`. You don't need them installed to
read the guide, but you do to run the example commands.
