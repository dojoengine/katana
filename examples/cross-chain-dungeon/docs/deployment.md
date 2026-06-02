# Build, deploy, and run the stack

[← contracts](./contracts.md) · Next: [client →](./client.md)

From source to a running system. The toolchain and `sozo migrate` mechanics are
identical to [cross-chain-game's deployment chapter](../../cross-chain-game/docs/deployment.md)
(`scarb 2.13.1` / `sozo 1.8.7` / `torii 1.8.16`, Dojo by path from a sibling
checkout). What's new here: **plain-contract deploys alongside the Dojo migrations**,
**minter grants**, and a **real-Sepolia bootstrap** that needs funded accounts.

## Configuration (`.env`)

Because settlement is a real chain, the network choice, accounts, and external USDC
address come from the environment (`.env.example` → `.env`):

```
SETTLEMENT_NETWORK=sepolia                  # sepolia (default) or mainnet
SETTLEMENT_RPC_URL=…                        # RPC for that network (SEPOLIA_RPC_URL still works)
OPERATOR_ADDRESS=…  OPERATOR_PRIVATE_KEY=…   # deploys contracts + migrates the bank world
SAYA_ADDRESS=…      SAYA_PRIVATE_KEY=…       # piltover operator + update_state (dedicated!)
USDC_ADDRESS=…                              # real Circle USDC for the chosen network (verify it)
GAME_RATE=… ENTRY_FEE=… REWARD_PER_GOLD=…    # economy (base units)
```

`SETTLEMENT_NETWORK` selects the chain id (`SN_SEPOLIA` / `SN_MAIN`), the explorer,
and the display name; `up.sh` records all of these into `deployments.json` so the app
is network-agnostic. The RPC and USDC must match the chosen network — **mainnet means
real funds**. `scripts/config.ts` loads the economy values as base units (GAME/GOLD
have 18 decimals, USDC 6) so the rate carries the decimal conversion.

## Two kinds of deploy

`scripts/deploy.ts` does both, in dependency order, recording everything into
`app/src/deployments.json`:

- **Dojo worlds** via `sozo migrate` (`migrateWorld` in `scripts/lib.ts`) — the
  `score` world on Sepolia, the `game` world on the appchain.
- **Plain Starknet contracts** via starknet.js `declareAndDeploy` (`game_token`,
  `token_sale`, `entry`) — these aren't worlds, so they're declared + deployed
  directly, then configured with `invoke` (the minter grants).

```ts
const gameToken = await declareAndDeploy(operator, "token", "game_token", { owner }); // GAME
const goldToken = await declareAndDeploy(operator, "token", "gold_token", { owner }); // GOLD
const bank  = migrateWorld({ pkg: "score", namespace: "bank", … initArgs: [piltover, goldToken, ...u256(rewardPerGold)] });
const game  = migrateWorld({ pkg: "game",  … initArgs: [bank.system] });
const tokenSale = await declareAndDeploy(operator, "token", "token_sale", { usdc, game_token: gameToken, treasury, rate });
const entry     = await declareAndDeploy(operator, "token", "entry", { game_token: gameToken, entry_fee, piltover, appchain_game: game.system });
await invoke(operator, gameToken, "set_minter", [tokenSale, "0x1"]);   // sale mints GAME
await invoke(operator, goldToken, "set_minter", [bank.system, "0x1"]); // bank mints GOLD
```

(`scripts/deploy.ts`.) The order matters: the token before the world+sale that
reference it; the score world before the game world (which needs its address); the
game world before `entry` (which addresses it); the grants last.

## The full bring-up sequence

`up.sh` orchestrates it. The settlement steps run against **real Sepolia**:

1. **Preflight** — `asdf install`, verify katana / patched saya / sozo·torii·scarb /
   the sibling dojo checkout, and that `.env` is filled.
2. **Mock TEE registry on Sepolia** (`saya-ops`, operator account).
3. **piltover core + rollup config** via `katana init rollup --tee` against Sepolia
   (saya account = piltover operator).
4. **Base `deployments.json`** — Sepolia + appchain rpc/accounts, piltover, USDC.
5. **Appchain Katana** (`:5070`, `--tee mock --messaging.enabled`).
6. **saya-tee** (`--mock-prove`, settling to Sepolia).
7. **Deploy economy + worlds** (`scripts/deploy.ts`).
8. **Two Torii instances** — Sepolia `score` (`:8091`), appchain `game` (`:8092`).
9. **Client** (Vite, `:3002`).

```bash
cp .env.example .env && ./up.sh     # Ctrl-C / ./down.sh tears down the local procs
```

## Costs & gotchas (real chain)

- Every deploy + every `saya update_state` costs real Sepolia STRK. Fund the
  operator generously and give **saya a dedicated account** (nonce contention with
  the operator stalls settlement).
- The **Poseidon saya patch** is required (see [contracts.md](./contracts.md#the-message-hash-gotcha)).
- `init rollup` against Sepolia needs the chain id (`SN_SEPOLIA`) and a funded
  account; a balance/chain-id mismatch fails the deploy.
- **Blake2s compiled-class hash (Starknet ≥ 0.14.1).** Sepolia/mainnet compute the
  `compiled_class_hash` with **Blake2s**, not Poseidon, and reject a declare that
  sends the old hash with `Mismatch compiled class hash`. So the deploy scripts pin
  **starknet.js 10.x** (whose `computeCompiledClassHash` is Blake2s); 8.x's Poseidon
  hash is rejected. `sozo 1.8.7` and the `katana` binary (dojoengine/katana#570)
  already emit Blake2s, so the worlds and `init rollup` are fine. A local Katana
  settlement layer accepts either, which is why `cross-chain-game` never hit this.
  The cairo **compiler** version is unrelated — scarb stays 2.13.1.

## Verify each stage

```bash
node -e 'console.log(require("./app/src/deployments.json"))'   # all addresses filled?
# score world indexed on Sepolia?
curl "http://localhost:8091/sql?query=SELECT%20*%20FROM%20%22score-Leaderboard%22"
# settled vs tip (the UI gauge): piltover get_state vs appchain block height
```

The real test is a full round trip: dev-mint → enter → a few actions → extract →
wait for saya → bank. The [client chapter](./client.md) shows the calls.

Next: [how the client queries and drives all this →](./client.md)
