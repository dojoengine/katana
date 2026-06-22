# Application architecture

[← Guide index](./README.md) · Next: [services →](./services.md)

This chapter is the mental model. Before the services and the wiring, you need to
know **where your app's state and logic live** and **how a client gets at them**.

## An appchain app is a world, not a pile of contracts

On a Katana appchain you build with the [Dojo](https://github.com/dojoengine/dojo)
framework. Dojo gives you a **world**: an on-chain database plus the systems that
write to it.

- **Models** are the tables — typed structs with a key. They *are* your state.
- **Systems** are the logic — contracts (`#[dojo::contract]`) whose functions
  read and write models through the world.
- **Events** are an append-only log — emitted by systems, ideal for feeds and
  history.
- The **world contract** ties it together: it stores model data, enforces
  permissions (which system may write which model), and is the single address a
  client/indexer points at.

Why this shape matters for an app: because state lives in **models with a known
schema**, an indexer ([Torii](./services.md)) can mirror it into a normal
database, and your client reads plain rows instead of decoding contract storage.
That read path is the whole reason the app is pleasant to build a UI for.

**In the demo.** The appchain world (namespace `game`) has a `Stats` model
(counters) and `GameConfig` (where to publish scores), one system `game` with the
gameplay functions, and `GameMinted` / `GamePlayed` events:

```cairo
#[derive(Copy, Drop, Serde)]
#[dojo::model]
pub struct Stats { #[key] pub id: u8, pub total_minted: u64, pub available: u64, /* … */ }
```
[`cairo/game/src/lib.cairo:35`](https://github.com/dojoengine/katana/blob/2e36ba5ae08b2f7c07e6e6a458464995e1d59a25/examples/cross-chain-game/cairo/game/src/lib.cairo#L35)

## The two-chain split: play on L2, anchor on L1

An appchain app usually spans **two chains**, and deciding what goes where is the
main design choice:

- **The appchain ("L2")** holds the high-frequency, app-specific state and logic —
  the part you want cheap, fast, and fully under your control.
- **The settlement layer ("L1")** holds the durable record and anything that must
  be verifiable by the outside world or other contracts on that layer.

They are connected by **cross-chain messages** in both directions (covered in
[contracts.md](./contracts.md)):

- **L1 → L2** is instant: the appchain relays a settlement message as a
  transaction. Use it for "an L1 action should cause an L2 effect."
- **L2 → L1** is *settled*: it only completes after the settler (the appchain's
  embedded settlement service) settles the appchain block onto L1. Use it for "an
  L2 result should become an L1 fact."

Because the two directions have different trust/latency, many apps put a **world
on each chain**: the appchain world for live state, a small settlement world for
the anchored record.

**In the demo.** Three worlds:

| World | Chain | Holds | Why there |
| --- | --- | --- | --- |
| `game` | appchain (L2) | credits, plays, scores | play is free + instant |
| `store` | settlement (L1) | the storefront (`buy_game`) | purchases are an L1 action |
| `score` | settlement (L1) | banked runs, leaderboard | the permanent, anchored record |

You buy a credit on L1 (it mints on L2), play on L2, then *bank* a score back to
L1 where it's settled and recorded — a complete L1→L2→L1 loop.

### Deciding what goes where

For each operation or piece of state, run it through this checklist:

**Keep it on L2 (the appchain)** if any of these hold:
- It's **high-frequency or latency-sensitive** — it should feel instant.
- It should be **cheap or free** to do (you control the fee market; the demo
  runs the appchain fee-less).
- **Only your app needs to trust it** — no outside contract or user has to verify
  it independently. (Gameplay, in-progress scores, transient state.)

**Put it on L1 (the settlement layer)** if any of these hold:
- It must be **permanent and verifiable by outsiders** — other L1 contracts,
  users, or services read it as ground truth. (Leaderboards, ownership, balances.)
- It involves **real value or payment** (charging a token, minting an asset),
  where you want L1's security and finality.
- It's a **low-frequency "commit"** action — the moment a result becomes
  official. (Banking a score, completing a purchase.)

**Tie-breakers when an op could live on either side:**
- **Latency tolerance** — an L2 → L1 result isn't usable on L1 until the settler
  settles the block (seconds+). If the op can't wait, keep it on L2.
- **Who must verify it** — if the answer is "only the game," L2; if "anyone," L1.
- **Cost vs security** — frequent/cheap leans L2; valuable/contested leans L1.

Then connect the two sides with a message in the right direction: an **L1 action
that should cause an L2 effect** is L1 → L2 (instant); an **L2 result that should
become an L1 fact** is L2 → L1 (settled).

Mapped onto the demo: rolling (`play_game`) is frequent, free, and only the game
cares mid-flight → **L2**. Buying (the `store`, real "payment") and banking a
score (the permanent leaderboard others read) are commit actions others must
trust → **L1**.

## Read path vs write path

Keep these two paths separate in your head; the rest of the guide is organized
around them.

```
        write path (commands)                 read path (queries)
   client ──tx──► system / piltover      chain ──indexed──► Torii ──SQL──► client
        (starknet.js, signed)                 (models + events)     (eventually consistent)
```

- **Writes** are ordinary signed Starknet transactions to your systems (or to
  piltover for L1→L2). Synchronous: you get a tx hash and can await the receipt.
- **Reads** go through **Torii**, not the chain. A write updates a model / emits
  an event; Torii indexes it a moment later; the client polls Torii and re-renders.
  Reads are therefore **eventually consistent** — the client design has to expect
  a short lag (see [client.md](./client.md)).

This separation is why the demo's client can be a thin React app: it *sends*
transactions and *reads* Torii, and never decodes a single contract storage slot.

## Where each concern lives (demo map)

| Concern | Lives in | File |
| --- | --- | --- |
| Appchain state + gameplay | `game` world | `cairo/game/src/lib.cairo` |
| L1 storefront (purchase) | `store` world | `cairo/store/src/lib.cairo` |
| Settlement record | `score` world | `cairo/score/src/lib.cairo` |
| Cross-chain mailbox + settled state | piltover core | deployed by `katana init rollup` |
| Indexing for the client | two Torii instances | started in `up.sh` |
| Client reads/writes | React app | `app/src/chain.ts`, `app/src/App.tsx` |

Next: [why each of those services exists and how it works →](./services.md)
