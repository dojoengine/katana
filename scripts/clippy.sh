#!/bin/bash

# Tells the shell to exit immediately if a command exits with a non-zero status
set -e
# Enables tracing of the commands as they are executed, showing the commands and their arguments
set -x
# Causes a pipeline to return a failure status if any command in the pipeline fails
set -o pipefail

run_clippy() {
  cargo +nightly-2025-09-18 clippy --all-targets "$@" -- -D warnings -D future-incompatible -D nonstandard-style -D rust-2018-idioms -D unused -D missing-debug-implementations
}

# The katana-dev CI image pre-bakes clippy only for its default nightly; this script pins a
# specific nightly (>=1.92.0-nightly, required by the rustc 1.91.1 MSRV of the SP1 v6 deps),
# so make sure that toolchain + the clippy component are present before invoking it.
rustup toolchain install nightly-2025-09-18 --component clippy --no-self-update

. ./scripts/cairo-native.env.sh && run_clippy -p katana --all-features
