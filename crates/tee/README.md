# katana-tee

TEE (Trusted Execution Environment) attestation support for Katana.

## Overview

This crate provides abstractions for generating hardware-backed attestation quotes that cryptographically bind Katana's blockchain state to a TEE measurement. When running Katana inside a TEE, clients can request attestation quotes that prove the sequencer is executing within a secure, isolated environment.

## What is TEE?

A Trusted Execution Environment (TEE) is a secure area within a processor that guarantees code and data loaded inside are protected with respect to confidentiality and integrity. TEEs provide:

- **Isolation**: Code executing inside a TEE is isolated from the rest of the system, including the operating system and hypervisor
- **Attestation**: Hardware-backed proof that specific code is running inside a genuine TEE
- **Integrity**: Guarantee that the code and data have not been tampered with

## Supported Platforms

### AMD SEV-SNP

AMD Secure Encrypted Virtualization - Secure Nested Paging (SEV-SNP) is a hardware security feature available on AMD EPYC processors (3rd Gen or later). See the [References](#references) section for detailed documentation.

**Requirements:**
- AMD EPYC processor with SEV-SNP support
- Linux kernel with SEV-SNP guest support
- Access to `/dev/sev-guest` device

To enable SEV-SNP support, compile with the `snp` feature:

```toml
[dependencies]
katana-tee = { features = [ "snp" ], .. }
```

## Verifying Attestation Quotes

SEV-SNP attestation quotes can be verified using:

1. **AMD's Key Distribution Service (KDS)** - Fetches the certificate chain for verification
2. **sev-snp-measure** - Tool for measuring and verifying SEV-SNP guests
3. **Custom verification** - Parse the attestation report and verify the signature chain

The quote contains:
- Measurement of the guest VM
- User-provided report data (the Poseidon hash commitment)
- Hardware-signed attestation from AMD's security processor

## Feature Flags

| Feature | Description |
|---------|-------------|
| `snp` | Enables AMD SEV-SNP support via the `sev-snp` crate |

## Security Considerations

- TEE attestation only proves code is running in a TEE; it does not verify the correctness of the code itself
- The attestation binds to a specific state; verifiers should check the block number/hash is recent
- Quote generation requires hardware access; quote generation will return errors on unsupported platforms
- The 64-byte report data is a Poseidon hash of `H_poseidon(state_root, block_hash)`, padded with zeros

## Reproducible Builds

For TEE deployments, verifiable builds are essential. The release pipeline produces a reproducible TEE binary with signed attestation.

### Verify Build Attestation

```bash
# Download artifacts from workflow run
gh run download <run-id> --name tee-release-artifacts --repo dojoengine/katana
tar -xzf katana_v1.7.0_linux_amd64_tee.tar.gz
gh attestation verify ./katana-reproducible --repo dojoengine/katana
```

### Reproduce Locally

```bash
git clone https://github.com/dojoengine/katana.git && cd katana
git checkout v1.7.0
docker build -f reproducible.Dockerfile -t katana-verify .
docker create --name verify katana-verify
docker cp verify:/katana ./katana-local && docker rm verify
sha384sum ./katana-local  # Should match published hash
```

### Build Details

- **Base**: `rust:1.86.0-slim-bookworm` (pinned by digest)
- **Target**: `x86_64-unknown-linux-musl` (static linking)
- **Profile**: `performance` (fat LTO, single codegen unit)
- **SOURCE_DATE_EPOCH**: `1735689600` (2025-01-01 00:00:00 UTC)

## References

### AMD SEV-SNP

- [AMD Secure Encrypted Virtualization (SEV)](https://www.amd.com/en/developer/sev.html) - Official AMD SEV developer resources
- [AMD SEV-SNP White Paper](https://docs.amd.com/v/u/en-US/SEV-SNP-strengthening-vm-isolation-with-integrity-protection-and-more.pdf) - Technical overview of SEV-SNP architecture and security guarantees
- [AMD SEV Developer Guide](https://www.amd.com/content/dam/amd/en/documents/epyc-technical-docs/programmer-references/55766_SEV-KM_API_Specification.pdf) - API specification for SEV key management
- [Linux Kernel SEV Guest API](https://docs.kernel.org/virt/coco/sev-guest.html) - Documentation for `/dev/sev-guest` interface

### Automata Network SDK

- [Automata SEV-SNP SDK](https://github.com/automata-network/amd-sev-snp-attestation-sdk) - The Rust SDK used by this crate for SEV-SNP attestation
- [Automata Network Documentation](https://docs.ata.network/) - Automata's TEE and attestation documentation

### Running SEV-SNP VMs

- [Azure Confidential VMs](https://learn.microsoft.com/en-us/azure/confidential-computing/confidential-vm-overview) - AMD SEV-SNP powered confidential VMs on Azure
- [Google Cloud Confidential VMs](https://cloud.google.com/confidential-computing/confidential-vm/docs/confidential-vm-overview) - Confidential computing on Google Cloud

### General TEE Resources

- [Confidential Computing Consortium](https://confidentialcomputing.io/) - Industry consortium for TEE standards and resources
- [TEE Fundamentals (ARM)](https://developer.arm.com/documentation/102418/latest/) - General TEE concepts (ARM-focused but broadly applicable)
