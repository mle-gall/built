# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Worker-local native settings, application data and document state."""
import os
import sys
import tempfile

os.environ.setdefault("BUILT_BACKGROUND", "1")
# Camera moves jump instead of gliding (as in `built --script`): a test reads
# the camera right after a view command.
os.environ.setdefault("BUILT_SCRIPTED", "1")
# Also suppress activation in independently launched legacy test processes.
os.environ.setdefault("QT_MAC_DISABLE_FOREGROUND_APPLICATION_TRANSFORM", "1")
os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

# Set these before importing product code, including collection-time imports.
from scripts.isolated_launch import preferences_mtime

_real_preferences_before = preferences_mtime()
_profile_root = tempfile.TemporaryDirectory(prefix="built-pytest-")
_home = os.path.join(_profile_root.name, "home")
_data_dir = os.path.join(_profile_root.name, "data")
_settings_dir = os.path.join(_profile_root.name, "settings")
for directory in (_home, _data_dir, _settings_dir):
    os.makedirs(directory)
os.environ.update(HOME=_home, CFFIXED_USER_HOME=_home, XDG_DATA_HOME=_data_dir,
                  XDG_CONFIG_HOME=_settings_dir, BUILT_SETTINGS_DIR=_settings_dir)
if sys.platform == "win32":
    os.environ.update(USERPROFILE=_home, APPDATA=_data_dir, LOCALAPPDATA=_data_dir)

from built_core import install_main_thread_collector

install_main_thread_collector()

import pytest  # noqa: E402
from built_core import settings as product_settings  # noqa: E402
from tests.shell_isolation import native_host  # noqa: E402,F401

product_settings.use_native(_settings_dir)


@pytest.fixture(autouse=True, scope="module")
def _settings_per_module():
    """Isolate preferences between modules even when xdist reuses a worker."""
    folder = tempfile.mkdtemp(prefix="module-", dir=_settings_dir)
    os.environ["BUILT_SETTINGS_DIR"] = folder
    product_settings.use_native(folder)
    yield
    os.environ["BUILT_SETTINGS_DIR"] = _settings_dir
    product_settings.use_native(_settings_dir)


@pytest.fixture
def private_settings(tmp_path, monkeypatch):
    folder = tmp_path / "settings"
    folder.mkdir()
    monkeypatch.setenv("BUILT_SETTINGS_DIR", str(folder))
    from built_core import NativeSettings
    monkeypatch.setattr(product_settings, "_store", NativeSettings(folder))
    return folder


@pytest.fixture(autouse=True)
def _sheet_looks():
    """The sheets' remembered looks (sheets::looks) are the process's: a
    dimension styled in one test must not dress the next one's."""
    yield
    try:
        import built_core
        forget = getattr(built_core, "sheet_forget_looks", None)
    except Exception:  # noqa: BLE001 — no native module, nothing remembered
        forget = None
    if forget is not None:
        forget()


@pytest.fixture(autouse=True)
def _process_environment():
    """Each test leaves ``os.environ`` as it found it. The app writes it on
    purpose (``core._native.start_guard`` sets BUILT_RUST=0 for
    ``--python-engine``), so a test that runs such code without a
    monkeypatch, or with one that records the written value, changed every
    later test in the process: under pytest-xdist the bake tests then ran
    with the engine off. Autouse, so it is set up before and torn down
    after the test's own monkeypatch."""
    before = dict(os.environ)
    yield
    if dict(os.environ) != before:
        for key in set(os.environ) - set(before):
            del os.environ[key]
        for key, value in before.items():
            if os.environ.get(key) != value:
                os.environ[key] = value


@pytest.fixture(autouse=True)
def _process_units():
    """Restore the document whose units the formatters and value field read."""
    from built_core import units
    bound = units._SCENE
    yield
    units._SCENE = bound


@pytest.fixture(autouse=True)
def _paint_tool_choice():
    """The Paint tool keeps its state on the class, as the app keeps it
    across documents: the chosen material (``PaintTool.current_*``: colour,
    texture, opacity, material, library revision), the recent materials and
    whether the pipette is armed (``sample_armed``). Tests set them
    directly; under pytest-xdist the next file on the worker painted with
    another material (test_pool_water, test_library_paint_selection) or
    sampled instead of painting (test_edit_textured_material_everywhere)."""
    from copy import copy
    from tools.paint import PaintTool

    def mine(key):
        return key.startswith("current_") or key in ("sample_armed", "recent")
    before = {k: copy(v) for k, v in vars(PaintTool).items() if mine(k)}
    yield
    for key in [k for k in vars(PaintTool) if mine(k) and k not in before]:
        delattr(PaintTool, key)
    for key, value in before.items():
        setattr(PaintTool, key, value)


@pytest.fixture(autouse=True)
def _world_drawing_axes():
    """The drawing axes are process-wide (built_core.axes, issue #44): a test that
    leaves a turned group open must not tilt the next one's inferences."""
    from built_core import axes
    axes.sync(None)
    yield
    axes.sync(None)


@pytest.fixture(autouse=True)
def _fresh_skp_parse_tables():
    """The SKP reader keeps the last parse's tables per thread (group
    definition ids, tag ids, persistent ids: built_core.reader_compat). A
    parse resets them first; a test that feeds the adapter a model of its
    own, with no parse, must not inherit the previous file's."""
    from built_core import reader_compat
    reader_compat.reset_group_definitions()
    for name in ("layers", "pids", "folders", "pid_frames"):
        if hasattr(reader_compat._parse_tables, name):
            delattr(reader_compat._parse_tables, name)
    yield


@pytest.fixture(autouse=True, scope="session")
def _session_native():
    """``core._native.native`` as the app has it: resolved once, at start,
    from the environment and the saved preference."""
    import core._native as native_mod
    return native_mod.native


@pytest.fixture(autouse=True)
def _native_per_test(_session_native):
    """``core._native.native`` is resolved on first read and then FIXED for
    the session (a module global written by ``_native.__getattr__``). Tests
    that patch ``_native.wanted`` and read ``native`` meanwhile
    (``tests/test_deferred_mesh.py``, through the loader), or reload the
    module and leave the preference off (``tests/test_rust_engine_switch``),
    fixed it to what THEY had: when one of them ran first in a batch,
    ``native`` was None for the rest of the session and the store tests
    failed. Every test now ends with the session's value put back."""
    yield
    import core._native as native_mod
    native_mod.__dict__["native"] = _session_native


#: Environment variables a test may leave changed: pytest's own, and those
#: the app or a library sets once on first import (read-only afterwards).
_ENV_OWNED_ELSEWHERE = {"PYTEST_CURRENT_TEST"}


def _process_state():
    return {"environment": dict(os.environ)}


def _leaked(before, after, owned=frozenset()):
    out = []
    env_a, env_b = before["environment"], after["environment"]
    for key in sorted(set(env_a) | set(env_b)):
        if key in _ENV_OWNED_ELSEWHERE or key in owned or env_a.get(key) == env_b.get(key):
            continue
        out.append(f"environment {key}: {env_a.get(key)!r} -> {env_b.get(key)!r}")
    return out


def _owned_by_module(request):
    """The native host may select its module fixture's data and settings paths."""
    if "restore_shell_patches" not in request.fixturenames:
        return frozenset()
    from tests.shell_isolation import _ENV
    return frozenset(_ENV)


@pytest.fixture(autouse=True)
def _leak_guard(request, _process_environment, _paint_tool_choice):
    """Identify tests that leave process-wide environment or profile changes."""
    if os.environ.get("BUILT_LEAK_GUARD") == "0":
        yield
        return
    before = _process_state()
    yield
    owned = _owned_by_module(request)
    leaks = _leaked(before, _process_state(), owned)
    if leaks:
        pytest.fail("This test left process-wide state changed:\n  " + "\n  ".join(leaks)
                    + "\nUndo it in the test (monkeypatch, or a finally).", pytrace=False)


def _draw_fuzz_seed():
    """``BUILT_FUZZ_SEED=random``: one seed for the run, drawn here (on the
    controller, before pytest-xdist starts its workers, which inherit it)
    so every worker samples the same fuzz cases (tests/fuzz.py)."""
    from tests import fuzz
    if os.environ.get(fuzz.ENV) == "random" and not os.environ.get("PYTEST_XDIST_WORKER"):
        import secrets
        os.environ[fuzz.ENV] = str(secrets.randbelow(1_000_000))


def pytest_report_header(config):
    from tests import fuzz
    return f"fuzz seed: {fuzz.run_seed()} (set {fuzz.ENV} to repeat it, or to 'random')"


#: A test module that builds an application window, in process or by
#: launching the app, is marked ``window`` (docs/testing.md): the quick
#: subset leaves those to the fast suite.
_WINDOW_RE = None


def pytest_collection_modifyitems(config, items):
    import re
    global _WINDOW_RE
    if _WINDOW_RE is None:
        _WINDOW_RE = re.compile(r"\b(MainWindow|ComposerWindow|ShellWindow|Host)\(|"
                                r"\b(native_host|native_window|preferences_chrome)\b|isolated_launch|release/built\b|\bmain\.py\b")
    import shutil
    blender_skip = None
    if os.environ.get("BUILT_BLENDER_TESTS") != "1":
        blender_skip = pytest.mark.skip(reason="set BUILT_BLENDER_TESTS=1 to run Blender")
    elif shutil.which("blender") is None:
        blender_skip = pytest.mark.skip(reason="no Blender executable on PATH")
    seen = {}
    for item in items:
        layout = item.get_closest_marker("window_size")
        available = os.environ.get("BUILT_CI_WINDOW_SIZE")
        if layout and available:
            width, height = map(int, available.split("x"))
            w, h = layout.args
            if width < w or height < h:
                item.add_marker(pytest.mark.skip(reason=f"scripted clicks require {w}x{h} points; display has {available}"))
        if blender_skip and item.get_closest_marker("blender"):
            item.add_marker(blender_skip)
        path = getattr(item, "path", None)
        if path is None:
            continue
        if path not in seen:
            try:
                seen[path] = bool(_WINDOW_RE.search(path.read_text(errors="ignore")))
            except OSError:
                seen[path] = False
        if seen[path]:
            item.add_marker(pytest.mark.window)
        if os.environ.get("BUILT_CI_SKIP_WINDOWS") and item.get_closest_marker("window"):
            item.add_marker(pytest.mark.skip(reason=os.environ["BUILT_CI_SKIP_WINDOWS"]))


def pytest_configure(config):
    """With pytest-xdist (``-n 8``): file-level scheduling, so a file's
    module fixtures are built once and its tests share a worker, and a
    crashed worker is replaced, unless ``--dist`` / ``--max-worker-restart``
    are given. Here rather than in pytest.ini so a venv without xdist still
    runs, and ``pytest tests/x.py::test`` stays serial (docs/testing.md)."""
    _draw_fuzz_seed()
    if not config.pluginmanager.hasplugin("xdist"):
        return
    given = " ".join(map(str, config.invocation_params.args))
    if getattr(config.option, "numprocesses", None) and "--dist" not in given:
        config.option.dist = "loadfile"
    if getattr(config.option, "maxworkerrestart", None) is None and "--max-worker-restart" not in given:
        config.option.maxworkerrestart = 8


@pytest.hookimpl(trylast=True)
def pytest_sessionfinish(session, exitstatus):
    """A change to the real macOS preferences domain invalidates the run."""
    if not hasattr(session.config, "workerinput") and preferences_mtime() != _real_preferences_before:
        pytest.exit("The real Built preferences plist changed during the test session",
                    returncode=pytest.ExitCode.TESTS_FAILED)
