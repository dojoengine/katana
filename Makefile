# Environment detection.

UNAME := $(shell uname)

# Usage is the default target for newcomers running `make`.
.PHONY: usage
usage:
	@echo "Usage:"
	@echo "    prepare-snos-test:         Prepare the tests environment."
	@echo "    extract-test-db:           Extract the test database file."
	@echo "    native-deps-macos:         Install cairo-native dependencies for macOS."
	@echo "    native-deps-linux:         Install cairo-native dependencies for Linux."

.PHONY: prepare-snos-test
prepare-snos-test: extract-test-db
	git submodule update --init --recursive
	cd tests/snos/snos && \
		source ./setup-scripts/setup-cairo.sh && \
		source ./setup-scripts/setup-tests.sh

.PHONY: extract-test-db
extract-test-db:
	cd tests/fixtures && \
		tar -xzf katana_db.tar.gz

.PHONY: check-llvm
check-llvm:
ifndef MLIR_SYS_190_PREFIX
	$(error Could not find a suitable LLVM 19 toolchain (mlir), please set MLIR_SYS_190_PREFIX env pointing to the LLVM 19 dir)
endif
ifndef TABLEGEN_190_PREFIX
	$(error Could not find a suitable LLVM 19 toolchain (tablegen), please set TABLEGEN_190_PREFIX env pointing to the LLVM 19 dir)
endif
	@echo "LLVM is correctly set at $(MLIR_SYS_190_PREFIX)."

.PHONY: native-deps
native-deps:
ifeq ($(UNAME), Darwin)
native-deps: native-deps-macos
else ifeq ($(UNAME), Linux)
native-deps: native-deps-linux
endif
	@echo "Run  \`source scripts/cairo-native.env.sh\` to setup the needed environment variables for cairo-native."

.PHONY: native-deps-macos
native-deps-macos:
	-brew install llvm@19 --quiet

.PHONY: native-deps-linux
native-deps-linux:
	sudo apt-get install llvm-19 llvm-19-dev llvm-19-runtime clang-19 clang-tools-19 lld-19 libpolly-19-dev libmlir-19-dev mlir-19-tools
