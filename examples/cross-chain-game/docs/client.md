# Querying appchain state from the client

[← deployment](./deployment.md) · [Guide index](./README.md)

The client's job splits cleanly (see [architecture.md](./architecture.md#read-path-vs-write-path)):
**send transactions** to systems/piltover, **read state** from Torii. The demo's
whole data layer is `app/src/chain.ts`; the poll loop is in `app/src/App.tsx`.

## The read model: Torii, not the chain

The client never decodes contract storage. It reads from Torii, where:

- **Current state** = a model row. `game-Stats` is the live counters.
- **A feed/history** = a per-event table. `game-GamePlayed` is one row per play
  (because the event is keyed by a unique sequence — [contracts.md](./contracts.md#events-as-an-append-log)).

Reads are **eventually consistent**: a write lands on-chain, Torii indexes it a
beat later, the client polls and re-renders. Design for the lag (below).

## Torii SQL over HTTP

Torii serves SQL at `GET /sql?query=<urlencoded>` and returns JSON rows. The
demo's helper:

```ts
async function toriiSql(base, sql) {
  const res = await fetch(`${base}/sql?query=${encodeURIComponent(sql)}`);
  return res.json();                       // array of row objects
}
```
[`app/src/chain.ts:81`](https://github.com/dojoengine/katana/blob/279073a3d4fd6e99ada6ec40bd5c3e1f9bd28bbc/examples/cross-chain-game/app/src/chain.ts#L81)

Two parsing details every client needs:

- **Columns come back as hex strings** (`"0x…41"`). Collapse to numbers:
  `const num = (v) => Number(BigInt(v));` [`app/src/chain.ts:46`](https://github.com/dojoengine/katana/blob/279073a3d4fd6e99ada6ec40bd5c3e1f9bd28bbc/examples/cross-chain-game/app/src/chain.ts#L46)
- **Each event row has `internal_event_id`** = `block:txHash:world:idx`. Split it
  to recover the block height and the tx hash (for explorer links / settlement
  gating): [`app/src/chain.ts:89`](https://github.com/dojoengine/katana/blob/279073a3d4fd6e99ada6ec40bd5c3e1f9bd28bbc/examples/cross-chain-game/app/src/chain.ts#L89)

Reading current state is then a one-liner against the model table:

```ts
const rows = await toriiSql(TORII_GAME, 'SELECT total_minted, available … FROM "game-Stats" WHERE id = 0');
```
[`app/src/chain.ts:117`](https://github.com/dojoengine/katana/blob/279073a3d4fd6e99ada6ec40bd5c3e1f9bd28bbc/examples/cross-chain-game/app/src/chain.ts#L117)

## Joining across two Toriis

An entity's lifecycle can span both chains: in the demo a *play* is a
`GamePlayed` event on the appchain Torii, and its *bank* is a `ScoreClaimed` event
on the settlement Torii. There's no cross-chain query — so the client reads each
feed from its own Torii and **stitches them in JS** (here, matching a play to its
claim by score, FIFO):

```ts
const [played, claimed] = await Promise.all([
  toriiSql(TORII_GAME,  'SELECT game_no, score, internal_event_id FROM "game-GamePlayed" ORDER BY game_no'),
  toriiSql(TORII_SCORE, 'SELECT claim_no, score, internal_event_id FROM "score-ScoreClaimed" ORDER BY claim_no'),
]);
// → each play row gets a claimTxHash if a matching claim exists
```
[`app/src/chain.ts:220`](https://github.com/dojoengine/katana/blob/279073a3d4fd6e99ada6ec40bd5c3e1f9bd28bbc/examples/cross-chain-game/app/src/chain.ts#L220)

This "read both indexers, join on a shared field" pattern is how any multi-chain
appchain app presents one coherent timeline.

## What still comes from RPC

A few things aren't world state, so they bypass Torii and hit the chain directly:

- **Block heights** (appchain tip) and **piltover `get_state`** (settled height) —
  the "settled N / tip M" gauge. [`app/src/chain.ts:281`](https://github.com/dojoengine/katana/blob/279073a3d4fd6e99ada6ec40bd5c3e1f9bd28bbc/examples/cross-chain-game/app/src/chain.ts#L281)
- **The L1 purchase log** (piltover `MessageSent`) via `getEvents`, so a purchase
  shows as *pending* before the appchain relays it — that L1-side event isn't in
  either world. [`app/src/chain.ts:189`](https://github.com/dojoengine/katana/blob/279073a3d4fd6e99ada6ec40bd5c3e1f9bd28bbc/examples/cross-chain-game/app/src/chain.ts#L189)

Rule of thumb: world models/events → Torii; raw chain facts (block numbers,
non-world contract events, settled state) → RPC.

## The write path

Writes are ordinary signed transactions; nothing Torii-specific:

- **L1→L2 (buy):** `storeSystem.buy_game(game_id)` — the L1 store runs its rules, then messages the appchain — [`app/src/chain.ts:145`](https://github.com/dojoengine/katana/blob/279073a3d4fd6e99ada6ec40bd5c3e1f9bd28bbc/examples/cross-chain-game/app/src/chain.ts#L145)
- **L2 (play):** `gameSystem.play_game()` — [`app/src/chain.ts:253`](https://github.com/dojoengine/katana/blob/279073a3d4fd6e99ada6ec40bd5c3e1f9bd28bbc/examples/cross-chain-game/app/src/chain.ts#L253)
- **L2→L1 (bank):** `scoreSystem.claim_score(game_system, player, score)` — [`app/src/chain.ts:292`](https://github.com/dojoengine/katana/blob/279073a3d4fd6e99ada6ec40bd5c3e1f9bd28bbc/examples/cross-chain-game/app/src/chain.ts#L292)

Two practical notes from the demo:

- **Serialize same-account writes.** Purchases and claims are both signed by the
  settlement account; firing them concurrently races the nonce. The demo funnels
  them through a promise-chain mutex (`withSettlementLock`).
- **A write returns before the read updates.** `playGame()` sends the tx, waits
  for the receipt, then **polls Torii** until the play is indexed before returning
  the score — bridging the write path to the eventually-consistent read path.
  [`app/src/chain.ts:253`](https://github.com/dojoengine/katana/blob/279073a3d4fd6e99ada6ec40bd5c3e1f9bd28bbc/examples/cross-chain-game/app/src/chain.ts#L253)

## Tying it to the UI: poll + derive

The client keeps no authoritative state of its own. One interval re-reads
everything from Torii (and the few RPC facts) and the UI derives from that:

```ts
const [g, sc, ph, plh, sb, tp] = await Promise.all([
  readGameState(), readScoreState(), getPurchaseHistory(),
  getPlayHistory(), settledBlock(), appchainBlock(),
]);
// …setState; const h = setInterval(tick, 1500)
```
[`app/src/App.tsx:90`](https://github.com/dojoengine/katana/blob/279073a3d4fd6e99ada6ec40bd5c3e1f9bd28bbc/examples/cross-chain-game/app/src/App.tsx#L90)

Everything shown is then derived from those reads — e.g. unbanked vs banked runs
are just a filter over the joined play list:

```ts
const unbanked = plays.filter((p) => !p.claimTxHash);   // still on L2
const banked   = plays.filter((p) =>  p.claimTxHash);    // settled to L1
```
[`app/src/App.tsx:138`](https://github.com/dojoengine/katana/blob/279073a3d4fd6e99ada6ec40bd5c3e1f9bd28bbc/examples/cross-chain-game/app/src/App.tsx#L138)

Because the feeds are rebuilt from Torii every tick, the UI is **refresh-proof**:
reload the page and the same state reappears, since it was never only in React.

---

That's the whole loop: [architecture](./architecture.md) (worlds + two chains) →
[services](./services.md) (Katana, piltover, saya, Torii) →
[contracts](./contracts.md) (both messaging directions) →
[deployment](./deployment.md) (migrate + orchestrate) → this read/write client.
