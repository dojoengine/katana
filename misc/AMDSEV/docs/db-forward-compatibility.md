# Database forward-compatibility in TEE mode

**Question:** when we ship a newer Katana version into the SEV-SNP VM, can it open
the database that an older Katana version persisted on the sealed data disk?

**Short answer:** not today, and the reason has nothing to do with the database
format. The data disk is unlocked with a key derived from the launch
*measurement*, and the Katana binary is part of that measurement. A version bump
changes the measurement, changes the key, and `luksOpen` fails before
`katana-db` ever runs. The DB format itself is already forward-migratable; the
sealing layer is the blocker.

This document audits exactly where forward-compat breaks and lays out the
options. It does **not** change behaviour.

---

## Two independent layers

A persisted-state upgrade has to clear two gates, in order:

1. **Sealed disk (LUKS / dm-integrity)** — can the new VM *decrypt* the old disk?
2. **`katana-db` on-disk format** — given a decrypted disk, can the new binary
   *read and migrate* the old schema?

Gate 2 is in good shape. Gate 1 is the wall.

---

## Layer 1 — sealed disk (the blocker)

### Mechanism

In sealed mode (the default) the LUKS passphrase is the SNP-derived key, used
**directly**, with a **single keyslot** and **no wrapped-DEK indirection**:

- `snp-tools/src/bin/snp-derivekey.rs:37` — `FIELD_SELECT = "001001"` =
  `MEASUREMENT | GUEST_POLICY`. The derived key is a function of the launch
  measurement and the guest policy. (TCB/SVN bits are deliberately *off* so
  firmware/SVN updates don't rotate the key — but the MEASUREMENT bit is on.)
- `scripts/build-initrd.sh:1219` — `snp-derivekey` writes 32 bytes into a FIFO.
- `scripts/build-initrd.sh:1256-1257` — first boot: `luksFormat` seals the
  header with that key as the only keyslot.
- `scripts/build-initrd.sh:1274-1276` — every subsequent boot: `luksOpen` with
  the re-derived key; on mismatch it halts:
  `"luksOpen failed — chip mismatch or measurement drift"`.

### Why a Katana version bump breaks it

The measurement covers the **initrd**, and the Katana binary is baked into the
initrd (`docs/release-pipeline.md`: "New katana version → Initrd hash changes →
new measurement"). So:

```
new katana binary
  → new initrd hash
  → new launch measurement
  → new derived key (FIELD_SELECT includes MEASUREMENT)
  → luksOpen fails on the old disk  → guest halts
```

The old disk is not corrupted — it simply cannot be unlocked by the new image.
From the operator's perspective the chain "resets" (a fresh `luksFormat` only
happens on a *headerless* disk; an existing header that won't open just halts).

Note this is by design and orthogonal to Katana: **any** measured-byte change
re-keys the disk — kernel pin, OVMF, cmdline, initrd scripts
(`docs/release-pipeline.md`, "What moves the measurement between releases").

### Why you can't pre-stage the next key

LUKS2 supports multiple keyslots, so in principle you could add a second slot
keyed by the *next* version's derived key and migrate without re-encrypting.
**But `snp-derivekey` only ever yields the key for the *currently running*
measurement.** A VM running the old image cannot compute the new image's key, so
it cannot add the new slot ahead of time. Any in-place re-key therefore needs a
transition image trusted to hold *both* keys at once, or an external key source.

### What it would take to make Layer 1 forward-compatible

| Option | Mechanism | Tradeoff |
|---|---|---|
| **A. Decouple key from measurement** | Change `FIELD_SELECT` to bind to a stable identity field (`FAMILY_ID` / `IMAGE_ID`, set by the launcher, measured into the report but not into the key) instead of `MEASUREMENT`. Any newer image with the same family/image ID derives the same disk key. | Loses the "only this *exact* code can decrypt the data" property. The host could launch a different image under the same ID and unseal. You must lean entirely on attestation policy to constrain what runs. |
| **B. External / escrowed key (KMS)** | Disk key comes from an external service that releases it only after verifying the attestation report against a policy (allow-list of measurements, or a signer). | Adds an online dependency and a trusted KMS + verifier; richest policy control; standard production pattern for confidential VMs with persistent storage. |
| **C. Wrapped DEK + re-key ceremony** | Store a random disk-encryption key on the header, wrapped by the measurement-derived KEK. On upgrade, a transition step unwraps with the old KEK and re-wraps with the new KEK. | No disk re-encryption, but needs a trusted transition image that holds both KEKs (chicken-and-egg from §"pre-stage"), plus careful operational choreography. Most moving parts. |
| **D. Unsealed mode** | `--unsealed`: plain ext4, no derived key. New versions open the old disk freely. | No confidentiality / integrity at rest. Only acceptable where the disk's secrecy isn't part of the threat model. |

Options A and B are the realistic ones if persistent state must survive Katana
upgrades. Both **weaken or relocate** the "code identity ⇒ data access" binding
that the current design gets for free — that is the core security decision and
it is the team's to make.

---

## Layer 2 — `katana-db` on-disk format (already forward-migratable)

If the disk *does* open (unsealed mode, or after Layer 1 is solved), the DB layer
already supports opening older databases:

- **Version window** — `crates/storage/db/src/version.rs:11-13`:
  `LATEST_DB_VERSION = 9`, `MIN_OPENABLE_DB_VERSION = 5`. A current binary opens
  on-disk versions **5–9**. Outside that range it errors with
  `IncompatibleVersion` (`version.rs:29-34`, `61-71`).
- **Version file** — a 4-byte big-endian `db.version` file in the data dir
  (`version.rs:16`, `74-83`).
- **Migrations run on open** — `crates/storage/db/src/migration/mod.rs`: a staged
  pipeline (e.g. v8→v9 rebuilds `BlockStateUpdates` and converts receipts/txs to
  the envelope format) with batching + checkpointing, then bumps `db.version` to
  `LATEST_DB_VERSION`.
- **New tables auto-created** — `Tables::ALL` → `create_default_tables`; a newer
  binary that adds tables just creates them on open.
- **Regression fixture** — `tests/db-compat/` ships an old DB snapshot to guard
  backward reads.

### Risks that would silently break Layer 2 over time

These are the things to keep honest if forward-compat is a hard requirement:

1. **`MIN_OPENABLE_DB_VERSION` creeping up.** Each bump drops the oldest openable
   version. If an in-the-field TEE disk is older than the new minimum, the new
   binary rejects it even with the disk decrypted. Treat raising this constant as
   a breaking change.
2. **A format bump shipped without a migration stage.** Bumping
   `LATEST_DB_VERSION` without a corresponding migration leaves old rows
   unreadable.
3. **Envelope-format / encoding changes** (`crates/storage/db/src/models/envelope.rs`)
   that aren't matched by a migration — pre-envelope rows must be rewritten, not
   read in place.
4. **One-way migrations.** Once the new binary migrates a disk to v9, an *older*
   Katana can no longer open it. There is no downgrade path. In TEE terms: a
   rollback to a previous VM image is also a rollback to a previous measurement
   (so under Layer-1 option A/B it would re-open the disk, then fail to read the
   already-migrated format). Rollbacks must be planned as data operations.

### Cheap guardrails (if/when we act on Layer 2)

- A CI check that any PR raising `LATEST_DB_VERSION` also adds a migration stage
  and refreshes a `db-compat` fixture for the previous version.
- A CI check / explicit sign-off gate on any change to `MIN_OPENABLE_DB_VERSION`.
- Extend `tests/db-compat/` to assert that a DB created by the *minimum openable*
  version opens and migrates clean under HEAD.

---

## Verdict

"Forward-compatible database in TEE mode" requires **both** gates, and they are
currently asymmetric:

- **Layer 2 (DB format):** already migrates forward (v5→v9). Needs only
  discipline/CI to *stay* that way.
- **Layer 1 (sealing):** hard blocker. The measurement-bound, single-slot,
  no-DEK design means a Katana version bump cannot open the prior sealed disk at
  all. Solving it means choosing one of options A–D above, each of which
  relocates or weakens the code-identity⇒data-access guarantee.

Recommended next decision: pick the Layer-1 key-binding strategy (A vs B are the
realistic candidates), since everything else is downstream of it. Layer-2
guardrails are low-cost and can land independently.

---

*Audit only — reflects the code at the time of writing. Key references:
`snp-tools/src/bin/snp-derivekey.rs`, `misc/AMDSEV/scripts/build-initrd.sh`
(`unseal_and_mount`), `crates/storage/db/src/version.rs`,
`crates/storage/db/src/migration/mod.rs`, `misc/AMDSEV/docs/release-pipeline.md`.*
