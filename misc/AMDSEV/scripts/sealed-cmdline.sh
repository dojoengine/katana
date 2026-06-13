#!/bin/bash
# Single source of truth for the sealed-storage kernel cmdline.
#
# This string is part of the SEV-SNP launch measurement. Verifiers must
# reproduce it byte-for-byte to recompute the published measurement, so
# the format MUST NOT be inlined anywhere — always source this file and
# call build_sealed_cmdline.
#
# Consumers:
#   - start-vm.sh                       (at boot, with operator's LUKS_UUID)
#   - .github/workflows/amdsev-release.yml  (at release time, with KATANA_CANONICAL_LUKS_UUID
#                                        from build-config, to compute the published digest)
#   - verify-build.sh                   (at verification time, to recompute and assert)

build_sealed_cmdline() {
    local uuid="$1"
    printf 'console=ttyS0 KATANA_EXPECTED_LUKS_UUID=%s' "$uuid"
}
