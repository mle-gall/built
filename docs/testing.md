# Running the tests

A fresh virtual environment created with `python -m venv .venv` and
`.venv/bin/python -m pip install -r requirements.txt` installs neither
PySide6 nor shiboken6. Build the Rust extensions with
`VENV=.venv scripts/build_native.sh` before running the suite.

The suite runs in parallel with [pytest-xdist](https://pypi.org/project/pytest-xdist/)
(in `requirements.txt`). Give it the number of workers; everything else is set
by `tests/conftest.py`: file-level scheduling (`--dist loadfile`, so a file's
module fixtures are built once and its tests share one worker) and crashed
workers replaced (`--max-worker-restart 8`).

```bash
export HOME=$(mktemp -d) BUILT_SETTINGS_DIR=$(mktemp -d)   # never the real profile
python -m pytest -n 4 -m "not slow and not window" tests  # quick subset: while iterating
python -m pytest -n 8 -m "not slow and not perf" tests     # fast suite: before handing work back
# Full release suite with window tests. Keep timing budgets off the parallel runners.
export BUILT_WINDOW_TESTS=1
python -m pytest -n 8 -m "not perf" tests && python -m pytest -n 0 -m perf tests
python -m pytest tests/test_x.py::test_y      # one test, serially (no -n)
```

| Selection | Tests (2026-11-24) | What it leaves out |
|---|---|---|
| Quick subset, `-m "not slow and not window"` | about 7 500 of 14 200; 2.5 min at `-n 4` with the machine loaded, under 3 min quiet | the `window` files and the `slow` tests |
| Fast suite, `-m "not slow"` | about 12 000 | the `slow` tests |
| Full suite | all enabled tests | opt-in cases unless enabled |

The quick subset covers the document and its kernels (store, topology,
booleans, push/pull, snapping, inference), file formats (IGZ/Built, SKP,
IFC, DXF, STL and the rest), the Rust bridges, the chrome's data and view
models, tools driven without a window, and the Project tree. It leaves out
the files marked `window`: `tests/conftest.py` marks a file that builds an
application window (`MainWindow`, `ComposerWindow`, the shell's `Host`) or
launches the app. Run the quick subset while iterating, the tests of the
files you touched, and the fast suite before handing work back. Work on
windows, docks, menus or dialogs needs the fast suite.

Use at most `-n 8`: more workers than cores slows the run down. Without `-n` nothing
changes from a plain serial pytest run.

## Rendering with Blender

The default suite checks Blender job generation, render scripts, camera updates
and plugin controls. To run the five tests that launch Blender, put its
executable on `PATH` and run `BUILT_BLENDER_TESTS=1 python -m pytest -m blender`.
These tests are also marked `slow`. An opted-in run reports renderer crashes
as failures.

## The `slow` marker

`slow` marks tests whose cost is the point and which the fast suite leaves
to the full run:

- long fuzz sweeps: the default run takes a seeded sample of each sweep's
  cases (`tests/fuzz.py`) and `slow` runs the whole sweep. The sample comes
  from `BUILT_FUZZ_SEED` (`0` unless set; `random` draws one per run, the
  same in every worker). The header prints it; set it again to collect the
  same cases;
- real-size data, where the size is what is tested: the 20 000-placement
  composite in `test_file_io_parity.py` (`large-*`; the default run checks the
  same format-3 paths on a scaled composite with the large-document threshold
  lowered), thousands of faces in `test_move_copy_exact_mesh_1087.py`;
- external programs, such as a real Blender.

Mark a new test `slow` only when shrinking its data would make it say
nothing. Otherwise shrink it.

## Keeping tests independent

Under xdist many files run one after another in the same process, so a test
must leave process-wide state as it found it.

`tests/conftest.py` puts back, after every test:

- `os.environ` (`_process_environment`). The app writes it on purpose
  (`--python-engine` sets `BUILT_RUST=0`), which turned the engine off
  for every later test on a worker;
- the Paint tool's class state: its chosen material (`PaintTool.current_*`),
  recent materials and armed pipette;
- the scene used for dimension formatting by `core.units.bind_scene`;
- the drawing axes, the SKP parse tables and `core._native.native`.

Before collection, the controller and each worker set `HOME`,
`CFFIXED_USER_HOME` and `BUILT_SETTINGS_DIR` to private directories.
Each test module gets an empty native settings store. Tests that need to
reopen preferences in their own directory can use `private_settings`.
A change to the real macOS Built preferences plist's mtime fails the
session, including when the suite runs in batches.

The leak guard checks environment changes left behind by a test. Restore
them with `monkeypatch` or a `finally` block. Shell fixtures restore the
process-wide services they install. `BUILT_LEAK_GUARD=0` disables the
per-test check. The session's plist check runs independently.

The guard does not cover the class and module state the fixtures put back
(the Paint tool, `core.units`, the drawing axes). For a new kind of leak,
compare a fingerprint of process state before and after each test across an
xdist run and look at what changed just before the failing test on its
worker.
- Use `monkeypatch` for module globals. A `monkeypatch.delenv` or `setattr`
  done after the code under test has written the value records that value
  and puts it back at teardown.
- Bound a cost with what the operation does (a call that must not happen, a
  count) or with `time.process_time()`, never with wall time: under eight
  workers wall time measures the machine.

## Shared fixtures

- `tests/composite_fixture.py` builds the linked-files composite. Build it
  once per module (`scope="module"`), at the size the test needs: the
  Project tree tests use 2 000 placements for what the tree shows and 20 000
  only for the three tests that bound its cost.

## Built CI

Pushes to `main` and pull requests run the read-only Built workflow. A newer
run on the same branch cancels the earlier run. Cargo downloads and pip
packages are cached by runner platform. Tests use temporary homes and settings.

| Runner | Checks |
| --- | --- |
| macOS arm64 | Build the three release wheels and launcher. Run the full pytest suite with `-n auto --dist loadfile -m "not perf"`, then serial performance budgets and the Rust library tests. Build a local app bundle and run its executable with `--check`. |
| Ubuntu | Install GTK, Wayland and Mesa development/runtime packages, build the core and render wheels, run the `not slow and not window` tier and Rust library tests, and check the launcher with Cargo. |
| Windows x86-64 | Type-check all five crate manifests with the MSVC target and the runner's Python. |

The macOS window probe checks for a console session and a Metal device. If
either is missing, pytest reports the window tier as skipped with the reason.
Blender keeps its usual separate opt-in. The bundle uses the packager's local
ad-hoc signature for executable integrity. CI uses no signing identity and
publishes no release artifacts.

The RFA crate expects a sibling `rvt-rs` checkout. CI checks out public
revision `62cc17e0d8bc8781cfab482ca50e8216e0a3ce1d` and applies
`.github/ci/rfa-family.patch`, the saved-graphics style change from commit
`89cbaf5b89d155a9313bfe3516dce8a3b30c511b` on the local
`ingetrazo/rfa-family` branch. Both macOS and Windows use this source.
