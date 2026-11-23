"""The two bundled declarative plugins and their host event boundary."""
import json
from types import SimpleNamespace
from unittest.mock import MagicMock

import pytest

from core import plugin_panels
from plugins import python_console, render_blender
from views.extension_api import ExtensionApp


@pytest.fixture
def window():
    from views.main_window import MainWindow
    win = MainWindow(defer_extensions=True)
    yield win
    win._saved_version = win.viewport.scene.version
    win.close()


def test_console_session(window):
    panel = python_console.create_panel(ExtensionApp(window, "python_console"), lambda: None)
    panel.event("prompt", "console_submit", "scene is model")
    assert panel.lines[-1] == "True"
    assert panel._scope["document"] is window._doc
    panel.event("prompt", "console_submit", "1/0")
    assert any("ZeroDivisionError" in line for line in panel.lines)
    panel.event("clear", "invoke", None)
    assert not panel.lines and panel.history == ["scene is model", "1/0"]


@pytest.mark.parametrize("cancel", [False, True])
def test_render_job(window, monkeypatch, tmp_path, cancel):
    rb = render_blender.rb
    found = rb.BlenderFound(["unused"], "mock Blender")
    monkeypatch.setattr(rb, "find_blender", lambda *_: found)
    monkeypatch.setattr(rb, "work_base", lambda: tmp_path)
    write_job = MagicMock(return_value=tmp_path / "job.json")
    monkeypatch.setattr(rb, "write_job", write_job)
    monkeypatch.setattr(rb, "command", lambda *_: ["mock-blender", "--background"])
    process_class = MagicMock()
    monkeypatch.setattr(render_blender, "QProcess", process_class)
    proc = process_class.return_value
    panel = render_blender.create_panel(ExtensionApp(window, "render_blender"), lambda: None)
    panel.event("width", "commit", 384)
    panel.event("engine", "commit", "cycles")
    panel.event("render", "invoke", None)
    proc.start.assert_called_once_with("mock-blender", ["--background"])
    assert write_job.call_args.kwargs["width"] == 384
    assert write_job.call_args.kwargs["engine"] == "cycles"
    assert window._chrome_jobs == [panel.job]
    proc.readAllStandardOutput.return_value = b"INGETRAZO stats Rendering 32 / 64 samples\n"
    proc.readyReadStandardOutput.connect.call_args.args[0]()
    assert panel.fraction == panel.job.fraction == .5
    if cancel:
        panel.job.cancel()
        proc.kill.assert_called_once()
    else:
        from PySide6.QtGui import QImage
        image = QImage(8, 8, QImage.Format_RGB32)
        image.fill(0x557788)
        assert image.save(str(panel.work / "render.png"))
    proc.finished.connect.call_args.args[0](0)
    assert not window._chrome_jobs
    assert panel.status == ("Cancelled" if cancel else "Complete")
    preview = [i for s in panel.sections() for i in s["items"] if i["kind"] == "image"]
    assert [i["path"] for i in preview] == ([] if cancel else [str(panel.work / "render.png")])


@pytest.mark.parametrize("kind,value", [
    ("text", "title"), ("number", 2.5), ("toggle", True), ("choice", "cycles"),
    ("colour", [1, 2, 3]), ("path", "/tmp/render.png")])
def test_typed_dispatch_only_to_current_instance(kind, value):
    scene = SimpleNamespace(version=4)
    win = SimpleNamespace(viewport=SimpleNamespace(scene=scene))
    view = MagicMock()
    view._plugin_document = "doc"
    win._doc = SimpleNamespace(stand_in=view)
    model = SimpleNamespace(id="settings", title="Settings", slot="dock", event=MagicMock())
    binding = plugin_panels.Binding(win, "example", lambda *_: model, view)
    view._plugin_bindings = {binding.instance: binding}
    event = dict(api=1, plugin="example", panel="settings", instance=binding.instance,
                 document="doc", revision=4, event_id=1, control=kind, event="commit", value=value)
    plugin_panels.dispatch(win, json.dumps(event))
    plugin_panels.dispatch(win, json.dumps(event))
    scene.version += 1
    plugin_panels.dispatch(win, json.dumps(dict(event, event_id=2)))
    model.event.assert_called_once_with(kind, "commit", value)


def test_pick_callback_only_for_original_revision(window):
    app = ExtensionApp(window, "demo")
    done = MagicMock()
    app.pick_point("Place light", done)
    vp = app.viewport
    tool = vp.active_tool
    vp.scene.version += 1
    from types import SimpleNamespace
    tool.on_click(SimpleNamespace(world=None, snap=None))
    done.assert_not_called()
    assert vp.active_tool is not tool


def test_classic_plugin_entry_points(window, monkeypatch):
    from views import wgpu_ui
    monkeypatch.setattr(wgpu_ui, "ENABLED", False)
    python_console.PythonConsoleTool().on_activate(window.viewport)
    assert isinstance(window._python_console, python_console.PythonConsoleDialog)
    window._python_console.close()

    app = ExtensionApp(window, "render_blender")
    panels = []
    monkeypatch.setattr(app, "add_panel", lambda title, panel, **kw: panels.append(panel))
    monkeypatch.setattr(app, "add_menu_action", lambda *a, **kw: None)
    render_blender.setup(app)
    assert len(panels) == 1 and isinstance(panels[0], render_blender.RenderPanel)
    panels[0].deleteLater()
