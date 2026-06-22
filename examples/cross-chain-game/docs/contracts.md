# Contract architecture (Dojo + cross-chain)

[← services](./services.md) · Next: [deployment →](./deployment.md)

This chapter shows how to write the contracts: the Dojo building blocks, then
each cross-chain direction, then the wiring and the gotchas. Snippets are from the
demo's worlds — `cairo/game/src/lib.cairo` (appchain) and
`cairo/score/src/lib.cairo` + `cairo/store/src/lib.cairo` (settlement).

## Dojo building blocks

**A model** is a keyed struct = one table. Use a singleton key (`id: u8 = 0`) for
global state, or a real key (a player address) for per-entity rows.

```cairo
#[derive(Copy, Drop, Serde)]
#[dojo::model]
pub struct Stats { #[key] pub id: u8, pub total_minted: u64, pub available: u64, /* … */ }
```
[`cairo/game/src/lib.cairo:35`](https://github.com/dojoengine/katana/blob/2e36ba5ae08b2f7c07e6e6a458464995e1d59a25/examples/cross-chain-game/cairo/game/src/lib.cairo#L35)

**A system** is a `#[dojo::contract]`. It reaches the world via `self.world(@"ns")`
and reads/writes models:

```cairo
let mut world = self.world_default();          // self.world(@"game")
let mut stats: Stats = world.read_model(SINGLETON);
stats.available -= 1;
world.write_model(@stats);
```
[`cairo/game/src/lib.cairo:114`](https://github.com/dojoengine/katana/blob/2e36ba5ae08b2f7c07e6e6a458464995e1d59a25/examples/cross-chain-game/cairo/game/src/lib.cairo#L114)

**Config goes in `dojo_init`** — it runs once at migration. Pass addresses you
only learn at deploy time (e.g. the contract on the *other* chain) here rather
than hard-coding them:

```cairo
fn dojo_init(self: @ContractState, registry: ContractAddress) {
    let mut world = self.world_default();
    world.write_model(@GameConfig { id: SINGLETON, registry });
}
```
[`cairo/game/src/lib.cairo:80`](https://github.com/dojoengine/katana/blob/2e36ba5ae08b2f7c07e6e6a458464995e1d59a25/examples/cross-chain-game/cairo/game/src/lib.cairo#L80)

**Permissions:** a system may only write models in namespaces it's a *writer* of;
that grant is declared at migration (see [deployment.md](./deployment.md)).

## L1 → L2: receive a message in an `#[l1_handler]`

The instant direction. piltover emits a message on L1; the appchain
(`--messaging.enabled`) relays it as an `L1HandlerTx` that calls the handler whose
**selector matches** the one in the message. The first parameter is always the L1
sender, injected by the relayer; the rest is the payload.

A useful fact the demo relies on: **the `#[dojo::contract]` macro passes
`#[l1_handler]` through unchanged**, so your handler can live inside a system and
use the world like any other function:

```cairo
#[l1_handler]
fn mint_game(ref self: ContractState, from_address: felt252, game_id: felt252) {
    let mut world = self.world_default();
    let mut stats: Stats = world.read_model(SINGLETON);
    stats.total_minted += 1; stats.available += 1;
    world.write_model(@stats);
    world.emit_event(@GameMinted { mint_no: stats.total_minted, buyer: from_address, /* … */ });
}
```
[`cairo/game/src/lib.cairo:93`](https://github.com/dojoengine/katana/blob/2e36ba5ae08b2f7c07e6e6a458464995e1d59a25/examples/cross-chain-game/cairo/game/src/lib.cairo#L93)

In the demo the client doesn't call piltover directly — the player calls
`buy_game` on the L1 `store` contract, which runs the store's rules and then makes
this `send_message_to_appchain` call. So `mint_game`'s `from_address` on L2 is the
store contract (provenance L2 can trust), not the buyer. See
[deciding what goes where](./architecture.md#deciding-what-goes-where) for why a
purchase belongs on L1.

## L2 → L1: send on the appchain, consume on the settlement layer

The settled direction, two halves:

**1. Send from an appchain system** with the raw syscall. `to_address` is the L1
contract that will consume it; the payload is yours to define:

```cairo
send_message_to_l1_syscall(config.registry.into(), array![player, score.into()].span())
    .unwrap_syscall();
```
[`cairo/game/src/lib.cairo:131`](https://github.com/dojoengine/katana/blob/2e36ba5ae08b2f7c07e6e6a458464995e1d59a25/examples/cross-chain-game/cairo/game/src/lib.cairo#L131)

**2. Consume on a settlement system.** It calls the piltover core; this reverts
until the settler has settled the block containing the message (that's the whole point —
[services.md](./services.md)):

```cairo
let piltover = IPiltoverMessagingDispatcher { contract_address: config.piltover };
piltover.consume_message_from_appchain(from_address, array![player, score].span());
```
[`cairo/score/src/lib.cairo:99`](https://github.com/dojoengine/katana/blob/2e36ba5ae08b2f7c07e6e6a458464995e1d59a25/examples/cross-chain-game/cairo/score/src/lib.cairo#L99)

**The payload contract is sacred.** The `(from_address, payload)` the consumer
passes must reconstruct exactly what the sender emitted, or the hash won't match
and the consume reverts. Here both sides agree on `[player, score]`, and
`from_address` is the appchain system's address. Define this payload shape once
and keep both ends in lockstep.

## Wiring the worlds together

Each side needs the other's address, but they're deployed on different chains at
different times. Resolve it by **ordering the deploys** and passing the dependency
through `dojo_init`:

1. Deploy the **settlement** world first → learn its system address.
2. Deploy the **appchain** world, passing that address as the `registry` init arg
   so `send_message_to_l1` knows where to send.

The reverse dependency (the settlement consumer needs the appchain sender's
address) is supplied by the **client at call time** as `from_address`, so there's
no circular deploy. The `store` world is wired the same way — it takes the game
system address at init so `buy_game` knows where to send. [deployment.md](./deployment.md)
shows the full deploy order.

## Events as an append log

Dojo events are stored like models — **keyed, and upserted by key**. If you key an
event by something non-unique (a player address), every emission overwrites the
same row and your "feed" collapses to one entry. To get an append log, **key by a
monotonic sequence**:

```cairo
#[dojo::event]
pub struct GamePlayed { #[key] pub game_no: u64, pub player: felt252, pub score: u64 }
```
[`cairo/game/src/lib.cairo:71`](https://github.com/dojoengine/katana/blob/2e36ba5ae08b2f7c07e6e6a458464995e1d59a25/examples/cross-chain-game/cairo/game/src/lib.cairo#L71)

The demo keys `GameMinted`/`GamePlayed`/`ScoreClaimed` by `mint_no`/`game_no`/
`claim_no` so Torii keeps one row per mint/play/bank — which is exactly what the
client's feeds need ([client.md](./client.md)).

## The message-hash gotcha

A Starknet-settled appchain hashes **L1→L2** messages with **Poseidon**; Ethereum
L1s use keccak. Whatever settles the appchain must register the L1→L2 message hash
the way piltover expects, or **every block that consumes an L1→L2 message (i.e.
every purchase) stalls in settlement.** Katana's embedded settlement service hashes
with Poseidon, so this matches out of the box. (Historically the demo ran the
external `saya-tee` sidecar; saya 0.4.0 shipped the keccak formula and needed a
patch to fix it — no longer relevant now that Katana settles.) If you build your
own appchain, verify your prover hashes L1→L2 messages the way your settlement
chain expects.

Next: [build the worlds and bring up the stack →](./deployment.md)
