#!/bin/bash

cargo clippy --all --all-targets --all-features -- -D warnings

for workspace in bin/persistent bin/ops bin/persistent-tee; do
    (cd "$workspace" && cargo clippy --all-targets --all-features -- -D warnings)
done
