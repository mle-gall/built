# Development guide

## Setup

Built has three Rust crates and a Python layer:
- `rust/built_app`: the application;
- `rust/built_core` and `rust/built_render`: Python extension modules
  (`built_core`, `built_render`) that the app embeds and the tests import.

```bash
python3 -m venv .venv
.venv/bin/python -m pip install -r requirements.txt
VENV=.venv scripts/build_native.sh
cargo build --release --manifest-path rust/built_app/Cargo.toml
rust/built_app/target/release/built
```

`scripts/build_native.sh` builds both extensions in release mode, installs
them into the virtual environment, and runs a short engine check. Run it
again after any change under `rust/`. The optional .rfa family reader
(`rust/built_rfa`) is built when an `rvt-rs` checkout sits beside this
repository; without it, `.rfa` import reports the reader missing.

## Running tests

See [testing.md](testing.md) for the subsets, the markers and keeping tests
independent. A fix or a feature comes with its test, ideally one that fails
without the change. Tests that open a window run against
`rust/built_app/target/release/built`; build it first and set `BUILT_WINDOW_TESTS=1`.

## Style

- All code, comments and commit messages in English.
- UI strings localized via `i18n/`. Never hardcode user-facing text.

## Packaging

`packaging/build-macos-app.sh`, `packaging/build-appimage.sh` and
`installer/built.iss` make the release bundles; the release workflows under
`.github/workflows/` run them.

## Submitting changes

See [../CONTRIBUTING.md](../CONTRIBUTING.md).
