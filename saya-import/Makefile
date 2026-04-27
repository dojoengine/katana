SUBMODULE_DIR := piltover

# Scarb version is read from the submodule's .tool-versions — single source of
# truth, no drift between saya's Makefile and the piltover pin.
SCARB_VERSION := $(shell if [ -f $(SUBMODULE_DIR)/.tool-versions ]; then \
                            awk '$$1 == "scarb" { print $$2 }' $(SUBMODULE_DIR)/.tool-versions; \
                        fi)

.PHONY: contracts install-scarb

install-scarb:
	@command -v asdf >/dev/null 2>&1 || { \
		echo "Error: asdf is required. Install from https://asdf-vm.com/"; exit 1; }
	@if [ ! -f $(SUBMODULE_DIR)/.tool-versions ]; then \
		echo "Initializing piltover submodule first (to read scarb version)..."; \
		git submodule update --init --recursive --force $(SUBMODULE_DIR); \
	fi
	@if [ -z "$(SCARB_VERSION)" ]; then \
		echo "Error: could not read scarb version from $(SUBMODULE_DIR)/.tool-versions"; \
		exit 1; \
	fi
	@asdf plugin list 2>/dev/null | grep -qx scarb || asdf plugin add scarb
	@asdf where scarb $(SCARB_VERSION) >/dev/null 2>&1 || asdf install scarb $(SCARB_VERSION)
	@echo "scarb $(SCARB_VERSION) ready (pinned by $(SUBMODULE_DIR)/.tool-versions)."

# Diagnostic / explicit-refresh target. `cargo build -p saya-ops` already runs
# scarb via bin/ops/build.rs — this exists for cases where you want to warm
# the scarb cache or verify contracts compile before a full cargo build.
contracts: install-scarb
	@cd $(SUBMODULE_DIR) && asdf exec scarb build
	@echo "Piltover contracts built in $(SUBMODULE_DIR)/target/dev/"
	@echo "Note: bin/ops/build.rs re-runs scarb at cargo build time; nothing"
	@echo "needs to be copied manually."
