# Environment detection.

UNAME := $(shell uname)
CAIRO_2_VERSION = 2.10.0
SCARB_VERSION = 2.10.1

# Usage is the default target for newcomers running `make`.
.PHONY: usage
usage:
	@echo "Usage:"
	@echo "    prepare-snos-test:         Prepare the tests environment."
	@echo "    run-tests:                 Run SNOS tests."
	@echo "    reset-tests:               Reset the test environment."

.PHONY: prepare-snos-test
prepare-snos-test:
	git submodule update --init --recursive
	cd tests/snos/snos && \
		./setup-scripts/setup-cairo.sh && \
		./setup-scripts/setup-tests.sh

.PHONY: run-tests
run-tests:
	cd tests/snos/snos && \
		source ./snos-env/bin/activate && \
		cargo test

.PHONY: reset-tests
reset-tests:
	./scripts/reset-tests.sh
