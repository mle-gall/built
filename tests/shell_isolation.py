"""Isolated native document and window fixtures."""
from contextlib import contextmanager
import importlib
from pathlib import Path
import os

import pytest

_ENV = ("XDG_DATA_HOME", "BUILT_ANSWERS", "BUILT_SETTINGS_DIR")

_PATCHED = {
    "core.settings": ("_store",),
    "ingetrazo_render.chrome_dialogs": ("live", "QEventLoop", "_shell_installed"),
    "views.filedialogs": ("QFileDialog",),
    "core.renderer_settings": ("capability", "status_text", "_shell_installed"),
    "app_shell.modal": ("_HOST",),
}
_MISSING = object()


@contextmanager
def _shell_state():
    """Restore process-wide shell services even when a host fails to open."""
    env = {k: os.environ.get(k) for k in _ENV}
    saved = []
    for name, attrs in _PATCHED.items():
        mod = importlib.import_module(name)
        saved.extend((mod, attr, getattr(mod, attr, _MISSING)) for attr in attrs)
    try:
        yield
    finally:
        for k, v in env.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v
        for mod, attr, value in reversed(saved):
            if value is _MISSING:
                if hasattr(mod, attr):
                    delattr(mod, attr)
            else:
                setattr(mod, attr, value)


@pytest.fixture(autouse=True, scope="module")
def restore_shell_patches(tmp_path_factory):
    with _shell_state():
        os.environ["BUILT_SETTINGS_DIR"] = str(tmp_path_factory.mktemp("shell-settings"))
        yield


@contextmanager
def open_test_host(profile, *, width=900, height=600, scale=1.):
    """Open a private document without creating an OS window."""
    from app_shell.host import Host
    with _shell_state():
        os.environ["BUILT_SETTINGS_DIR"] = str(Path(profile).resolve())
        host = Host(0, width, height, scale, settings=os.environ["BUILT_SETTINGS_DIR"])
        try:
            yield host
        finally:
            host.close()


@pytest.fixture
def native_host(tmp_path):
    with open_test_host(tmp_path / "profile") as host:
        yield host


@pytest.fixture
def native_question_host(native_host, monkeypatch):
    """Use the real sheet stack while driving a native document without a renderer."""
    from ingetrazo_render import chrome_dialogs
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


def attach_test_renderer(host):
    """Create a real offscreen GPU target for an isolated Host."""
    from ingetrazo_render import Renderer
    renderer = Renderer(0, host.width, host.height, host.scale, platform="offscreen")
    # The offscreen target can present a frame although there is no native window.
    host.view._presentable = lambda: host.view.renderer is not None
    host.hand_renderer(host.view, renderer)
    host.window._activate_tool("select")
    return renderer


@pytest.fixture
def native_gpu_host(native_host, monkeypatch):
    from views import wgpu_ui
    monkeypatch.setattr(wgpu_ui, "attach", lambda view: None)
    attach_test_renderer(native_host)
    return native_host
