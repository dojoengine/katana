# The services: why, where, how

[← architecture](./architecture.md) · Next: [contracts →](./contracts.md)

The processes that make up the running system. This is the same cast as
[cross-chain-game's services](../../cross-chain-game/docs/services.md) with one big
change — **the settlement layer is remote** — so there's no second local Katana,
and saya/piltover/Torii all point at real Sepolia.

```
   Starknet Sepolia (remote)                     Local appchain (Katana rollup, :5070)
  ┌─────────────────────────────────┐           ┌──────────────────────────────┐
  │ piltover core   score world      │   L1→L2   │  game world (dungeon)        │
  │ GAME_TOKEN  TokenSale  Entry      │ ◄───────► │                              │
  │       ▲            ▲              │   L2→L1   └───────┬──────────────────────┘
  └───────│────────────│─────────────┘ (settled)         │ update_state
          │ index      │ index                    saya ──┘ (--mock-prove)
       Torii(:8091)  Torii(:8092)◄── client ──┐
          (Sepolia)    (appchain)             └── reads/writes
```

## Katana — the sequencer (you run one)

Only the **appchain** is a local Katana here; the settlement role is filled by real
Sepolia. The appchain is created by `katana init rollup` (which deploys piltover on
Sepolia and writes the chain config) and runs as a rollup:

```bash
katana --chain "$CHAIN_DIR" --tee mock --dev --dev.no-fee --block-time 5000 \
       --data-dir .run/appchain-db --http.port 5070 --explorer --messaging.enabled
```

- `--tee mock` — TEE-settled rollup with mock attestation locally.
- `--messaging.enabled` — watch **Sepolia** and relay L1→L2 messages as
  `L1HandlerTx`. Without this, entries never reach the appchain.
- `--dev --dev.no-fee` — fees off (so play actions are free) on chain id `DUNGEON`.
- `--block-time 5000` + `--data-dir` — mine on a steady 5s interval and persist
  state to disk. Both are deliberate; they change the timing model enough that the
  client and Torii must read/write the **pre-confirmed** block. See
  [interval-mining.md](./interval-mining.md).

Port `5070` (and the toriis on `8091`/`8092`, the client on `3002`) are chosen
distinct from cross-chain-game so both demos can run at once.

## piltover core — the cross-chain mailbox, on Sepolia

Deployed by `katana init rollup --tee` **on Sepolia** (a real, gas-costing
deploy). Same interface as before — `send_message_to_appchain` (L1→L2),
`consume_message_from_appchain` (L2→L1, succeeds only after settlement),
`get_state` (settled height for the UI gauge). The difference is purely that it
lives on a public chain, so its operator account must be funded with real STRK.

## saya — the prover, now settling to a real chain

The `saya-tee --mock-prove` sidecar watches the appchain, proves each block, and
submits `update_state` to the piltover core **on Sepolia**:

```bash
saya-tee tee start --mock-prove \
  --rollup-rpc http://localhost:5070 \
  --settlement-rpc "$SEPOLIA_RPC_URL" \
  --settlement-piltover-address "$PILTOVER" \
  --settlement-account-address "$SAYA_ADDRESS" ...
```

Two consequences of settling to a real chain:

- **saya pays real gas** for every `update_state`. Give it a **dedicated** funded
  account, distinct from the operator — sharing one causes nonce contention that
  stalls settlement (cross-chain-game hit exactly this). `init rollup` and `saya`
  must use the *same* account (the piltover operator is the only `update_state`
  caller), and that account is the saya account here.
- **`--mock-prove` still applies.** It exercises the settlement plumbing (message
  hashes, state roots) against a real chain without a real SP1/TEE prover. And the
  **Poseidon L1→L2 hash patch is still required** — a Starknet-settled appchain
  hashes L1→L2 messages with Poseidon, and stock saya 0.4.0 ships keccak, which
  stalls every entry. See [contracts.md](./contracts.md#the-message-hash-gotcha).

The **mock TEE registry** (the on-L1 attestation verifier) is also deployed on
Sepolia, by `saya-ops`, before `init rollup`.

## Torii — the indexers (one per chain)

Two instances, as before, but the settlement one indexes a **Sepolia** world:

```bash
torii --rpc "$SEPOLIA_RPC_URL" --world "$SCORE_WORLD" --http.port 8091 ...                       # Sepolia
torii --rpc http://localhost:5070 --world "$GAME_WORLD" --http.port 8092 --indexing.preconfirmed # appchain
```

Torii resolves the world's deploy block from the contract, so the Sepolia indexer
doesn't rescan the whole chain. Token balances aren't world state, so the client
reads them straight from Sepolia RPC (`balanceOf`), not Torii.

The appchain Torii adds **`--indexing.preconfirmed`** so it indexes the pre-confirmed
block — with 5s `--block-time`, the dungeon view would otherwise lag a full interval
behind each action. The Sepolia bank Torii doesn't need it (real L1 blocks pace it).
See [interval-mining.md](./interval-mining.md).

## Who triggers whom

| Step | Actor | Touches |
| --- | --- | --- |
| Buy GAME | client → `token_sale` (Sepolia) | `USDC.transfer_from` + mint GAME |
| Enter | client → `entry` → piltover (Sepolia) | charge GAME, emit `MessageSent` |
| Relay | appchain Katana (`--messaging.enabled`) | runs `mint_run` |
| Play | client → `game` system (appchain) | one tx per action; `extract` → `send_message_to_l1` |
| Settle | saya → piltover (Sepolia) | registers L2→L1 message hashes |
| Bank | client → `score` (Sepolia) | `consume_message_from_appchain` + mint reward |
| Read | client → Torii ×2 + RPC | run state, feeds, balances, settled height |

Next: [how the contracts implement all this →](./contracts.md)
