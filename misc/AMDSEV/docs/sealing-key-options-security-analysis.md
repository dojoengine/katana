# Security analysis: sealed-disk key-binding options A & B

**Purpose.** The sealed data disk cannot survive a Katana version upgrade because
its LUKS key is bound to the launch **measurement**, and the Katana binary is in
the measured initrd (see `db-forward-compatibility.md`). Two options can restore
forward-compatibility at the sealing layer:

- **Option A** — bind the derived key to `FAMILY_ID` / `IMAGE_ID` (stable
  identity fields) instead of `MEASUREMENT`.
- **Option B** — release the disk key from an external, attestation-gated KMS
  that checks the measurement against an allow-list.

This document evaluates the security of each against the project's stated threat
model. It is analysis only — no code changes.

> **Bottom line up front.** Option A is operationally trivial but, under this
> project's threat model (**the host is untrusted**), it does not merely *weaken*
> the at-rest guarantee — it **removes cryptographic access control against the
> primary adversary** and additionally enables **state-substitution**. Option B
> preserves the "only attested code decrypts" property and solves forward-compat,
> at the cost of pulling a **KMS, an allow-list, and a long-lived escrowed key
> into the TCB**, plus a boot-time network dependency. **B is the only one of the
> two that is consistent with the existing threat model.**

---

## 1. Baseline: what the current design buys

Current key derivation (`snp-tools/src/bin/snp-derivekey.rs:37`,
`FIELD_SELECT="001001"`):

```
disk_key = KDF(VCEK_chip_root, MEASUREMENT | GUEST_POLICY, VMPL=1)
```

Properties (the bar A and B must be measured against):

| Property | Current design |
|---|---|
| Confidentiality at rest vs untrusted host | **Strong** — only a guest whose launch measurement matches can derive the key. The host cannot launch *different* code and read the disk. |
| Integrity / anti-substitution | **Strong** — the same binding gates the dm-integrity key; a host that wipes the LUKS header can only reset to a *fresh* disk, not substitute chosen state (`build-initrd.sh:1205-1209`). |
| Offline disk theft (disk moved to another machine) | **Defeated** — VCEK is chip-unique; the stolen `data.img` is useless elsewhere. |
| External secret / escrow | **None** — the key never exists outside the AMD-SP and the live guest. |
| Network/boot dependency | **None** — fully offline unseal. |
| Forward-compat across Katana versions | **None** — this is the problem we are solving. |

The job of A and B is to recover the last row **without losing the others.**

---

## 2. Threat model (assets, actors, trust boundaries)

Restating the project's model (`README.md:322-325`: "Host software (QEMU, host
kernel, hypervisor) — **Untrusted by design** under SEV-SNP").

**Assets**
1. Confidentiality of persisted chain state at rest (the sealed disk).
2. Integrity of persisted chain state (no attacker-chosen substitution).
3. Availability of the sequencer (it must boot).

**Adversaries**
- **A1 — Untrusted host / hypervisor operator.** Controls QEMU, the host kernel,
  all VM launch parameters, the data disk file, and the network the guest sees.
  This is SEV-SNP's *headline* adversary and the one the sealing exists to stop.
- **A2 — Co-tenant / other guest** on the same physical chip (shares the VCEK
  root, *not* the per-launch inputs).
- **A3 — Offline thief** who exfiltrates `data.img` to a different machine.
- **A4 — Remote network attacker** with no host access.
- **A5 — Malicious/compromised KMS or allow-list maintainer** (relevant only to
  Option B; a new actor that does not exist today).
- **A6 — Supply-chain attacker** against the released VM image / measurement
  allow-list (e.g. slipping a backdoored measurement onto the list).

**Trust boundary today:** AMD-SP + the measured guest are inside; everything else
is outside. The sealed-storage key never crosses that boundary.

---

## 3. The mechanic that decides Option A: who controls FAMILY_ID / IMAGE_ID

`GET_DERIVED_KEY`'s selectable mixing fields are `GUEST_POLICY`, `IMAGE_ID`,
`FAMILY_ID`, `MEASUREMENT`, `GUEST_SVN`, `TCB_VERSION`
(`snp-derivekey.rs:12-16`). `FAMILY_ID` and `IMAGE_ID` are **16-byte values
supplied in the ID block at `SNP_LAUNCH_FINISH`**. In this repo they are **not
set at all** today — there is no `id-block` / `id-auth` / `author-key` on the
`sev-snp-guest` object (`start-vm.sh:551`:
`policy=0x30000,cbitpos=51,reduced-phys-bits=1,kernel-hashes=on` — nothing else).

Critically: **the ID block is provided by whoever launches the VM — the host.**
The firmware verifies the ID block is internally self-consistent (signed by an ID
key endorsed by an author key) and records `ID_KEY_DIGEST` / `AUTHOR_KEY_DIGEST`
in the attestation report — **but it does not pin *which* author key is
acceptable.** Any launcher can generate its own author key, sign an ID block
carrying *any* `FAMILY_ID`/`IMAGE_ID`, and the firmware accepts it. Only an
*external verifier* checking the digests can tell the difference — and there is no
selectable derived-key field for the author/ID-key digest, so **the derived key
cannot be bound to the signer.**

This single fact drives the Option A verdict below.

---

## 4. Option A — bind to FAMILY_ID / IMAGE_ID

### Design
Change `FIELD_SELECT` to select `FAMILY_ID | IMAGE_ID | GUEST_POLICY` (drop
`MEASUREMENT`). Set a fixed family/image id at launch. Every future Katana image
launched with the same ids derives the same disk key → upgrades unseal the old
disk. No network, no external secret.

### Security evaluation

**Gained:** forward-compat, with zero new infrastructure.

**Lost — and this is decisive:**

- **A1 (untrusted host) defeats confidentiality outright.** The key now depends
  only on values the *host* supplies at launch (`FAMILY_ID`/`IMAGE_ID`) plus the
  chip root and policy. The host can launch **any** guest it likes — a malicious
  initrd, a stock Linux, a debugging shell — under the victim's family/image id
  and **derive the identical disk key**, then read the plaintext. Because the
  host is untrusted *by definition* here, this is equivalent to **no cryptographic
  access control on the data at rest.** The property that justified SEV-SNP
  sealing in the first place is gone.

- **State substitution becomes possible (integrity loss).** The same forged
  launch yields the dm-integrity key too, so the attacker can write a fully
  valid, attacker-chosen database. The current design's "header-wipe can only
  reset to a fresh chain, not substitute state" property
  (`build-initrd.sh:1205-1209`) no longer holds.

- **Rollback to vulnerable versions is unconstrained.** Any past or future image
  under the same id reads the data — including a known-broken Katana the host
  spins up specifically to exploit a since-fixed bug against live state.

**Residual protections that *do* survive A:**
- **A3 (offline theft to another machine):** still defeated — the VCEK chip root
  is mixed in, so the disk is useless on different hardware.
- **A2 (co-tenant):** a co-tenant with a *different* family/image id still can't
  derive the key; but A2 collapses into A1 if the host cooperates, and the host
  is untrusted, so this offers little.
- RAM remains SEV-SNP-encrypted against cold-memory attacks (unchanged).

### Can Option A be hardened?
Not within the derived-key mechanism. The intuitive fix — require a *signed* ID
block from the data owner's author key — fails because (a) the firmware doesn't
pin the author key, and (b) there is no derived-key field that mixes in the
author/ID-key digest, so the key is identical regardless of who signed. Guest-side
self-checks are useless: the firmware computes the key for *whatever* guest runs,
including a malicious one that simply skips the check. To actually enforce the
signer you need an external verifier — at which point you have rebuilt Option B.

### When is Option A acceptable?
Only under a **relaxed threat model where the host operator is trusted** (operator
== data owner, single-tenant, no hostile local root), and the only adversaries are
A3/A4 (offline disk theft, remote network). That directly contradicts the current
documented model ("host… untrusted by design"). **Adopting A is a conscious
threat-model downgrade, and should be labeled as such, not slipped in as a
forward-compat fix.**

---

## 5. Option B — external attestation-gated KMS

### Design
1. Measured initrd boots, requests an SEV-SNP attestation report binding a
   **fresh ephemeral transport public key** (and a server-supplied nonce) into
   `REPORT_DATA`. (The report path already exists: `tee_generateQuote` RPC +
   `snp-report` decoder + `report_data` field.)
2. Guest sends `{report, VCEK cert chain}` to the KMS.
3. KMS verifies: AMD cert chain (ARK→ASK→VCEK), report signature, **measurement
   ∈ allow-list**, `GUEST_POLICY` correct, TCB ≥ floor, and **nonce freshness**.
4. KMS returns the disk key (or a wrapping key) **encrypted to the ephemeral
   pubkey from `REPORT_DATA`**.
5. Guest decrypts, opens LUKS.

Forward-compat: a new Katana version → new measurement → **add it to the
allow-list** → KMS releases the *same* disk key. The "only attested code decrypts"
property is **retained**, now enforced by KMS policy over a *set* of measurements
instead of by the chip over a *single* measurement.

### Security evaluation

**Preserved vs baseline:**
- **A1 (untrusted host):** still cannot read the disk. A forged/rogue guest
  produces a report whose measurement is **not on the allow-list**, so the KMS
  refuses the key. This is the property Option A loses and B keeps.
- **A3 (offline theft):** the disk key isn't on the disk and the chip can't
  derive it alone; theft yields nothing without also defeating the KMS.

**New attack surface introduced by B (must be controlled):**

1. **A5 — KMS / allow-list compromise = full break.** Whoever can add a
   measurement to the allow-list (or read the escrowed key) can authorize a rogue
   image and exfiltrate everything. The allow-list and the key store become the
   crown jewels. *Controls:* HSM-backed key custody; signed, append-only,
   peer-reviewed allow-list changes; least-privilege + audit on the KMS.
2. **Long-lived escrowed secret.** Unlike baseline (key exists only in-chip), the
   canonical disk key now persists in the KMS. New secret-at-rest to protect.
3. **Replay / freshness.** A captured valid report must not be replayable.
   *Controls:* server nonce/challenge mixed into `REPORT_DATA`; short report
   validity window; bind ephemeral transport key into `REPORT_DATA` so a host
   MITM can't substitute its own key and intercept the release.
4. **TOCTOU + the unmeasured-args gap (inherited, arguably worse).** The
   measurement covers initrd/kernel/cmdline but **not** Katana's runtime args or
   chain config (`README.md:322-325` — fw_cfg args and the chain disk are
   untrusted operator input the verifier "cannot tell… from the report alone").
   So the KMS can confirm *which binary* booted but not *how it was invoked*. It
   may release live data to a correctly-measured guest that the host launched with
   adversarial `--`args. *Control:* pin the security-relevant args into the
   measured cmdline (or into `REPORT_DATA`) so the KMS can gate on them; treat
   this as a prerequisite, not a nicety.
5. **Rollback / revocation is now an active ops duty.** The allow-list must
   *remove* measurements of known-vulnerable versions, or A1 just boots an old
   allow-listed image to attack live state. Baseline got coarse rollback
   resistance for free (each version had a distinct key); B must enforce it as
   policy. *Control:* minimum-version floor + explicit revocation process.
6. **A4/availability — boot now needs the network + KMS.** KMS down, network
   partition, or targeted DoS ⇒ the sequencer can't unseal/boot. *Controls:* HA
   KMS, sensible retry, and a documented (manual, audited) break-glass path.
7. **Verifier correctness is critical code.** Cert-chain validation, signature
   checks, and field comparisons are classic footgun territory (skipped chain
   verification, accepting `debug`-policy reports, not checking VMPL/TCB).
   *Control:* use a vetted SNP verification library; test against known-bad
   reports.
8. **Where the KMS runs decides the trust anchor.** It must be operated by the
   **data owner** (or an independent party they trust), *not* by the same entity
   as the untrusted host — otherwise the trust is circular and B degrades toward
   A's posture.

### When is Option B acceptable?
Whenever the threat model must keep treating the host as untrusted (i.e., the
current model). B is the standard confidential-computing pattern for persistent
state across upgrades. Its risk is **concentrated and manageable** (protect the
KMS + curate the allow-list), versus Option A's risk which is **structural and
unmitigable** under the same model.

---

## 6. Side-by-side

| Property / adversary | Baseline (measurement) | Option A (family/image id) | Option B (attested KMS) |
|---|---|---|---|
| Forward-compat across versions | ❌ none | ✅ yes | ✅ yes (allow-list) |
| Confidentiality vs untrusted host (A1) | ✅ strong | ❌ **broken** (host forges ids) | ✅ strong (allow-list gate) |
| Anti state-substitution (A1) | ✅ strong | ❌ **broken** | ✅ strong |
| Co-tenant (A2) | ✅ | ⚠️ weak (collapses w/ host) | ✅ |
| Offline disk theft (A3) | ✅ | ✅ | ✅ |
| Remote attacker, no host access (A4) | ✅ | ✅ | ✅ (unless KMS reachable & weak) |
| New trusted component | none | none | **KMS + allow-list (A5/A6)** |
| Long-lived escrowed key | none | none | **yes (protect in HSM)** |
| Boot-time availability dependency | none | none | **network + KMS** |
| Rollback/version revocation | implicit (per-version key) | ❌ none | ⚠️ must be enforced as policy |
| Operational complexity | low | **lowest** | high |
| Consistent with current threat model | n/a (the problem) | ❌ **no** | ✅ yes |

---

## 7. Cross-cutting issues (apply to whichever is chosen)

- **Unmeasured runtime args / chain config.** Already a gap today; both options
  inherit it, and it becomes load-bearing under B (the KMS gates on a measurement
  that doesn't capture how Katana was invoked). Pinning security-relevant args
  into the measured cmdline is worth doing **regardless** of A vs B.
- **Confidentiality vs integrity are coupled** through the single LUKS+integrity
  key. Any confidentiality break (A) is also an integrity break. Keep that in
  mind if a future design splits these.
- **Key lifecycle.** Whatever path delivers the key, preserve the current
  hygiene: `Zeroizing` buffers, abort-on-panic so the key isn't unwound through
  destructors, FIFO hand-off, no key on argv/env
  (`snp-derivekey.rs:34,44-48,77`). Option B adds a transport key and a network
  buffer to the same discipline.
- **Forward-compat is still two-layered.** Even with A or B unlocking the disk,
  `katana-db` must still migrate the format forward (it does: v5–v9). The Layer-2
  CI guardrails from `db-forward-compatibility.md` remain necessary.

---

## 8. Recommendation

Under the project's stated, unrelaxed threat model (**host untrusted**):

1. **Do not adopt Option A** as a general solution. It trades the core at-rest
   guarantee against the primary adversary for operational convenience, and it
   cannot be hardened within the derived-key mechanism. Reserve it *only* for an
   explicitly single-tenant, trusted-operator deployment, clearly documented as a
   threat-model downgrade.
2. **Pursue Option B** if persistent state must survive upgrades while keeping the
   host untrusted. It is the only option that preserves "only attested code
   decrypts." Treat these as **prerequisite controls**, not enhancements:
   - HSM-backed key custody; signed, append-only, reviewed allow-list with an
     explicit **revocation/min-version** policy.
   - Fresh-nonce challenge + ephemeral transport key bound into `REPORT_DATA`;
     full AMD cert-chain + policy + TCB validation in a vetted verifier.
   - Pin security-relevant Katana args into the measured cmdline to close the
     unmeasured-args gap before the KMS is authoritative over real data.
   - KMS operated by the data owner (not the untrusted host); HA + audited
     break-glass for availability.
3. **Independently**, land the Layer-2 `katana-db` CI guardrails — cheap and
   orthogonal to the A/B decision.

The decision is fundamentally *where to put the trust anchor*: Option A pushes it
onto host-controlled launch values (untenable here); Option B relocates it to a
data-owner-controlled KMS + allow-list (tenable, with disciplined operations).

## 9. Decision (interim)

Pending a deliberate Option-B build-out, `start-vm.sh` now **boots unsealed by
default**; sealed storage is opt-in via `--sealed`. Rationale: Option A is off the
table under this threat model, Option B is real work, and an *honest* unsealed
default is clearer than a sealed default that (a) breaks across version upgrades
and (b) doesn't hold against the untrusted host it appears to defend against. The
sealed code path is retained and tested (`scripts/test-snp-e2e.sh`) for
deployments that have accepted the trade-offs. See the README subsection
"Storage sealing: why unsealed is the default". Revisit if/when Option B lands.

---

*Analysis only — reflects the code at time of writing. Primary references:
`snp-tools/src/bin/snp-derivekey.rs`, `misc/AMDSEV/start-vm.sh` (sev-snp-guest
object), `misc/AMDSEV/snp-tools/src/bin/snp-report.rs` (existing report path),
`misc/AMDSEV/README.md` (trust model, measurement), `db-forward-compatibility.md`.
SEV-SNP firmware behavior (ID-block author key not pinned by firmware; derived-key
field-select set) is stated as the basis for the Option A finding and should be
confirmed against the current AMD SEV-SNP ABI spec before implementation.*
