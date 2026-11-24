# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""One full QApplication for the whole test session, created BEFORE any test
module imports. Test files that need widgets (MainWindow, the tray) and
files that only need fonts used to create their own app at import time —
a QGuiApplication first meant every later widget test aborted, so the
outcome depended on which files were on the command line."""
import os
import sys

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

# As main.py does: a collection a worker thread starts frees nothing, so
# no Qt object a test let go of is destroyed off the main thread.
from core import worker_gc  # noqa: E402

worker_gc.install()

# The user's own extensions folder stays out of the suite, like their
# settings below. Every MainWindow a test built loaded whatever the developer
# had installed (core.extensions.user_plugins_dir): 09-10, with three
# extensions installed from the manager, tests/test_outliner.py went from
# 29 s to 174 s and the fast suite from 12 min to over an hour — and a
# result could depend on someone's extensions. Same for the app's local
# data (the manager's screenshot cache). Set before anything reads it.
import tempfile  # noqa: E402

_data_dir = tempfile.mkdtemp(prefix="ingetrazo-tests-data-")
os.environ["XDG_DATA_HOME"] = _data_dir
if sys.platform == "darwin":
    # Foundation resolves QStandardPaths independently from HOME. Give each
    # pytest worker its own Application Support and untitled recovery slots.
    os.environ["CFFIXED_USER_HOME"] = _data_dir
if sys.platform == "win32":
    os.environ["APPDATA"] = _data_dir
    # core.paths.user_log_dir reads it: without this every test that rolls
    # a command back wrote its traceback into the developer's real
    # %LOCALAPPDATA%\IngeTrazo\ingetrazo-errors.log (ronda Windows 0.5.9).
    os.environ["LOCALAPPDATA"] = _data_dir

# Windows: the offscreen platform has NO font database there (fontconfig
# finds the fonts on Linux; on Windows nothing looks), so every glyph drew
# as an empty box — 3D text lost the O's counter and built faces for
# spaces, and every test that measures a label's width measured boxes.
# Pointed at the system fonts, and with the faces the app gets from the
# native platform (Segoe UI 9 pt; the "Sans" alias the tests and the
# Linux defaults name resolves to a sans-serif, not the alphabetically
# first family), the suite sees the text a Windows user sees.
_WIN_OFFSCREEN = (sys.platform == "win32"
                  and os.environ["QT_QPA_PLATFORM"] == "offscreen")
if _WIN_OFFSCREEN:
    os.environ.setdefault("QT_QPA_FONTDIR", os.path.join(
        os.environ.get("WINDIR", r"C:\Windows"), "Fonts"))

from PySide6.QtWidgets import QApplication  # noqa: E402

if QApplication.instance() is None:
    QApplication(sys.argv[:1])

# Match Qt startup before any test creates its first widget.
from views.theme import install_dark_palette  # noqa: E402

install_dark_palette(QApplication.instance())

if _WIN_OFFSCREEN:
    from PySide6.QtGui import QFont  # noqa: E402
    for _alias in ("Sans", "Sans Serif", "DejaVu Sans"):
        QFont.insertSubstitutions(_alias, ["Segoe UI", "Arial"])
    QApplication.setFont(QFont("Segoe UI", 9))

# And a QSettings store of its own, thrown away with the session. The suite
# is not read-only about preferences: a test that exercises «new items
# inherit the last style» writes that style out, so it landed in the
# developer's real settings and POISONED ITS OWN NEXT RUN — the cota style
# test placed a cota with text_pos="below" and then failed the next day
# asserting "above". Isolating the store makes every run start from the
# same place, here and on the CI runner.
import tempfile  # noqa: E402

from PySide6.QtCore import QSettings  # noqa: E402

_settings_dir = tempfile.mkdtemp(prefix="ingetrazo-tests-settings-")
os.environ["BUILT_SETTINGS_DIR"] = _settings_dir
QSettings.setDefaultFormat(QSettings.IniFormat)


#: The store the current test module writes (``_settings_per_module``).
_module_settings = [_settings_dir]


def _session_settings_paths():
    QSettings.setPath(QSettings.IniFormat, QSettings.UserScope, _module_settings[0])
    QSettings.setPath(QSettings.IniFormat, QSettings.SystemScope, os.path.join(_module_settings[0], "system"))


_session_settings_paths()


import pytest  # noqa: E402


@pytest.fixture(autouse=True, scope="module")
def _settings_per_module():
    """Each test file starts from an empty settings store of its own. A
    MainWindow saves its geometry and docks when it closes and the next one
    restores them, a style test saves «the last style used»: under
    pytest-xdist a file met whatever the files before it on its worker had
    left (a 173 px viewport in test_join_segments). Within a file the tests
    still share the store, in the order they always ran."""
    folder = tempfile.mkdtemp(prefix="module-", dir=_settings_dir)
    _module_settings[0] = folder
    os.environ["BUILT_SETTINGS_DIR"] = folder
    _session_settings_paths()
    yield
    _module_settings[0] = _settings_dir
    os.environ["BUILT_SETTINGS_DIR"] = _settings_dir
    _session_settings_paths()


@pytest.fixture(autouse=True)
def _restore_settings_paths():
    # setPath changes process-wide defaults. A private user profile must
    # never inherit another test's saved window state via SystemScope.
    yield
    _session_settings_paths()


@pytest.fixture(autouse=True)
def _process_environment():
    """Each test leaves ``os.environ`` as it found it. The app writes it on
    purpose (``core._native.start_guard`` sets INGETRAZO_RUST=0 for
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
def _process_qt_and_units():
    """Process-wide state a test can change in passing, put back after it:

    - the application and organization names, which name the settings file
      (a test that cleared them sent every later test on its worker to
      «Unknown Organization/Python.ini»);
    - the scene whose units the formatters and the value field read
      (``core.units.bind_scene``): every window binds its own, so the last
      one's inches were how test_vcb_comma_lists read «200,100»."""
    from PySide6.QtCore import QCoreApplication
    from core import units
    names = (QCoreApplication.organizationName(), QCoreApplication.organizationDomain(),
             QCoreApplication.applicationName())
    bound = units._SCENE
    yield
    units._SCENE = bound
    if names != (QCoreApplication.organizationName(), QCoreApplication.organizationDomain(),
                 QCoreApplication.applicationName()):
        QCoreApplication.setOrganizationName(names[0])
        QCoreApplication.setOrganizationDomain(names[1])
        QCoreApplication.setApplicationName(names[2])


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
    """The drawing axes are process-wide (core.axes, issue #44): a test that
    leaves a turned group open must not tilt the next one's inferences."""
    from core import axes
    axes.sync(None)
    yield
    axes.sync(None)


@pytest.fixture(autouse=True)
def _fresh_skp_parse_tables():
    """The SKP reader keeps the last parse's tables per thread (group
    definition ids, tag ids, persistent ids: formats.openskp_compat). A
    parse resets them first; a test that feeds the adapter a model of its
    own, with no parse, must not inherit the previous file's."""
    from formats import openskp_compat
    openskp_compat.reset_group_definitions()
    for name in ("layers", "pids", "folders", "pid_frames"):
        if hasattr(openskp_compat._parse_tables, name):
            delattr(openskp_compat._parse_tables, name)
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


def pytest_configure(config):
    """With pytest-xdist (``-n 8``): file-level scheduling, so a file's
    module fixtures are built once and its tests share a worker, and a
    crashed worker is replaced, unless ``--dist`` / ``--max-worker-restart``
    are given. Here rather than in pytest.ini so a venv without xdist still
    runs, and ``pytest tests/x.py::test`` stays serial (docs/testing.md)."""
    if not config.pluginmanager.hasplugin("xdist"):
        return
    given = " ".join(map(str, config.invocation_params.args))
    if getattr(config.option, "numprocesses", None) and "--dist" not in given:
        config.option.dist = "loadfile"
    if getattr(config.option, "maxworkerrestart", None) is None and "--max-worker-restart" not in given:
        config.option.maxworkerrestart = 8


_exit_status = [0]


def pytest_sessionfinish(session, exitstatus):
    """Leave no Python-owned clipboard entry (``formats.clip.ClipMime``) in
    Qt's clipboard while the interpreter tears down — a Copy in a test puts
    one there, and the CI run of 9393ef5 passed and then segfaulted on exit."""
    _exit_status[0] = int(exitstatus)
    try:
        from PySide6.QtWidgets import QApplication
        if QApplication.instance() is not None:
            from formats import clip
            clip.flush()
            QApplication.clipboard().clear()
    except Exception:  # noqa: BLE001 - teardown must not fail the run
        pass


@pytest.hookimpl(trylast=True)
def pytest_unconfigure(config):
    """Leave without tearing the interpreter down. The windows the tests
    build outlive them (~170 MainWindows, see scripts/pytest_batches.py),
    and destroying them all at exit, in whatever order Python picks, is
    not something the app ever does: the public CI of 0.5.8 passed every
    test and then two batches segfaulted on exit («shared QObject was
    deleted directly»). Everything is reported by now; the status is
    pytest's own."""
    sys.stdout.flush()
    sys.stderr.flush()
    os._exit(_exit_status[0])


@pytest.fixture(autouse=True, scope="session")
def _no_stray_autosave():
    """Every MainWindow a test builds armed the 5-minute auto-save timer,
    and the windows outlive their tests. Once the session ran past five
    minutes those timers fired inside later tests, and the tick's
    gc.collect() over half-torn-down Qt objects stalled or crashed the run
    (24-09: runs stuck at ~95 %, the faulthandler dump caught it in
    ``_on_autosave_tick``). The timer is still built and armed — the
    Preferences test reads its settings — then stopped at once; tests that
    exercise the tick call it directly."""
    from views.main_window import MainWindow
    original = MainWindow._setup_autosave

    def armed_but_stopped(self):
        original(self)
        self._autosave_timer.stop()

    MainWindow._setup_autosave = armed_but_stopped
    yield
    MainWindow._setup_autosave = original


@pytest.fixture
def explicit_store_meshes(monkeypatch):
    """Differential suites build their store operands explicitly."""
    monkeypatch.setenv("INGETRAZO_STORE", "0")
