#!/usr/bin/env bash
# Install the native executable alongside PyInstaller's collected runtime.
set -euo pipefail
dest=${1:?bundle directory required}
target=${CARGO_TARGET_DIR:-rust/built_app/target}
case "$(uname -s)" in
    Linux)
        export RUSTFLAGS="${RUSTFLAGS:-} -C link-arg=-Wl,-rpath,\$ORIGIN/_internal"
        suffix=''
        ;;
    MINGW*|MSYS*)
        suffix=.exe
        cp "$dest"/_internal/python3*.dll "$dest/"
        ;;
    *) exit 2 ;;
esac
cargo build --locked --release --target-dir "$target" --manifest-path rust/built_app/Cargo.toml
install -m 755 "$target/release/built$suffix" "$dest/built$suffix"
rm "$dest/ingetrazo$suffix"
