# Cross-Chain Dungeon — the guide

This guide builds the mental model for an appchain app that **settles to a real
public chain** and **leans on an external settlement-layer contract**, using the
dungeon game as the running example. It mirrors the
[`cross-chain-game` guide](../../cross-chain-game/docs/README.md); read that first
if you want the gentler two-local-Katana version. Here we focus on what changes
when the settlement layer is **real Starknet Sepolia** and there's a **token
economy** in front of the cross-chain loop.

## Chapters

1. **[Architecture](./architecture.md)** — the worlds, the one-appchain /
   real-Sepolia split, the USDC → GAME_TOKEN economy, and the read/write paths.
2. **[Services](./services.md)** — the appchain Katana, piltover on Sepolia, saya
   (`--mock-prove`, settling to a real chain), and the two Torii indexers.
3. **[Contracts](./contracts.md)** — the dungeon world, the token contracts, both
   messaging directions, and the death-is-local / extract-commits design.
4. **[Deployment](./deployment.md)** — toolchain, deploy order (token + worlds +
   minter grants), and the Sepolia bring-up sequence.
5. **[Client](./client.md)** — the data layer, the Torii tables, the write path,
   Controller-on-Sepolia, and the poll-and-derive UI.
6. **[Interval mining & pre-confirmed play](./interval-mining.md)** — why the
   appchain mines on a 5s interval and persists to disk, and the four
   pre-confirmed adjustments (tx wait, Torii indexing, nonce, fee estimate) that
   keep play snappy and correct.

## The game in one paragraph

Buy `GAME_TOKEN` with USDC (or dev-mint it). Pay it to **enter** — an L1→L2 message
starts a run on the appchain. Descend room by room; each **move / attack / loot /
use-item** is its own appchain transaction, and risk and reward both climb with
depth. **Extract** while alive to bank your score to Sepolia and mint a reward
(the L2→L1 settled commit). **Die** and the haul evaporates on L2 — nothing
settles. See [contracts.md](./contracts.md#the-dungeon-game) for the mechanics.
