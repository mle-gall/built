"""Undo what app_shell installs process-wide (app_shell.modal.install,
app_shell.files.install) so the shell's tests leave the Qt app's modules as
they found them for the tests that run after them in the same process."""
import importlib
import os

import pytest

_ENV = ("QT_QPA_PLATFORM", "XDG_DATA_HOME", "BUILT_ANSWERS", "BUILT_SETTINGS_DIR")

_PATCHED = {
    "core.settings": ("_store",),
    "views.chrome_dialogs": ("live", "QEventLoop", "_shell_installed"),
    "views.filedialogs": ("QFileDialog",),
    "core.renderer_settings": ("capability", "status_text", "_shell_installed"),
    "app_shell.modal": ("_HOST",),
}
_MISSING = object()


@pytest.fixture(autouse=True, scope="module")
def restore_shell_patches(tmp_path_factory):
    env = {k: os.environ.get(k) for k in _ENV}
    os.environ["BUILT_SETTINGS_DIR"] = str(tmp_path_factory.mktemp("shell-settings"))
    from PySide6.QtCore import QCoreApplication, QSettings, QStandardPaths, QTimer
    qt = [(QTimer, a, QTimer.__dict__.get(a, _MISSING))
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


@pytest.fixture
def native_host(tmp_path, restore_shell_patches, monkeypatch):
    """One native host, including its initial document and private profile."""
    from app_shell.host import Host
    from core import settings
    profile = str(tmp_path / "profile")
    # The offscreen QApplication lives for the module. Choose this document's
    # test store even when Qt has already been initialized by an earlier test.
    from ingetrazo_core import NativeSettings
    monkeypatch.setattr(settings, "_store", NativeSettings(profile))
    host = Host(0, 900, 600, 1., settings=profile)
    try:
        yield host
    finally:
        host.close()


@pytest.fixture
def native_question_host(native_host, monkeypatch):
    """Use the real sheet stack while driving a native document without a renderer."""
    from views import chrome_dialogs
    monkeypatch.setattr(chrome_dialogs, "async_host",
                        lambda parent: native_host.window if parent is native_host.window else None)
    return native_host


@pytest.fixture
def native_window(native_host):
    """A fresh document in the native window."""
    native_host.window.new_document()
    return native_host.window


def one_off_export(win, fmt, destination):
    """Drive the File menu, Export sheet and queued writer on the UI thread."""
    from ingetrazo_render import chrome_sheet
    win.run_command(f"export:{fmt}")
    ctx = chrome_sheet.Ctx(win, win.view)
    provider = chrome_sheet.current(win)
    assert provider.id == "export"
    form = provider.build(ctx).forms()[0]
    assert form.value("format") == fmt
    form.receive("destination", str(destination))
    form.press("export")
    for _ in range(20):
        if not win._delivery_queue:
            break
        win._host.idle()
    assert not win._delivery_queue and destination.is_file()
    return form


@pytest.fixture
def native_gpu_host(native_host, monkeypatch):
    """Attach the native renderer to a hidden platform window for pixel readbacks."""
    from PySide6.QtCore import Qt
    from PySide6.QtWidgets import QApplication, QWidget
    from views.wgpu_surface import window_handles
    if QApplication.platformName() in {"offscreen", "minimal"}:
        pytest.skip("pixel readback requires a native window system")
    surface = QWidget()
    surface.setAttribute(Qt.WA_DontShowOnScreen)
    surface.resize(native_host.width, native_host.height)
    handle, platform = window_handles(surface)
    # The window handle must outlive the renderer in native_host's teardown.
    native_host._test_surface = surface
    native_host.ns_view = handle
    native_host.surface_handles = platform
    # Pixel assertions sample model coordinates. Keep egui out of that buffer.
    from views import wgpu_ui
    monkeypatch.setattr(wgpu_ui, "attach", lambda view: None)
    assert native_host.attach_surface() is not None
    return native_host
