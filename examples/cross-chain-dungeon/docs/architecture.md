# Application architecture

[← guide index](./README.md) · Next: [services →](./services.md)

The mental model: **where the app's state lives**, **how the two chains connect**,
and **what the token economy adds** on top of the cross-chain loop.

## Two worlds, but only one local chain

Like any Dojo appchain app this is built from **worlds** — an on-chain database
(models) plus the systems that write to them (see the
[cross-chain-game architecture](../../cross-chain-game/docs/architecture.md) for the
Dojo primer). This demo has **two** worlds:

| World | Chain | Holds | Why there |
| --- | --- | --- | --- |
| `game` | appchain (L2, local) | the live run: depth, HP, gold, room, inventory | play is high-frequency, cheap, instant |
| `score` | **Starknet Sepolia** (L1, real) | the leaderboard + the minted rewards | the durable, publicly-verifiable record |

The difference from cross-chain-game: the settlement layer isn't a second local
Katana — it's **real Starknet Sepolia**. So you run **one** node locally (the
appchain), and everything on the settlement side (piltover, the `score` world, the
token contracts) is deployed to a public chain that other contracts and users can
see. `cairo/game/src/lib.cairo` is the appchain world; `cairo/score/src/lib.cairo`
is the settlement world.

## The token economy (depending on an external contract)

The cross-chain loop is wrapped in a small economy that demonstrates **relying on a
contract you don't own** — Circle's **USDC** on Sepolia:

```
 USDC (external) ──approve+transferFrom──► TokenSale ──mint──► GAME_TOKEN ──┐
                                                                            │ pay entry fee
 leaderboard + reward ◄──mint── score world ◄──settle── extract      Entry ◄┘──send L1→L2──► run
```

- **`token_sale`** pulls real USDC from the buyer and mints `GAME_TOKEN` at a fixed
  rate. This is the external dependency: the sale calls `USDC.transfer_from`, a
  contract the demo doesn't deploy. A `dev_mint` faucet on the token lets you skip
  USDC during development.
- **`entry`** charges a `GAME_TOKEN` fee and sends the L1→L2 message that starts a
  run. (`cairo/token/src/lib.cairo` holds all three plain contracts.)
- **`score`** mints a `GAME_TOKEN` reward when a run is banked — closing a
  spend-to-enter / earn-on-win loop.

Why a custom token instead of charging USDC directly? It keeps the game's currency
under the game's control (a faucet for dev, a tunable rate, a reward sink) while
still anchoring its value to the external USDC contract at the point of purchase.

## The cross-chain loop

Two messages, the same shapes as any appchain app, but now the "commit" carries
real economic weight:

- **L1 → L2 (instant):** `entry.enter()` on Sepolia charges the fee and calls the
  piltover core's `send_message_to_appchain`; the appchain relays it into the
  `mint_run` `#[l1_handler]`, starting the run.
- **L2 → L1 (settled):** `extract()` on the appchain calls `send_message_to_l1`
  with `[player, score, loot]`; once saya settles that block, `score.claim_run` on
  Sepolia consumes it, writes the leaderboard, and mints the reward.

**Death never sends a message.** If HP hits 0 the run ends on the appchain only —
no L2→L1 message, no reward. The only way value crosses to Sepolia is a live
`extract`. That makes the settled commit the core decision of the game.

## Read path vs write path

Same split as any appchain app: **writes** are signed Starknet transactions
(systems + piltover + the token contracts); **reads** come from **Torii** (model
rows + event feeds), eventually consistent. The few non-world facts (token
balances, the piltover settled height, the appchain tip) are read straight from
RPC. The client (`app/src/chain.ts`) never decodes contract storage.

## Where each concern lives

| Concern | Lives in | File |
| --- | --- | --- |
| The live run + dungeon logic | `game` world (appchain) | `cairo/game/src/lib.cairo` |
| Leaderboard + reward mint | `score` world (Sepolia) | `cairo/score/src/lib.cairo` |
| Game currency | `game_token` ERC20 (Sepolia) | `cairo/token/src/lib.cairo` |
| USDC → GAME purchase | `token_sale` (Sepolia) | `cairo/token/src/lib.cairo` |
| Charge + start a run | `entry` (Sepolia) | `cairo/token/src/lib.cairo` |
| Cross-chain mailbox + settled state | piltover core (Sepolia) | deployed by `katana init rollup` |
| Indexing for the client | two Torii instances | `up.sh` |
| Client reads/writes | React app | `app/src/chain.ts`, `app/src/App.tsx` |

Next: [why each service exists and how it works →](./services.md)
