# Interval mining & pre-confirmed play

[← client](./client.md) · [guide index](./README.md)

The appchain mines on a **5-second interval** (`--block-time 5000`) and **persists
its state to disk** (`--data-dir`). Both are deliberate, and both change the timing
model in a way that would break an interactive UI — unless the client and Torii read
and write against the **pre-confirmed** block instead of the last mined one. This
chapter is the why behind those flags and the four pre-confirmed adjustments they
require.

## Why 5s blocks + a persistent db

Default `--dev` mining seals a block **per transaction**, instantly. That's snappy,
but it has two costs here: every click becomes its own appchain block that **saya
settles to Sepolia** (a settlement tx per action, in bursts), and the chain is
**in-memory** — a restart wipes the game world and every run.

- **`--block-time 5000`** mines on a steady 5s cadence and batches a window of
  actions into one block, so saya settles at a predictable rate (and the "settled N
  / tip M" gauge ticks naturally) instead of bursting a block per click.
- **`--data-dir .run/appchain-db`** persists appchain state, so you can bounce the
  node — e.g. to change the block time — and keep the game world, runs, and vault.
  No redeploy. (Caveat: a *fresh* appchain db must pair with a *fresh* piltover, so
  saya's start block lines up — that's what `FRESH=1 ./up.sh` does. Reusing an old
  piltover under a fresh chain leaves saya waiting for a block height that no longer
  exists. See [services.md](./services.md#saya--the-prover-now-settling-to-a-real-chain).)

## The catch: `latest` lags `pre_confirmed`

Under interval mining the last *mined* block (`latest`) trails the in-progress
**pre-confirmed** block by up to one interval. A submitted tx — and its state writes
**and its nonce bump** — are live in the pre-confirmed block immediately, but don't
reach `latest` until the block seals up to 5s later. Anything that reads `latest` is
reading up-to-5s-stale truth. Katana (RPC 0.10) exposes the pre-confirmed block
directly, so the fix in every case is the same: target `pre_confirmed`, not `latest`.

Four things read `latest` by default; all four move to `pre_confirmed`.

| # | Symptom (with 5s blocks) | Reads `latest` | Fix |
| --- | --- | --- | --- |
| 1 | Each action takes ~2.5s before the UI frees up | `waitForTransaction` waits for the *mined* receipt | resolve on `PRE_CONFIRMED` |
| 2 | The board (HP/room/gold) lags to the tick | Torii indexes only mined blocks | `torii --indexing.preconfirmed` |
| 3 | Quick consecutive actions fail `Invalid transaction nonce` | starknet.js reads the latest (stale) nonce | serialize + read nonce from `pre_confirmed` |
| 4 | **Loot right after moving** reverts `'No treasure here'` | fee estimate simulates against latest state | estimate against `pre_confirmed` |

### 1 — Latency: wait for pre-confirmed, not the mine

`waitForTransaction` defaults to `ACCEPTED_ON_L2` (mined into a block) — up to 5s
away. The appchain is a local, trusted rollup, so play actions resolve on
`PRE_CONFIRMED` instead (`APPCHAIN_TX_WAIT.successStates` in `chain.ts`): the tx has
executed and its writes are live in the pre-confirmed block well before the seal.
Measured ~335ms vs ~2500ms.

### 2 — Stale reads: index the pre-confirmed block

The dungeon view reads from Torii. By default Torii indexes only mined blocks, so
even after the action button frees up, the board would wait for the 5s tick. The
**appchain** Torii runs with **`--indexing.preconfirmed`** (`up.sh`) so model and
event writes appear the instant they're in the pre-confirmed block. (The Sepolia
bank Torii doesn't need it — settlement is paced by real L1 blocks.)

### 3 & 4 — Nonces and fee estimates: pin both to pre-confirmed

Every play action is signed by the **one** appchain dev account, and starknet.js
reads *both* the nonce and the fee estimate against `latest` by default:

- **Nonce.** Two actions fired before the block mines both grab the stale latest
  nonce; the second is rejected. Fix: a promise-chain mutex (`withAppchainLock`, the
  same idiom as `withSettlementLock`) serializes them so fetch+submit is atomic, and
  each reads its nonce from `pre_confirmed` so it sees the previous action's bump.
- **Estimate.** The fee estimate *simulates* the call against `latest` state. The
  sharp edge is **loot immediately after moving into a treasure room**: the move
  lives only in the pre-confirmed block, Torii (now pre-confirmed-indexed) shows the
  treasure so the Loot button enables — but loot's estimate runs against the
  pre-move `latest` state where the room isn't treasure yet, and reverts. Fix:
  estimate against `pre_confirmed` and pass the bounds to `execute`. (Attack/move
  never hit this — they're valid in both the old and new state.)

Together, the appchain write path (`appchainCall` in `chain.ts`) looks like:

```ts
withAppchainLock(async () => {
  const call = { contractAddress: GAME_SYSTEM, entrypoint, calldata: [arg] };
  // pre_confirmed, not latest: nonce + fee estimate both see the pending block
  const nonce = await appchainProvider.getNonceForAddress(addr, BlockTag.PRE_CONFIRMED);
  const { resourceBounds } = await appchainAccount.estimateInvokeFee(call, {
    blockIdentifier: BlockTag.PRE_CONFIRMED,
    nonce,
  });
  const { transaction_hash } = await appchainAccount.execute(call, { nonce, resourceBounds });
  await appchainProvider.waitForTransaction(transaction_hash, APPCHAIN_TX_WAIT); // PRE_CONFIRMED
});
```

> RPC note: katana's RPC 0.10 rejects the old `"pending"` block tag (`Invalid
> params`) — use `"pre_confirmed"` (`BlockTag.PRE_CONFIRMED`).

## The alternative: just use instant mining

All four symptoms vanish under plain `--dev` (drop `--block-time`): each tx mines
immediately, so `latest` *is* the truth — no pre-confirmed gymnastics, no nonce
races, no stale estimates. The trade is settlement cadence (a settlement tx per
action, bursty) and a non-persistent, restart-wiped chain. This demo keeps the 5s
blocks for the steadier settlement feel and the persistent state, and pays for it
with the four client/Torii adjustments above.

---

Back to the [client](./client.md) read/write layer, or the [services](./services.md)
that run the appchain, saya, and the toriis.
