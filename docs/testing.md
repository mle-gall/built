# Running the tests

The suite runs in parallel with [pytest-xdist](https://pypi.org/project/pytest-xdist/)
(in `requirements.txt`). Give it the number of workers; everything else is set
by `tests/conftest.py`: file-level scheduling (`--dist loadfile`, so a file's
module fixtures are built once and its tests share one worker) and crashed
workers replaced (`--max-worker-restart 8`).

```bash
export HOME=$(mktemp -d) BUILT_SETTINGS_DIR=$(mktemp -d)   # never the real profile
python -m pytest -n 8 -m "not slow" tests     # fast suite: while iterating, delegates
python -m pytest -n 8 tests                   # full suite: before installing a build
python -m pytest tests/test_x.py::test_y      # one test, serially (no -n)
```

Use at most `-n 8`: other agents share the machine. Without `-n` nothing
changes from a plain serial pytest run.

## The `slow` marker

`slow` marks tests whose cost is the point and which the fast suite leaves
to the full run:

- long fuzz sweeps;
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
  (`--python-engine` sets `INGETRAZO_RUST=0`), which turned the engine off
  for every later test on a worker;
- the Paint tool's chosen material (`PaintTool.current_*`, class state);
- the scene `core.units` formats with (`bind_scene`), and the Qt application
  and organization names, which name the settings file;
- the drawing axes, the SKP parse tables and `core._native.native`.

Each test file also gets an empty settings store of its own
(`_settings_per_module`): windows save their geometry and docks when they
close, and the next file on the worker restored them.

To find a new leak, compare a fingerprint of process state before and
after each test across an xdist run and look at what changed just before
the failing test on its worker.
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
