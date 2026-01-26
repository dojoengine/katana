#!/usr/bin/env bash
#
# gcp-setup-instance.sh - Setup GCP instance for SEV-SNP testing
#
# This script installs all necessary dependencies on the GCP Confidential VM:
# - QEMU with SEV-SNP support
# - KVM kernel modules
# - Build tools and utilities
# - Python and sev-snp-measure
#
# Run this on the GCP instance after creation.
#

set -euo pipefail

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log_info() { echo -e "${BLUE}[INFO]${NC} $*"; }
log_success() { echo -e "${GREEN}[SUCCESS]${NC} $*"; }
log_warning() { echo -e "${YELLOW}[WARNING]${NC} $*"; }
log_error() { echo -e "${RED}[ERROR]${NC} $*"; }

check_sev_snp() {
    log_info "Checking AMD SEV-SNP support..."

    if [ ! -f /sys/firmware/efi/efivars/ConfidentialComputing-* ] 2>/dev/null; then
        log_warning "Not running in a Confidential VM"
    else
        log_success "Running in a Confidential VM"
    fi

    # Check if nested virtualization is enabled
    if [ ! -e /dev/kvm ]; then
        log_error "/dev/kvm not found - nested virtualization may not be enabled"
        log_info "Instance must be created with --enable-nested-virtualization"
        exit 1
    fi

    log_success "KVM is available"

    # Check for AMD CPU
    if grep -q "AMD" /proc/cpuinfo; then
        log_success "AMD CPU detected"
    else
        log_warning "Non-AMD CPU detected - SEV-SNP may not be available"
    fi
}

install_dependencies() {
    log_info "Installing dependencies..."

    export DEBIAN_FRONTEND=noninteractive

    sudo apt-get update -qq

    # Install QEMU and KVM
    log_info "Installing QEMU and KVM..."
    sudo apt-get install -y -qq \
        qemu-system-x86 \
        qemu-utils \
        qemu-kvm \
        libvirt-daemon-system \
        libvirt-clients \
        bridge-utils

    # Install build tools
    log_info "Installing build tools..."
    sudo apt-get install -y -qq \
        build-essential \
        git \
        curl \
        wget \
        jq \
        xxd \
        python3 \
        python3-pip \
        python3-venv

    # Install pipx for isolated tool installation
    log_info "Installing pipx..."
    sudo apt-get install -y -qq pipx
    pipx ensurepath

    # Install sev-snp-measure
    log_info "Installing sev-snp-measure..."
    pipx install sev-snp-measure

    log_success "Dependencies installed"
}

configure_kvm() {
    log_info "Configuring KVM permissions..."

    # Add user to kvm group
    sudo usermod -aG kvm "$USER" || true

    # Set permissions on /dev/kvm
    sudo chmod 666 /dev/kvm || true

    log_success "KVM configured"
}

check_qemu_version() {
    log_info "Checking QEMU version..."

    local qemu_version
    qemu_version=$(qemu-system-x86_64 --version | head -1)

    log_info "QEMU: $qemu_version"

    # Check for SEV support
    if qemu-system-x86_64 -machine help | grep -q "confidential-guest-support"; then
        log_success "QEMU supports confidential guests"
    else
        log_warning "QEMU may not support SEV-SNP (version too old?)"
    fi
}

create_workspace() {
    log_info "Creating workspace directory..."

    mkdir -p ~/katana-sev-test
    cd ~/katana-sev-test

    log_success "Workspace created at ~/katana-sev-test"
}

show_status() {
    echo ""
    echo "==========================================="
    log_success "Instance Setup Complete"
    echo "==========================================="
    echo ""
    echo "System Information:"
    echo "  CPU: $(grep -m1 "model name" /proc/cpuinfo | cut -d: -f2 | xargs)"
    echo "  Kernel: $(uname -r)"
    echo "  QEMU: $(qemu-system-x86_64 --version | head -1 | cut -d' ' -f3)"
    echo "  KVM: $([ -e /dev/kvm ] && echo "Available" || echo "Not available")"
    echo ""
    echo "Next steps:"
    echo "  1. Upload boot components from your local machine:"
    echo "     ./tee/scripts/gcp-upload-components.sh"
    echo ""
    echo "  2. Launch QEMU with SEV-SNP (on this instance):"
    echo "     cd ~/katana-sev-test"
    echo "     ./launch-qemu.sh"
    echo ""
    echo "  3. Verify attestation (on this instance):"
    echo "     ./verify-attestation.sh"
    echo ""
    echo "Note: Log out and back in for group changes to take effect"
    echo "      Or run: newgrp kvm"
    echo ""
}

main() {
    echo "==========================================="
    echo "GCP Instance Setup for SEV-SNP Testing"
    echo "==========================================="
    echo ""

    check_sev_snp
    install_dependencies
    configure_kvm
    check_qemu_version
    create_workspace
    show_status

    log_success "Setup complete!"
}

main "$@"
