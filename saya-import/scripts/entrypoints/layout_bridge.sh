#!/bin/bash

set -e

if [ -z "$CAIRO_VERSION" ]; then
  echo "CAIRO_VERSION not set" >&2
  exit 1
fi

CAIRO_VERSION=${CAIRO_VERSION#"v"}

git clone --recursive https://github.com/starkware-libs/cairo-lang -b v$CAIRO_VERSION --depth 1 /src


# Patch to make the verifier work with `dynamic` layout
sed -i s/all_cairo/dynamic/g /src/src/starkware/cairo/cairo_verifier/layouts/all_cairo/cairo_verifier.cairo

cd /src/src && cairo-compile --no_debug_info --proof_mode --output /output/layout_bridge.json starkware/cairo/cairo_verifier/layouts/all_cairo/cairo_verifier.cairo
