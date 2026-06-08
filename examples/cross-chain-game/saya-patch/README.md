# saya-tee patch — L1→L2 message hash for Starknet-settled appchains

This demo requires a small fix to [`saya`](https://github.com/cartridge-gg/saya)
**v0.4.0**. Without it, the **L1→L2 direction stalls settlement**.

## The bug

When the appchain consumes an L1→L2 message (the `mint_game` L1-handler from a
purchase), saya-tee must settle that block onto the piltover core. piltover
recomputes a `messages_commitment` over the block's messages and asserts it
matches what Katana attested. saya-tee derives the L1→L2 message hashes with the
**Ethereum `keccak256`** `StarknetMessaging.sol` formula:

```rust
// bin/persistent-tee/src/settlement.rs — original
fn compute_l1_to_l2_msg_hash(msg: &L1ToL2Message) -> Felt {
    let mut hasher = Keccak256::new();
    hasher.update(&msg.from_address.to_bytes_be()[12..]); // 20-byte ETH addr
    hasher.update(msg.to_address.to_bytes_be());
    hasher.update(msg.nonce.to_bytes_be());
    hasher.update(msg.selector.to_bytes_be());
    hasher.update(Felt::from(msg.payload.len() as u64).to_bytes_be());
    for p in &msg.payload { hasher.update(p.to_bytes_be()); }
    Felt::from_bytes_be(&hasher.finalize().into())
}
```

But a saya-tee-settled appchain settles to a **Starknet** piltover core, so the
canonical hash is **Poseidon**, not keccak. Katana stores the L1-handler
`message_hash` as `Poseidon([from, to, nonce, selector, calldata.len(), ...calldata])`
where `calldata = [from, ...payload]` (see katana
`crates/messaging/src/stream/collector/starknet.rs::compute_starknet_to_appchain_message_hash`,
whose result flows into the appchain attestation's `messages_commitment`).

keccak ≠ poseidon ⇒ commitment mismatch ⇒ piltover reverts with
`'tee: invalid messages'`, and every later block then fails `'invalid block number'`.
(The repo's own `tests/saya-tee` only settles plain transfer blocks, so this path
was never exercised.)

## The fix

Replace `compute_l1_to_l2_msg_hash` with the Poseidon formula Katana uses:

```rust
// bin/persistent-tee/src/settlement.rs — patched
use starknet_types_core::hash::{Poseidon, StarkHash};

fn compute_l1_to_l2_msg_hash(msg: &L1ToL2Message) -> Felt {
    // Katana hashes over the L1-handler calldata = [from_address, ...payload].
    let mut calldata: Vec<Felt> = Vec::with_capacity(msg.payload.len() + 1);
    calldata.push(msg.from_address);
    calldata.extend(msg.payload.iter().copied());

    let mut buf: Vec<Felt> = vec![
        msg.from_address,
        msg.to_address,
        msg.nonce,
        msg.selector,
        Felt::from(calldata.len() as u64),
    ];
    buf.extend(calldata);
    Poseidon::hash_array(&buf)
}
```

Remove the now-unused `use sha3::{Digest, Keccak256};`.

## Apply & install

```bash
git clone https://github.com/cartridge-gg/saya && cd saya
git checkout v0.4.0
# apply the change above to bin/persistent-tee/src/settlement.rs, then:
cargo install --path bin/persistent-tee --force --locked
```

`saya-ops` is installed the same way from `bin/ops` (or its package), and both
must be on `PATH`. `up.sh` checks for them.
