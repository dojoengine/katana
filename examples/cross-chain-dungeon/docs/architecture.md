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
| `game` | appchain (L2, local) | the live run, the GOLD vault, the leaderboard | play is high-frequency, cheap, instant |
| `bank` | **Starknet Sepolia** (L1, real) | mints GOLD when a withdrawal settles | the durable, publicly-verifiable record |

The difference from cross-chain-game: the settlement layer isn't a second local
Katana — it's **real Starknet Sepolia**. So you run **one** node locally (the
appchain), and everything on the settlement side (piltover, the `bank` world, the
token contracts) is deployed to a public chain that other contracts and users can
see. `cairo/game/src/lib.cairo` is the appchain world; `cairo/score/src/lib.cairo`
is the settlement `bank` world.

## The token economy (depending on an external contract)

The cross-chain loop is wrapped in a **two-token** economy. **GAME** is the entry
credit (bought with Circle's external **USDC**); **GOLD** is the winnings, minted on
L1 when you bank. This also demonstrates **relying on a contract you don't own** —
USDC on Sepolia:

```
 USDC (external) ──approve+transferFrom──► TokenSale ──mint──► GAME ──┐ pay entry fee
                                                                      Entry ──send L1→L2──► run
 GOLD (L1) ◄──mint── bank world ◄──settle── withdraw ◄──extract (vault, L2) ◄── collect gold
```

- **`token_sale`** pulls real USDC and mints **GAME** at a fixed rate. This is the
  external dependency: the sale calls `USDC.transfer_from`, a contract the demo
  doesn't deploy. A `dev_mint` faucet on GAME lets you skip USDC during development.
- **`entry`** charges a **GAME** fee and sends the L1→L2 message that starts a run.
- The appchain accumulates collected gold into a per-player **vault** (on extract).
- **`bank`** mints **GOLD** to the player when a withdrawal settles — closing a
  spend-GAME-to-play / earn-GOLD-to-keep loop. (`cairo/token/src/lib.cairo` holds the
  two ERC20s plus the sale and entry contracts.)

Why two tokens? GAME keeps the *cost of playing* under the game's control (faucet,
tunable rate) while anchoring to USDC at purchase; GOLD makes the *winnings* a real,
ownable L1 asset that only exists once a run's haul has crossed the bridge.

## The cross-chain loop

Two messages, the same shapes as any appchain app, but now the "commit" carries
real economic weight:

- **L1 → L2 (instant):** `entry.enter()` on Sepolia charges the GAME fee and calls
  the piltover core's `send_message_to_appchain`; the appchain relays it into the
  `mint_run` `#[l1_handler]`, starting the run.
- **L2 → L1 (settled, batched):** `extract()` banks a run's gold into the on-L2
  vault (no message). When the player chooses to bank, `withdraw()` calls
  `send_message_to_l1` with `[player, amount, withdraw_no]` for the *whole* vault;
  once saya settles that block, `bank.bank` on Sepolia consumes it and mints GOLD.

**Death forfeits, extract banks, withdraw bridges.** If HP hits 0 the in-progress
run's gold is lost (it never reached the vault). Extracting locks a run's gold into
the vault on L2; only `withdraw` + settlement carries value to Sepolia. So the game
has two decisions: push-or-extract (per run), and when to bank the vault (per batch).

## Read path vs write path

Same split as any appchain app: **writes** are signed Starknet transactions
(systems + piltover + the token contracts); **reads** come from **Torii** (model
rows + event feeds), eventually consistent. The few non-world facts (token
balances, the piltover settled height, the appchain tip) are read straight from
RPC. The client (`app/src/chain.ts`) never decodes contract storage.

## Where each concern lives

| Concern | Lives in | File |
| --- | --- | --- |
| Live run + dungeon logic + vault + leaderboard | `game` world (appchain) | `cairo/game/src/lib.cairo` |
| GOLD mint on settlement | `bank` world (Sepolia) | `cairo/score/src/lib.cairo` |
| Entry credit + winnings | `game_token` (GAME) + `gold_token` (GOLD) | `cairo/token/src/lib.cairo` |
| USDC → GAME purchase | `token_sale` (Sepolia) | `cairo/token/src/lib.cairo` |
| Charge + start a run | `entry` (Sepolia) | `cairo/token/src/lib.cairo` |
| Cross-chain mailbox + settled state | piltover core (Sepolia) | deployed by `katana init rollup` |
| Indexing for the client | two Torii instances | `up.sh` |
| Client reads/writes | React app | `app/src/chain.ts`, `app/src/App.tsx` |

Next: [why each service exists and how it works →](./services.md)
