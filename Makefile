ifeq ($(OS),Windows_NT)
	UNAME := Windows
else
	UNAME := $(shell uname)
endif

EXPLORER_UI_DIR ?= crates/explorer/ui/src
EXPLORER_UI_DIST ?= crates/explorer/ui/dist

SNOS_OUTPUT ?= tests/snos/snos/build/
FIXTURES_DIR ?= tests/fixtures
DB_FIXTURES_DIR ?= $(FIXTURES_DIR)/db

SNOS_DB_TAR ?= $(DB_FIXTURES_DIR)/snos.tar.gz
SNOS_DB_DIR := $(DB_FIXTURES_DIR)/snos

COMPATIBILITY_DB_TAR ?= $(DB_FIXTURES_DIR)/1_6_0.tar.gz
COMPATIBILITY_DB_DIR ?= $(DB_FIXTURES_DIR)/1_6_0

CONTRACTS_CRATE := crates/contracts
CONTRACTS_DIR := $(CONTRACTS_CRATE)/contracts
CONTRACTS_BUILD_DIR := $(CONTRACTS_CRATE)/build

# The `scarb` version that is required to compile the feature contracts in katana-contracts
SCARB_VERSION := 2.8.4

# Detect LLVM paths based on OS
ifeq ($(UNAME), Darwin)
	LLVM_PREFIX := $(shell brew --prefix llvm@19 2>/dev/null || echo "/opt/homebrew/opt/llvm@19")
else ifeq ($(UNAME), Linux)
	LLVM_PREFIX := /usr/lib/llvm-19
else
	LLVM_PREFIX :=
endif

# Environment for cargo builds
CARGO_ENV := MLIR_SYS_190_PREFIX=$(LLVM_PREFIX) LLVM_SYS_191_PREFIX=$(LLVM_PREFIX) TABLEGEN_190_PREFIX=$(LLVM_PREFIX)

# Ensure scarb is in PATH (add common installation locations)
SCARB_PATH := $(HOME)/.local/bin:$(HOME)/.cargo/bin:$(PATH)

.DEFAULT_GOAL := usage
.SILENT: clean
.PHONY: usage help check-llvm native-deps native-deps-macos native-deps-linux native-deps-windows build-explorer contracts clean deps install-scarb test-artifacts snos-artifacts db-compat-artifacts install-protoc test-deps all

usage help:
	@echo "=========================================="
	@echo "Katana Build System - Dependency Setup"
	@echo "=========================================="
	@echo ""
	@echo "Main Commands:"
	@echo "    all                        Install ALL dependencies and build required artifacts."
	@echo "    deps                       Install system dependencies (Scarb, LLVM)."
	@echo "    test-deps                  Install dependencies for running tests (includes protoc)."
	@echo ""
	@echo "Artifact Building:"
	@echo "    contracts                  Build the contracts (required before cargo build)."
	@echo "    build-explorer             Build the explorer UI."
	@echo "    test-artifacts             Prepare test artifacts (including test database)."
	@echo ""
	@echo "Individual Dependency Installation:"
	@echo "    install-scarb              Install Scarb (Cairo package manager)."
	@echo "    install-protoc             Install Protocol Buffers compiler."
	@echo "    native-deps-macos          Install LLVM 19 for macOS."
	@echo "    native-deps-linux          Install LLVM 19 for Linux."
	@echo "    native-deps-windows        Install LLVM 19 for Windows."
	@echo ""
	@echo "Other Commands:"
	@echo "    check-llvm                 Check if LLVM is properly configured."
	@echo "    clean                      Clean up generated files and artifacts."
	@echo "    help                       Show this help message."
	@echo ""
	@echo "=========================================="
	@echo "Quick Start:"
	@echo "=========================================="
	@echo "  1. Install all dependencies:"
	@echo "     make all"
	@echo ""
	@echo "  2. Setup environment variables:"
	@echo "     source scripts/cairo-native.env.sh"
	@echo ""
	@echo "  3. Build Katana:"
	@echo "     cargo build"
	@echo "=========================================="

all: deps contracts
	@echo ""
	@echo "=========================================="
	@echo "✓ All dependencies installed successfully!"
	@echo "=========================================="
	@echo ""
	@echo "Dependencies installed:"
	@echo "  ✓ Scarb $(SCARB_VERSION)"
	@echo "  ✓ LLVM 19"
	@echo "  ✓ Contracts built"
	@echo ""
	@echo "Next steps:"
	@echo "  1. Setup environment variables:"
	@echo "     source scripts/cairo-native.env.sh"
	@echo ""
	@echo "  2. Build Katana:"
	@echo "     cargo build"
	@echo ""
	@echo "  3. (Optional) For release build:"
	@echo "     cargo build --release"
	@echo ""
	@echo "  4. (Optional) For tests:"
	@echo "     make test-deps && cargo build --tests"
	@echo "=========================================="

deps: install-scarb native-deps
	@echo "✓ System dependencies installed successfully."

install-protoc:
	@echo "Checking for protoc..."
	@if which protoc > /dev/null 2>&1; then \
		echo "protoc is already installed: $$(protoc --version)"; \
	else \
		echo "Installing protoc..."; \
		if [ "$(UNAME)" = "Linux" ]; then \
			sudo apt-get update && sudo apt-get install -y protobuf-compiler; \
		elif [ "$(UNAME)" = "Darwin" ]; then \
			brew install protobuf; \
		else \
			echo "Please install protoc manually from https://github.com/protocolbuffers/protobuf/releases"; \
			exit 1; \
		fi; \
	fi

test-deps: deps install-protoc
	@echo ""
	@echo "Test dependencies installed. Note: SNOS tests also require Python 3 and pyenv."
	@echo "To install pyenv, visit: https://github.com/pyenv/pyenv#installation"

install-scarb:
	@if scarb --version 2>/dev/null | grep -q "^scarb $(SCARB_VERSION)"; then \
		echo "scarb $(SCARB_VERSION) is already installed."; \
	else \
		echo "Installing scarb $(SCARB_VERSION)..."; \
		curl --proto '=https' --tlsv1.2 -sSf https://docs.swmansion.com/scarb/install.sh | sh -s -- -v $(SCARB_VERSION) || { echo "Failed to install scarb!"; exit 1; }; \
		echo "scarb $(SCARB_VERSION) installed successfully."; \
	fi

snos-artifacts: $(SNOS_OUTPUT)
	@echo "SNOS test artifacts prepared successfully."

db-compat-artifacts: $(COMPATIBILITY_DB_DIR)
	@echo "Database compatibility test artifacts prepared successfully."

test-artifacts: $(SNOS_DB_DIR) $(SNOS_OUTPUT) $(COMPATIBILITY_DB_DIR) contracts
	@echo "All test artifacts prepared successfully."

build-explorer:
	@which bun >/dev/null 2>&1 || { echo "Error: bun is required but not installed. Please install bun first."; exit 1; }
	@$(MAKE) $(EXPLORER_UI_DIST)

contracts: $(CONTRACTS_BUILD_DIR)

# Generate the list of sources dynamically to make sure Make can track all files in all nested subdirs
$(CONTRACTS_BUILD_DIR): $(shell find $(CONTRACTS_DIR) -type f)
	@echo "Building contracts..."
	@PATH="$(SCARB_PATH)" cd $(CONTRACTS_DIR) && scarb build
	@mkdir -p $(CONTRACTS_BUILD_DIR) && \
		mv $(CONTRACTS_DIR)/target/dev/* $(CONTRACTS_BUILD_DIR)/ || { echo "Contracts build failed!"; exit 1; }

$(EXPLORER_UI_DIR):
	@echo "Initializing Explorer UI submodule..."
	@git submodule update --init --recursive --force crates/explorer/ui

$(EXPLORER_UI_DIST): $(EXPLORER_UI_DIR)
	@echo "Building Explorer..."
	@cd crates/explorer/ui && \
		bun install && \
		bun run build || { echo "Explorer build failed!"; exit 1; }
	@echo "Explorer build complete."

$(SNOS_OUTPUT): $(SNOS_DB_DIR)
	@echo "Initializing submodules..."
	@git submodule update --init --recursive
	@echo "Setting up SNOS tests..."
	@cd tests/snos/snos && \
		. ./setup-scripts/setup-cairo.sh && \
		. ./setup-scripts/setup-tests.sh || { echo "SNOS setup failed\!"; exit 1; }

$(SNOS_DB_DIR): $(SNOS_DB_TAR)
	@echo "Extracting SNOS test database..."
	@cd $(DB_FIXTURES_DIR) && \
		tar -xzf snos.tar.gz || { echo "Failed to extract SNOS test database\!"; exit 1; }
	@echo "SNOS test database extracted successfully."

$(COMPATIBILITY_DB_DIR): $(COMPATIBILITY_DB_TAR)
	@echo "Extracting backward compatibility test database..."
	@cd $(DB_FIXTURES_DIR) && \
		tar -xzf $(notdir $(COMPATIBILITY_DB_TAR)) && \
		mv katana_db $(notdir $(COMPATIBILITY_DB_DIR)) || { echo "Failed to extract backward compatibility test database\!"; exit 1; }
	@echo "Backward compatibility database extracted successfully."

check-llvm:
ifndef MLIR_SYS_190_PREFIX
	$(error Could not find a suitable LLVM 19 toolchain (mlir), please set MLIR_SYS_190_PREFIX env pointing to the LLVM 19 dir)
endif
ifndef TABLEGEN_190_PREFIX
	$(error Could not find a suitable LLVM 19 toolchain (tablegen), please set TABLEGEN_190_PREFIX env pointing to the LLVM 19 dir)
endif
	@echo "LLVM is correctly set at $(MLIR_SYS_190_PREFIX)."

native-deps:
ifeq ($(UNAME), Darwin)
native-deps: native-deps-macos
else ifeq ($(UNAME), Linux)
native-deps: native-deps-linux
else ifeq ($(UNAME), Windows)
native-deps: native-deps-windows
endif
	@echo "Run  \`source scripts/cairo-native.env.sh\` to setup the needed environment variables for cairo-native."

native-deps-macos:
	@echo "Installing LLVM dependencies for macOS..."
	-brew install llvm@19 --quiet
	@echo "macOS dependencies installed successfully."

native-deps-linux:
	@echo "Installing LLVM dependencies for Linux..."
	sudo apt-get install -y llvm-19 llvm-19-dev llvm-19-runtime clang-19 clang-tools-19 lld-19 libpolly-19-dev libmlir-19-dev mlir-19-tools
	@echo "Linux dependencies installed successfully."

native-deps-windows:
	@echo "Installing LLVM dependencies for Windows..."
	@where choco >nul 2>&1 || { echo "Error: Chocolatey is required but not installed. Please install Chocolatey first: https://chocolatey.org/install"; exit 1; }
	choco install llvm --version 19.1.7 -y
	@echo "Windows dependencies installed successfully."

clean:
	echo "Cleaning up generated files..."
	-rm -rf $(SNOS_DB_DIR) $(COMPATIBILITY_DB_DIR) $(SNOS_OUTPUT) $(EXPLORER_UI_DIST) $(CONTRACTS_BUILD_DIR)
	echo "Clean complete."
