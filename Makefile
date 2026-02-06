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

VRF_DIR := $(CONTRACTS_DIR)/vrf
AVNU_DIR := $(CONTRACTS_DIR)/avnu/contracts

# The scarb version required by the AVNU contracts (no .tool-versions in that directory)
AVNU_SCARB_VERSION := 2.11.4

# The `scarb` version that is required to compile the feature contracts in katana-contracts
SCARB_VERSION := 2.8.4

.DEFAULT_GOAL := usage
.SILENT: clean
.PHONY: usage help check-llvm native-deps native-deps-macos native-deps-linux native-deps-windows build-explorer contracts clean deps install-scarb test-artifacts snos-artifacts db-compat-artifacts install-pyenv

usage help:
	@echo "Usage:"
	@echo "    deps:                      Install all required dependencies for building Katana with all features (incl. tests)."
	@echo "    snos-deps:                 Install SNOS test dependencies (pyenv, Python 3.9.15)."
	@echo "    build-explorer:            Build the explorer."
	@echo "    contracts:                 Build the contracts."
	@echo "    test-artifacts:            Prepare tests artifacts (including test database)."
	@echo "    snos-artifacts:            Prepare SNOS tests artifacts."
	@echo "    db-compat-artifacts:       Prepare database compatibility test artifacts."
	@echo "    native-deps-macos:         Install cairo-native dependencies for macOS."
	@echo "    native-deps-linux:         Install cairo-native dependencies for Linux."
	@echo "    native-deps-windows:       Install cairo-native dependencies for Windows."
	@echo "    check-llvm:                Check if LLVM is properly configured."
	@echo "    clean:                     Clean up generated files and artifacts."
	@echo "    help:                      Show this help message."

deps: install-scarb native-deps snos-deps
	@echo "All dependencies installed successfully."

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
	@mkdir -p $@
	@echo "Building main contracts..."
	@cd $(CONTRACTS_DIR) && asdf exec scarb build || { echo "Main contracts build failed!"; exit 1; }
	@cp $(CONTRACTS_DIR)/target/dev/* $@
	@echo "Building VRF contracts..."
	@cd $(VRF_DIR) && asdf exec scarb build || { echo "VRF contracts build failed!"; exit 1; }
	@cp $(VRF_DIR)/target/dev/* $@
	@echo "Building AVNU contracts..."
	@cd $(AVNU_DIR) && ASDF_SCARB_VERSION=$(AVNU_SCARB_VERSION) asdf exec scarb build || { echo "AVNU contracts build failed!"; exit 1; }
	@cp $(AVNU_DIR)/target/dev/* $@

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
		PIP_DEFAULT_TIMEOUT=120 PIP_RETRIES=5 . ./setup-scripts/setup-cairo.sh && \
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

install-pyenv:
	@if command -v pyenv >/dev/null 2>&1; then \
		echo "pyenv is already installed."; \
	else \
		echo "Installing pyenv..."; \
		curl https://pyenv.run | bash || { echo "Failed to install pyenv!"; exit 1; }; \
		echo "pyenv installed successfully."; \
		echo "NOTE: Add the following to your shell profile (~/.bashrc or ~/.zshrc):"; \
		echo '  export PYENV_ROOT="$$HOME/.pyenv"'; \
		echo '  command -v pyenv >/dev/null || export PATH="$$PYENV_ROOT/bin:$$PATH"'; \
		echo '  eval "$$(pyenv init -)"'; \
	fi

snos-deps:
ifeq ($(UNAME), Darwin)
snos-deps: snos-deps-macos
else ifeq ($(UNAME), Linux)
snos-deps: snos-deps-linux
endif

snos-deps-linux: install-pyenv
	@echo "Installing Python build dependencies for Linux..."
	sudo apt-get update
	sudo apt-get install -y make build-essential libssl-dev libgmp-dev libbz2-dev libreadline-dev libsqlite3-dev liblzma-dev zlib1g-dev
	@echo "Linux SNOS dependencies installed successfully."
	@echo "NOTE: You may need to restart your shell or run 'source ~/.bashrc' before using pyenv."

snos-deps-macos: install-pyenv
	@echo "Installing Python build dependencies for macOS..."
	-brew install openssl readline sqlite3 zlib --quiet
	@echo "macOS SNOS dependencies installed successfully."
	@echo "NOTE: You may need to restart your shell or run 'source ~/.zshrc' before using pyenv."

clean:
	echo "Cleaning up generated files..."
	-rm -rf $(SNOS_DB_DIR) $(COMPATIBILITY_DB_DIR) $(SNOS_OUTPUT) $(EXPLORER_UI_DIST) $(CONTRACTS_BUILD_DIR)
	echo "Clean complete."
