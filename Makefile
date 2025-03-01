# Environment detection.

UNAME := $(shell uname)

# Usage is the default target for newcomers running `make`.
.PHONY: usage
usage:
	@echo "Usage:"
	@echo "    prepare-snos-test:         Prepare the tests environment."
	@echo "    extract-test-db:           Extract the test database file."

.PHONY: prepare-snos-test
prepare-snos-test: extract-test-db
	git submodule update --init --recursive
	cd tests/snos/snos && \
		./setup-scripts/setup-cairo.sh && \
		./setup-scripts/setup-tests.sh

.PHONY: extract-test-db
extract-test-db:
	cd tests/fixtures && \
		tar -xzf katana_db.tar.gz
