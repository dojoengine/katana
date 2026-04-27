#!/bin/bash

if [[ " $* " =~ " --fix " ]]; then
    prettier --write "**/*.md"
    prettier --write "**/*.{yaml,yml}"
    cargo fmt --all
    for workspace in bin/persistent bin/ops bin/persistent-tee; do
        (cd "$workspace" && cargo fmt --all)
    done
else
    prettier --check "**/*.md"
    prettier --check "**/*.{yaml,yml}"
    cargo fmt --all --check
    for workspace in bin/persistent bin/ops bin/persistent-tee; do
        (cd "$workspace" && cargo fmt --all --check)
    done
fi
