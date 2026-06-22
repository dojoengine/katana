# The services: why, where, how

[← architecture](./architecture.md) · Next: [contracts →](./contracts.md)

An appchain app is a small distributed system. This chapter takes each process in
turn and answers: **what it is, why the architecture needs it, where it sits, and
how it works.** None of it is specific to the demo game — every app on a Katana
appchain runs this same set.

```
   ┌─ Settlement Katana ─────────────┐         ┌─ Appchain Katana ──────────────┐
   │  piltover core   settlement world│  L1→L2  │  appchain world                │
   │       ▲                ▲         │ ◄─────► │       ▲                        │
   └───────│────────────────│─────────┘  L2→L1  └───────│────────────────────────┘
           │ update_state   │ index            relay │  │ index
        ┌──┴──┐          ┌──┴───┐                     │ ┌┴─────┐
        │ tee │          │Torii │◄──── client ────────┘ │Torii │
        └─────┘          └──────┘   reads/writes        └──────┘
```

## Katana — the sequencer (you run two)

**What.** Katana is the Starknet sequencer: it accepts transactions, executes
them, and produces blocks. It's also the Dojo dev chain, so it hosts worlds.

**Why two.** The architecture has two roles — a chain to settle *to* and the
chain your app runs *on* — and each is a Katana instance:

- **Settlement Katana ("L1").** Stands in for the chain you anchor to (Starknet
  mainnet/Sepolia in production). It hosts the **piltover core** and your
  settlement world. In the demo it runs with `--chain-id SN_SEPOLIA` so the
  embedded settlement service and `katana init rollup` agree on the chain id.
  [`up.sh:91`](https://github.com/dojoengine/katana/blob/2e36ba5ae08b2f7c07e6e6a458464995e1d59a25/examples/cross-chain-game/up.sh#L91)
- **Appchain Katana ("L2").** Your app's chain, started as a **rollup** that
  settles to the piltover core. Key flags:
  - `--tee mock` — run as a TEE-settled rollup (mock attestation locally).
  - `--messaging.enabled` — watch the settlement chain and **relay L1→L2
    messages** as `L1HandlerTx`. Without this, purchases never reach L2.
  - `--dev --dev.no-fee` — fees off (an empty rollup can't price gas sanely).
  - `--block-time 5000` — mine a block every 5s (interval mining) instead of
    per-transaction, so the chain advances on a steady cadence and the settler
    keeps settling even when the app is idle (trades instant inclusion for a predictable
    block time).
  ```bash
  katana --chain "$CHAIN_DIR" --tee mock --dev --dev.no-fee --block-time 5000 \
         --http.port 5051 --explorer --messaging.enabled
  ```
  [`up.sh:147`](https://github.com/dojoengine/katana/blob/2e36ba5ae08b2f7c07e6e6a458464995e1d59a25/examples/cross-chain-game/up.sh#L147)

**How they connect.** The appchain is created by `katana init rollup`, which
deploys the piltover core on the settlement chain and writes a chain config the
appchain boots from. From then on the appchain knows where to settle, and (with
`--messaging.enabled`) which contract to watch for inbound messages.

## piltover core — the cross-chain mailbox

**What.** A contract **on the settlement chain** that is the appchain's interface
on L1: the message mailbox in both directions plus the settled state root.

**Why.** Cross-chain messaging needs a contract on L1 that (a) records outbound
L1→L2 messages for the appchain to pick up, and (b) holds the set of L2→L1
messages that have been *settled*, so L1 contracts can consume them safely. That
contract is piltover.

**Where / how.** Deployed by `katana init rollup --tee` on the settlement chain
([`up.sh:109`](https://github.com/dojoengine/katana/blob/2e36ba5ae08b2f7c07e6e6a458464995e1d59a25/examples/cross-chain-game/up.sh#L109)). Its interface, as used in this guide:

- `send_message_to_appchain(to, selector, payload)` — **L1 → L2**. Emits
  `MessageSent`; the appchain relays it. (In the demo, called by the L1 `store`
  contract's `buy_game` to start a round trip.)
- `consume_message_from_appchain(from, payload)` — **L2 → L1**. Succeeds only if
  the settler has settled a block containing that message. (Called by your
  settlement system.)
- `get_state()` — returns the latest settled block height; the demo reads it to
  show the "settled N / tip M" gauge.

## The settler — what makes L2 → L1 possible

**What.** Katana's **embedded settlement service** (the *settler*) runs inside the
appchain node itself — no separate process. It watches the appchain, **proves each
block** (`--tee mock` locally), and submits an `update_state` transaction to the
piltover core.

**Why it's the linchpin of L2→L1.** When an appchain system calls
`send_message_to_l1`, that only emits intent on L2 — L1 doesn't know about it yet.
The message becomes consumable on L1 **only after** its block is settled:
`update_state` registers the block's outbound message hashes in piltover. So:

> Without settlement ⇒ `consume_message_from_appchain` always reverts ⇒ nothing
> ever banks back to L1.

This is why the demo's *bank* step can't be instant: the client has to wait for
the settler to settle the block the play landed in before claiming.

**Where / how.** The settler isn't a sidecar — it's turned on by adding a
`[settlement.runtime]` section to the chain config (`katana init rollup` writes
only the settlement *layer*; the operator adds the runtime) and running the
appchain node with `--tee mock` ([`up.sh:163`](https://github.com/dojoengine/katana/blob/2e36ba5ae08b2f7c07e6e6a458464995e1d59a25/examples/cross-chain-game/up.sh#L163)):

```toml
[settlement.runtime]
account-address = "$SETTLER_ADDR"     # the operator account that submits update_state
account-private-key = "$SETTLER_PK"
tee-registry = "$TEE_REGISTRY"        # on-L1 attestation verifier (mock locally)
batch-size = 1
```

- `--tee mock` — exercises the settlement *plumbing* without a real SP1/TEE
  prover. It proves the messaging path works, not proof soundness.
- The **mock TEE registry** (declared + deployed by
  [`scripts/bootstrap/deploy-tee-registry-mock.sh`](../../../scripts/bootstrap/deploy-tee-registry-mock.sh),
  run from `up.sh`) is the on-L1 attestation verifier; the mock accepts the
  settlement service's attestation so `update_state` is allowed. In production this
  is a real attestation registry.

> **Gotcha (see [contracts.md](./contracts.md#the-message-hash-gotcha)):** a
> Starknet-settled appchain must hash L1→L2 messages with Poseidon, not Ethereum
> keccak. Katana's embedded settlement service hashes with Poseidon, so the demo
> handles this out of the box (saya 0.4.0 shipped keccak and needed a patch — no
> longer used). Wrong hash ⇒ blocks that consume an L1→L2 message never settle.

## Torii — the indexer the client reads

**What.** Torii indexes a Dojo world into a SQLite database and serves it over
HTTP (SQL + GraphQL) and gRPC.

**Why.** Model data lives in the world contract's storage, keyed and packed —
awkward to read over raw RPC, and there's no "list all events of type X" RPC. A
client needs *queryable* state. Torii subscribes to the chain, decodes every
model write into a row and every Dojo event into an event-table row, and exposes
them. The client then reads plain JSON.

**Why one per chain.** A Torii instance indexes **one RPC / one world**. The demo
indexes the world the UI reads on each chain — `score` on the settlement layer,
`game` on the appchain — so there are **two Torii instances** (the L1 `store`
world isn't indexed; its purchases are read from the piltover log over RPC):

```bash
torii --rpc http://localhost:5050 --world "$SCORE_WORLD" --http.port 8081 …  # settlement
torii --rpc http://localhost:5051 --world "$GAME_WORLD"  --http.port 8082 …  # appchain
```
[`up.sh:185`](https://github.com/dojoengine/katana/blob/2e36ba5ae08b2f7c07e6e6a458464995e1d59a25/examples/cross-chain-game/up.sh#L185), [`up.sh:194`](https://github.com/dojoengine/katana/blob/2e36ba5ae08b2f7c07e6e6a458464995e1d59a25/examples/cross-chain-game/up.sh#L194)

**How the client uses it.** Current state from model tables (`game-Stats`), feeds
from per-event tables (`game-GamePlayed`), via `GET /sql?query=…`. The client even
*joins across the two Torii instances* to follow an entity whose lifecycle spans
both chains — see [client.md](./client.md). Relay (libp2p) ports are set per
instance so two Toriis don't collide (`up.sh` `--relay.port …`).

## Putting it together: who triggers whom

| Step | Actor | Touches |
| --- | --- | --- |
| Send L1→L2 | client → `store` → piltover (settlement Katana) | piltover emits `MessageSent` |
| Relay | appchain Katana (`--messaging.enabled`) | runs your `#[l1_handler]` |
| Act on L2 | client → appchain system | writes model, maybe `send_message_to_l1` |
| Settle | embedded settler → piltover | registers L2→L1 message hashes |
| Consume L1 | client → settlement system → piltover | `consume_message_from_appchain` |
| Read | client → Torii ×2 | model rows + event feeds |

Next: [how the contracts implement both messaging directions →](./contracts.md)
