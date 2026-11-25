# Contributing to Built

Thank you for your interest in Built! Contributions of any kind are welcome:
code, documentation, translations, bug reports and design feedback.

## Development setup

Built is a native Rust application on wgpu, with Python as its scripting and
plugin layer. You need a Rust toolchain (stable, through `rustup`) and
Python 3.12 or later.

```bash
git clone https://github.com/<your-user>/built.git
cd built
python3 -m venv .venv
.venv/bin/python -m pip install -r requirements.txt
VENV=.venv scripts/build_native.sh          # the Rust core and renderer, into .venv
cargo build --release --manifest-path rust/built_app/Cargo.toml
rust/built_app/target/release/built         # run the app
```

Run `scripts/build_native.sh` again after any change under `rust/`.
[docs/development.md](docs/development.md) has the details.

## Tests and CI

```bash
export HOME=$(mktemp -d) BUILT_SETTINGS_DIR=$(mktemp -d)   # never your real profile
.venv/bin/python -m pytest -n 4 -m "not slow and not window" tests   # quick subset
.venv/bin/python -m pytest -n 8 -m "not slow" tests                  # fast suite
```

[docs/testing.md](docs/testing.md) explains the subsets, the `slow` and
`window` markers and how to keep tests independent. Every pull request runs
the CI checks. New features come with tests, and bug fixes come with a
regression test that fails without the fix.

## Code style

- **Language**: all code, comments, commit messages and pull request
  descriptions in **English**.
- **Rust**: `cargo fmt` and no new warnings.
- **Python**: PEP 8 with a soft 100-character line limit; type hints and
  short docstrings for public APIs.
- **UI strings** go through `i18n/` (`i18n/en.json` is the source). Never
  hardcode user-facing text.

## Folder layout

See [docs/architecture.md](docs/architecture.md) for a tour. Briefly:

- `rust/built_app/`: the application (window, menus, event loop).
- `rust/built_core/`: the document, the geometry kernel, the tools, the
  commands and the file formats.
- `rust/built_render/`: the wgpu renderer and the egui chrome.
- `core/`, `formats/`, `plugins/`, `views/`: the Python layer (IFC, records,
  the plugin API, the bundled plugins).
- `resources/`, `i18n/`: icons, textures, components and translations.
- `packaging/`, `installer/`: release bundles for macOS, Linux and Windows.
- `scripts/`: build, release and resource tools.
- `docs/`: user, format and contributor documentation.
- `tests/`: the automated tests (`pytest`, plus `cargo test` in each crate).

## Writing a plugin

The lowest-friction way to extend Built, with no fork needed. Drop a Python
file in your user plugins folder and its tools appear in the **Extensions**
menu. The contract is in [docs/plugins.md](docs/plugins.md). The plugin API is
**not stable yet**: plugins you publish today may need updates between
versions.

## Workflow

1. **Fork** the repository.
2. Create a feature branch: `git checkout -b feature/short-description`.
3. Commit small, focused changes with descriptive messages.
4. Run the fast suite locally.
5. Push to your fork and open a **pull request**. CI runs on it.
6. A maintainer will review it.

## Issues

- **Bug report**: the steps to reproduce, what you expected and what
  happened, with screenshots when relevant.
- **Feature request**: what you want, why it matters, and the tools that do
  it today.

## Licensing of contributions

By submitting a contribution you agree it will be licensed under
**GPL-3.0-or-later** (the project licence).
