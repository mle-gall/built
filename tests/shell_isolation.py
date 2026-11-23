"""Undo what app_shell installs process-wide (app_shell.modal.install,
app_shell.files.install) so the shell's tests leave the Qt app's modules as
they found them for the tests that run after them in the same process."""
import importlib
import os

import pytest

_ENV = ("QT_QPA_PLATFORM", "XDG_DATA_HOME", "BUILT_ANSWERS", "BUILT_SETTINGS_DIR")

_PATCHED = {
    "views.chrome_dialogs": ("live", "QEventLoop", "_shell_installed"),
    "views.filedialogs": ("QFileDialog",),
    "core.renderer_settings": ("capability", "wanted", "status_text", "_shell_installed"),
    "views.wgpu_start": ("available", "_qt_home", "_shell_installed"),
    "app_shell.modal": ("_HOST",),
}
_MISSING = object()


@pytest.fixture(autouse=True, scope="module")
def restore_shell_patches(tmp_path_factory):
    env = {k: os.environ.get(k) for k in _ENV}
    os.environ["BUILT_SETTINGS_DIR"] = str(tmp_path_factory.mktemp("shell-settings"))
    from PySide6.QtCore import QCoreApplication, QSettings, QStandardPaths, QTimer
    from PySide6.QtGui import QCursor, QGuiApplication
    qt = [(QGuiApplication, a, QGuiApplication.__dict__.get(a, _MISSING))
          for a in ("keyboardModifiers", "queryKeyboardModifiers", "mouseButtons")]
    qt += [(QCursor, "pos", QCursor.__dict__.get("pos", _MISSING))]
    qt += [(QTimer, a, QTimer.__dict__.get(a, _MISSING))
           for a in ("start", "singleShot", "_shell_tracked")]
    names = (QCoreApplication.organizationName(), QCoreApplication.applicationName())
    test_mode = QStandardPaths.isTestModeEnabled()
    fmt = QSettings.defaultFormat()
    saved = {}
    for name, attrs in _PATCHED.items():
        try:
            mod = importlib.import_module(name)
        except Exception:  # noqa: BLE001
            continue
        saved[name] = (mod, {a: getattr(mod, a, _MISSING) for a in attrs})
    yield
    for cls, attr, value in qt:
        if value is _MISSING:
            if attr in cls.__dict__:
                try:
                    delattr(cls, attr)
                except (AttributeError, TypeError):
                    pass
        else:
            setattr(cls, attr, value)
    try:
        from app_shell import qt_host
        guard = getattr(qt_host, "_GUARD", None)
        app = getattr(qt_host, "_APP", None)
        if guard is not None and app is not None:
            app.removeEventFilter(guard.filter)
        qt_host._GUARD = None
        qt_host._APP = None
        qt_host._SETTINGS_DIR = None
    except Exception:  # noqa: BLE001
        pass
    QCoreApplication.setOrganizationName(names[0])
    QCoreApplication.setApplicationName(names[1])
    QStandardPaths.setTestModeEnabled(test_mode)
    QSettings.setDefaultFormat(fmt)
    for k, v in env.items():
        if v is None:
            os.environ.pop(k, None)
        else:
            os.environ[k] = v
    for mod, values in saved.values():
        for attr, value in values.items():
            if value is _MISSING:
                if hasattr(mod, attr):
                    delattr(mod, attr)
            else:
                setattr(mod, attr, value)
