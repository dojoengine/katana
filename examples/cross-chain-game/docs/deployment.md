# Build, migrate, and run the stack

[← contracts](./contracts.md) · Next: [client →](./client.md)

This chapter goes from source to a running system: the toolchain, what `sozo
migrate` does, how the demo wires three Dojo worlds across two chains, and the
full bring-up sequence you can copy for your own app.

## Toolchain

Dojo's compiler plugin, migrator, and indexer are versioned together and must
match the world contract your chain accepts. Pin them per project — the demo uses
`../.tool-versions`:

```
scarb 2.13.1
sozo 1.8.7
torii 1.8.16
```

Two consequences worth understanding:

- **`sozo` bundles its own scarb/cairo**, so build with `sozo build`, not a bare
  `scarb`. The demo pins `scarb 2.13.1` anyway so the version matches the plugin.
- **The Dojo dependency must match `sozo`.** The world class hash `sozo` deploys
  has to equal the one your contracts compile against, or migration fails. The
  demo depends on Dojo **by path** from a sibling checkout at the `sozo`-matching
  commit (`cairo/game/Scarb.toml`); a git tag pinned to the same release works too.

## What `sozo migrate` does

`sozo migrate` (default profile reads `dojo_dev.toml`) takes a built world and, in
one run: deploys the **world** (deterministic from its `seed`), declares + registers
**models/events/systems**, grants **permissions**, and calls each system's
**`dojo_init`** with the configured args. It writes the resulting addresses to
`manifest_dev.json`.

It's driven by a per-profile config. The fields that matter:

```toml
[world]                      # deterministic address from name+seed
seed = "ccg_game"
[namespace]
default = "game"             # resources register under this namespace
[env]                        # which chain + signer to migrate against
rpc_url = "http://localhost:5051/"
account_address = "0x…"
private_key = "0x…"
[writers]                    # permission: this system may write this namespace
"game" = ["game-game"]
[init_call_args]             # dojo_init(...) calldata, by contract tag
"game-game" = ["0x…score_system"]
```

You read deployed addresses back out of the manifest:

```ts
const manifest = loadJson<Manifest>(resolve(cwd, "manifest_dev.json"));
const world = manifest.world.address;
const system = manifest.contracts.find((c) => c.tag === s.systemTag).address;
```
[`scripts/lib.ts:106`](https://github.com/dojoengine/katana/blob/ae0e4ee74dc915b5db3b810eefc9c9b1452ca379/examples/cross-chain-game/scripts/lib.ts#L106)

## Migration order (wiring across chains)

A world's `dojo_init` often needs another world's address, so the demo migrates
**in dependency order**, feeding each result into the next. `scripts/deploy.ts`
runs three passes — `score` (settlement), then `game` (appchain, needs the score
system address), then `store` (settlement, needs the game system address):

```ts
const score = migrateWorld({ pkg: "score", seed: "ccg_score", namespace: "score",
  systemTag: "score-score_registry", rpcUrl: d.settlement.rpcUrl, account: d.settlement.account,
  initArgs: [d.settlement.piltover] });          // ← init arg known up front

const game = migrateWorld({ pkg: "game", seed: "ccg_game", namespace: "game",
  systemTag: "game-game", rpcUrl: d.appchain.rpcUrl, account: d.appchain.account,
  initArgs: [score.system] });                   // ← from the score pass

const store = migrateWorld({ pkg: "store", seed: "ccg_store", namespace: "store",
  systemTag: "store-store", rpcUrl: d.settlement.rpcUrl, account: d.settlement.account,
  initArgs: [d.settlement.piltover, game.system] }); // ← needs the game system
```
[`scripts/deploy.ts:20`](https://github.com/dojoengine/katana/blob/ae0e4ee74dc915b5db3b810eefc9c9b1452ca379/examples/cross-chain-game/scripts/deploy.ts#L20)

`migrateWorld` (`scripts/lib.ts`) generates that world's `dojo_dev.toml` from these
values, runs `sozo build` then `sozo migrate`, and returns the parsed addresses,
which the script records in `deployments.json` for the client. The generated
`dojo_dev.toml` and `manifest_dev.json` are gitignored (regenerated each run).

> **General rule:** if world A's `dojo_init` needs world B's address, migrate B
> first and pass it in. If the dependency is only needed at *call* time (like the
> settlement consumer needing the appchain sender), pass it from the client
> instead and avoid a deploy-order cycle.

## The full bring-up sequence

`up.sh` orchestrates the whole system. The order isn't arbitrary — each step
depends on the previous. For your own app, this is the template:

1. **Preflight** — install the Dojo toolchain (`asdf install`) + JS deps and
   verify the heavy prerequisites (katana binary, patched saya, dojo checkout).
2. **Settlement Katana** (`:5050`, `SN_SEPOLIA`). [`up.sh:91`](https://github.com/dojoengine/katana/blob/ae0e4ee74dc915b5db3b810eefc9c9b1452ca379/examples/cross-chain-game/up.sh#L91)
3. **Mock TEE registry** via `saya-ops` — the attestation verifier saya needs. [`up.sh:98`](https://github.com/dojoengine/katana/blob/ae0e4ee74dc915b5db3b810eefc9c9b1452ca379/examples/cross-chain-game/up.sh#L98)
4. **piltover core + rollup config** via `katana init rollup --tee` — deploys the
   mailbox on L1 and writes the appchain's chain config. [`up.sh:109`](https://github.com/dojoengine/katana/blob/ae0e4ee74dc915b5db3b810eefc9c9b1452ca379/examples/cross-chain-game/up.sh#L109)
5. **Base `deployments.json`** — rpc urls, accounts, piltover, Torii urls. [`up.sh:125`](https://github.com/dojoengine/katana/blob/ae0e4ee74dc915b5db3b810eefc9c9b1452ca379/examples/cross-chain-game/up.sh#L125)
6. **Appchain Katana** (`:5051`, rollup, `--tee mock --messaging.enabled`). [`up.sh:147`](https://github.com/dojoengine/katana/blob/ae0e4ee74dc915b5db3b810eefc9c9b1452ca379/examples/cross-chain-game/up.sh#L147)
7. **saya-tee** sidecar — starts proving/settling appchain blocks. [`up.sh:159`](https://github.com/dojoengine/katana/blob/ae0e4ee74dc915b5db3b810eefc9c9b1452ca379/examples/cross-chain-game/up.sh#L159)
8. **Migrate the three worlds** (`scripts/deploy.ts`: score → game → store) and record addresses.
9. **Two Torii instances** — settlement world `:8081`, appchain world `:8082`,
   distinct relay ports. [`up.sh:185`](https://github.com/dojoengine/katana/blob/ae0e4ee74dc915b5db3b810eefc9c9b1452ca379/examples/cross-chain-game/up.sh#L185)
10. **Client** (Vite, `:3001`).

```bash
cd examples/cross-chain-game && ./up.sh     # Ctrl-C / ./down.sh tears it all down
```

## Verify each stage

Don't trust the logs — check the chain and the indexers:

```bash
# all worlds migrated? (addresses filled in)
node -e 'console.log(require("./app/src/deployments.json"))'

# Torii indexed the initial model rows?
curl "http://localhost:8082/sql?query=$(python3 -c 'import urllib.parse;print(urllib.parse.quote("SELECT * FROM \"game-Stats\" WHERE id=0"))')"

# saya settling? compare settled vs tip
# (piltover get_state vs appchain block height — the UI's saya gauge)
```

A full round-trip check (buy → play → settle → bank) is the real test; the client
chapter shows the calls, or drive `chain.ts` directly from a script.

Next: [how the client queries all this state →](./client.md)
