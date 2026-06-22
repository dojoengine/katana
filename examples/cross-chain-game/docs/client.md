# Querying appchain state from the client

[← deployment](./deployment.md) · [Guide index](./README.md)

The client's job splits cleanly (see [architecture.md](./architecture.md#read-path-vs-write-path)):
**send transactions** to systems/piltover, **read state** from Torii. The demo's
whole data layer is `app/src/chain.ts`; the subscribe-and-refetch loop is in
`app/src/App.tsx`.

## The read model: Torii, not the chain

The client never decodes contract storage. It reads from Torii, where:

- **Current state** = a model row. `game-Stats` is the live counters.
- **A feed/history** = a per-event table. `game-GamePlayed` is one row per play
  (because the event is keyed by a unique sequence — [contracts.md](./contracts.md#events-as-an-append-log)).

Reads are **eventually consistent**: a write lands on-chain, Torii indexes it a
beat later, Torii pushes a subscription update, the client refetches and
re-renders. Design for the lag (below).

## Torii SQL over HTTP

Torii serves SQL at `GET /sql?query=<urlencoded>` and returns JSON rows. The
demo's helper:

```ts
async function toriiSql(base, sql) {
  const res = await fetch(`${base}/sql?query=${encodeURIComponent(sql)}`);
  return res.json();                       // array of row objects
}
```
[`app/src/chain.ts:81`](https://github.com/dojoengine/katana/blob/2e36ba5ae08b2f7c07e6e6a458464995e1d59a25/examples/cross-chain-game/app/src/chain.ts#L81)

Two parsing details every client needs:

- **Columns come back as hex strings** (`"0x…41"`). Collapse to numbers:
  `const num = (v) => Number(BigInt(v));` [`app/src/chain.ts:46`](https://github.com/dojoengine/katana/blob/2e36ba5ae08b2f7c07e6e6a458464995e1d59a25/examples/cross-chain-game/app/src/chain.ts#L46)
- **Each event row has `internal_event_id`** = `block:txHash:world:idx`. Split it
  to recover the block height and the tx hash (for explorer links / settlement
  gating): [`app/src/chain.ts:89`](https://github.com/dojoengine/katana/blob/2e36ba5ae08b2f7c07e6e6a458464995e1d59a25/examples/cross-chain-game/app/src/chain.ts#L89)

Reading current state is then a one-liner against the model table:

```ts
const rows = await toriiSql(TORII_GAME, 'SELECT total_minted, available … FROM "game-Stats" WHERE id = 0');
```
[`app/src/chain.ts:117`](https://github.com/dojoengine/katana/blob/2e36ba5ae08b2f7c07e6e6a458464995e1d59a25/examples/cross-chain-game/app/src/chain.ts#L117)

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
[`app/src/chain.ts:220`](https://github.com/dojoengine/katana/blob/2e36ba5ae08b2f7c07e6e6a458464995e1d59a25/examples/cross-chain-game/app/src/chain.ts#L220)

This "read both indexers, join on a shared field" pattern is how any multi-chain
appchain app presents one coherent timeline.

## What still comes from RPC

A few things aren't world state, so they bypass Torii and hit the chain directly:

- **Block heights** (appchain tip) and **piltover `get_state`** (settled height) —
  the "settled N / tip M" gauge. [`app/src/chain.ts:281`](https://github.com/dojoengine/katana/blob/2e36ba5ae08b2f7c07e6e6a458464995e1d59a25/examples/cross-chain-game/app/src/chain.ts#L281)
- **The L1 purchase log** (piltover `MessageSent`) via `getEvents`, so a purchase
  shows as *pending* before the appchain relays it — that L1-side event isn't in
  either world. [`app/src/chain.ts:189`](https://github.com/dojoengine/katana/blob/2e36ba5ae08b2f7c07e6e6a458464995e1d59a25/examples/cross-chain-game/app/src/chain.ts#L189)

Rule of thumb: world models/events → Torii; raw chain facts (block numbers,
non-world contract events, settled state) → RPC.

## The write path

Writes are ordinary signed transactions; nothing Torii-specific:

- **L1→L2 (buy):** `storeSystem.buy_game(game_id)` — the L1 store runs its rules, then messages the appchain — [`app/src/chain.ts:145`](https://github.com/dojoengine/katana/blob/2e36ba5ae08b2f7c07e6e6a458464995e1d59a25/examples/cross-chain-game/app/src/chain.ts#L145)
- **L2 (play):** `gameSystem.play_game()` — [`app/src/chain.ts:253`](https://github.com/dojoengine/katana/blob/2e36ba5ae08b2f7c07e6e6a458464995e1d59a25/examples/cross-chain-game/app/src/chain.ts#L253)
- **L2→L1 (bank):** `scoreSystem.claim_score(game_system, player, score)` — [`app/src/chain.ts:292`](https://github.com/dojoengine/katana/blob/2e36ba5ae08b2f7c07e6e6a458464995e1d59a25/examples/cross-chain-game/app/src/chain.ts#L292)

Two practical notes from the demo:

- **Serialize same-account writes.** Purchases and claims are both signed by the
  settlement account; firing them concurrently races the nonce. The demo funnels
  them through a promise-chain mutex (`withSettlementLock`).
- **A write returns before the read updates.** `playGame()` sends the tx, waits
  for the receipt, then **polls Torii** until the play is indexed before returning
  the score — bridging the write path to the eventually-consistent read path.
  [`app/src/chain.ts:253`](https://github.com/dojoengine/katana/blob/2e36ba5ae08b2f7c07e6e6a458464995e1d59a25/examples/cross-chain-game/app/src/chain.ts#L253)

## Wallets (optional Controller)

By default the client signs with the demo's hardcoded **dev accounts**. The header
**Login** button can swap to a [Cartridge Controller](https://github.com/cartridge-gg/controller)
— a hosted-keychain wallet wired via `@cartridge/connector` + `@starknet-react/core`
in `app/src/wallet.tsx`, with the active account injected into `purchaseGame`,
`claimScore`, and `playGame`. The **same** Controller signs on **both chains**: buy
+ bank on L1 and the roll on the appchain, at the **same address** (a Controller is
UDC-deployed deterministically from its username, so the address is
chain-independent). For the roll the connector switches the Controller to the
appchain (chain id `GAMECHAIN`) and executes `play_game`; the L1 signer switches
back to settlement on its next call, so the roll itself deliberately does **not**
switch back.

> **Two Controller gotchas the wallet handles** (`app/src/wallet.tsx`):
>
> - **Don't switch back to settlement after the roll.** The keychain's balance-change
>   preview re-simulates on whatever chain is currently active, so flipping back to Sepolia
>   makes it re-run the appchain call there (where the game contract doesn't exist) and
>   surface a bogus "not deployed" error. Because the L1 signer switches to settlement
>   itself, leaving the Controller on the appchain is correct.
> - **The switch can fail.** `switchStarknetChain` returns `false` when the hosted keychain
>   can't reach the appchain RPC — e.g. the `x.cartridge.gg` iframe blocked from
>   `http://localhost` by Chrome's Private Network Access. The wallet throws a clear error
>   instead of running the tx on Sepolia; enable `chrome://flags/#local-network-access-check`,
>   or self-host the keychain via `VITE_KEYCHAIN_URL`.

Controller is opt-in: the stack must be started with `CONTROLLER=1 ./up.sh` — that
makes **both** nodes Controller-capable (paymaster; `katana init rollup` declares
the Controller classes in the appchain genesis by default) and serves the app over
trusted HTTPS for the passkey login. See the
demo's [README](../README.md) → "Using Controller". Because the same Controller is
now the caller on the appchain, `play_game`'s `get_caller_address()`, the L2→L1 score
message, and the leaderboard `player` all key on the Controller.

> **Caveat:** `CONTROLLER=1 ./up.sh` now drives both chains via the **hosted keychain
> at `x.cartridge.gg`** with no manual steps — it even declares the controller class on
> the appchain at boot (a katana #584 workaround; see
> [Current known blockers](#current-known-blockers-appchain-controller) below). The L1
> side (buy/bank) and the default dev-account path are unaffected.

## Tying it to the UI: subscribe + derive

The client keeps no authoritative state of its own. A single `tick()` re-reads
everything from Torii (and the few RPC facts) and the UI derives from that. Rather
than run `tick()` on a fixed interval, the client **subscribes** to Torii and
refetches only when there's new data:

```ts
const cleanup = await subscribeToriiUpdates(ping); // ping = debounced tick()
const slow = setInterval(tick, 4000);              // safety net + RPC-only reads
```
[`app/src/App.tsx:135`](https://github.com/dojoengine/katana/blob/2e36ba5ae08b2f7c07e6e6a458464995e1d59a25/examples/cross-chain-game/app/src/App.tsx#L135)

`subscribeToriiUpdates` opens a gRPC subscription (`@dojoengine/torii-wasm`'s
`ToriiClient`) on **both** worlds — `onEntityUpdated` for models (`game-Stats`,
`score-Leaderboard`) and `onEventMessageUpdated` for the emitted feeds
(`GameMinted` / `GamePlayed` / `ScoreClaimed`). Any push triggers a debounced
refetch, so a roll or bank shows up the moment Torii indexes it instead of up to
an interval later:

```ts
subs.push(await client.onEntityUpdated(null, null, () => onUpdate()));
subs.push(await client.onEventMessageUpdated(null, null, () => onUpdate()));
```
[`app/src/chain.ts:107`](https://github.com/dojoengine/katana/blob/2e36ba5ae08b2f7c07e6e6a458464995e1d59a25/examples/cross-chain-game/app/src/chain.ts#L107)

The slow interval stays as a safety net and to refresh the few facts Torii can't
push: the settled block height and the appchain tip are read straight from RPC, and
the piltover `MessageSent` purchase log is RPC too (so a buy also nudges a refetch
explicitly). Everything shown is then derived from the reads — e.g. unbanked vs
banked runs are just a filter over the joined play list:

```ts
const unbanked = plays.filter((p) => !p.claimTxHash);   // still on L2
const banked   = plays.filter((p) =>  p.claimTxHash);    // settled to L1
```
[`app/src/App.tsx:170`](https://github.com/dojoengine/katana/blob/2e36ba5ae08b2f7c07e6e6a458464995e1d59a25/examples/cross-chain-game/app/src/App.tsx#L170)

## Current known blockers (appchain Controller)

Driving a **Cartridge Controller against the appchain** works end-to-end with the
**hosted keychain at `x.cartridge.gg`** — the same Controller address signs buy/bank
on L1 and the roll on `GAMECHAIN`. Getting there tripped a series of the keychain's
assumptions: it's built for Cartridge-known chains (mainnet, sepolia, slot), not a
bespoke `katana init rollup` appchain. **All but one are now fixed at the source**;
what remains is a single runtime step, and the **default dev-account path is
unaffected**.

### Resolved

- **Fee-token address mismatch.** The keychain and its prebuilt WASM hardcode
  **canonical STRK** (`0x04718f5a…`, the V3/FRI fee token) and call `balanceOf` /
  metadata on it. A default rollup put its fee token at a derived address and didn't
  host canonical STRK at all, so those calls 404'd — `Contract not found` in the fee
  step, and a red "Simulation Error" in the balance preview. **Fixed in katana:** the
  rollup genesis now pre-allocates STRK at the canonical mainnet address
  (`crates/chain-spec/src/fee_token.rs`), so the keychain resolves it.
- **Stale fee error + false "Simulation Error" on the Review screen.** A chain switch
  left a stale fee estimate from the previous chain, and the balance preview
  simulated a tx from the **not-yet-deployed** Controller (sender 404). **Fixed
  upstream** (`cartridge-gg/controller#2609`): re-estimate fees when the chain
  changes, and skip the balance preview when the account isn't deployed yet.
- **Hosted keychain didn't re-point on chain switch.** A chain switch didn't rebuild
  the keychain's controller for the new RPC, so the post-switch roll hit the old
  chain. The rebuild (`switchChain.ts` → `Controller.create({rpcUrl})`) landed in
  `controller` `main`, and **`x.cartridge.gg` redeployed from main right after #2609**
  (verified: the deployed bundle carries the #2609 `use-simulate` fix). So the
  **hosted keychain now drives the local appchain directly** — no self-hosting needed.
- **API CORS.** Only an issue for a *localhost* keychain (`api.cartridge.gg` restricts
  CORS to Cartridge origins). The hosted keychain is same-origin with the API, so it's
  moot; the self-hosted fork and its proxy are now just a fallback.

### Handled for you (a katana #584 workaround in `up.sh`)

- **Controller class hash (the #584 gap).** `katana init rollup`'s genesis JSON
  round-trip shifts the embedded controller class hash. #584 declares the preloaded
  genesis classes via a real declare tx, but it declares the **round-tripped**
  artifact — which hashes to a *shifted* value, not the **canonical** `0x743c8…` the
  keychain deploys (#584's test passes only because it asserts `ControllerLatest::HASH`,
  that same shifted constant). So the class lands on-chain at the wrong hash. **`up.sh`
  works around it**: in CONTROLLER mode it declares the original on-disk
  `controller.latest` after boot (`scripts/declare-controller-class.ts`) to land
  `0x743c8`, so the Controller can auto-deploy on the appchain. Worth fixing properly
  in katana.

Two operational notes (not blockers):

- **Chrome Private Network Access.** The hosted `x.cartridge.gg` iframe reaches the
  appchain at `localhost:5051`; if a connect/roll stalls, enable
  `chrome://flags/#local-network-access-check`.
- **Sessions are per-chain.** The session is approved on the settlement chain at login;
  the appchain has none (the demo's policies are unverified, so the keychain won't
  silently auto-create one), so the roll shows a manual confirm modal rather than being
  silent like buy/bank. Just an extra click — with #2609 it no longer shows a false error.

The boot-time declare lives in
[`scripts/declare-controller-class.ts`](../scripts/declare-controller-class.ts); the
(now-fallback) self-hosted keychain is in [`keychain-fork/`](../keychain-fork/). The
**default dev-account path is unaffected**.

Because the feeds are rebuilt from Torii on each refetch, the UI is
**refresh-proof**: reload the page and the same state reappears, since it was never
only in React.

---

That's the whole loop: [architecture](./architecture.md) (worlds + two chains) →
[services](./services.md) (Katana, piltover, the settler, Torii) →
[contracts](./contracts.md) (both messaging directions) →
[deployment](./deployment.md) (migrate + orchestrate) → this read/write client.
