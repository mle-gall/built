#!/usr/bin/env bash
# Build the Rust engine and wgpu renderer and install them in ./venv,
# the same way the release workflows do. Run again after any change to rust/.
set -euo pipefail
case "$#:${1:-}" in
    0:|1:--with-launcher) ;;
    *) echo "usage: $0 [--with-launcher]" >&2; exit 2 ;;
esac
cd "$(dirname "$0")/.."
# The venv is .venv on the Mac checkouts, ./venv in CI and older ones.
venv="${VENV:-$( [[ -d venv ]] && echo venv || echo .venv )}"
out="$(mktemp -d)"
trap 'rm -rf "$out"' EXIT
./$venv/bin/python -m pip install --quiet maturin
./$venv/bin/maturin build --release -m rust/built_core/Cargo.toml -o "$out"
./$venv/bin/python -m pip install --quiet --force-reinstall --no-deps "$out"/built_core-*.whl
# Preserve Rust's required symbols: rust-lang/rust#157750.
CARGO_PROFILE_RELEASE_STRIP=none ./$venv/bin/maturin build --release \
    --features extension-module -m rust/built_render/Cargo.toml -o "$out"
./$venv/bin/python -m pip install --quiet --force-reinstall --no-deps "$out"/built_render-*.whl
./$venv/bin/python -c "import built_render"
# Revit family reader: needs the rvt-rs checkout beside this repository
# (../rvt-rs, branch ingetrazo/rfa-family). Without it .rfa import reports
# the reader missing.
if [[ -d ../rvt-rs ]]; then
    ./$venv/bin/maturin build --release --features extension-module \
        -m rust/built_rfa/Cargo.toml -o "$out"
    ./$venv/bin/python -m pip install --quiet --force-reinstall --no-deps "$out"/built_rfa-*.whl
    ./$venv/bin/python -c "import built_rfa"
else
    echo "skipping built_rfa: no ../rvt-rs checkout" >&2
fi
BUILT_RUST=1 ./$venv/bin/python -m pytest -q tests/test_rust_engine_switch.py
if [[ "${1:-}" == --with-launcher ]]; then
    PYO3_PYTHON="$PWD/$venv/bin/python" cargo build --release --locked --manifest-path rust/built_app/Cargo.toml
fi
