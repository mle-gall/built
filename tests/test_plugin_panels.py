"""The two bundled declarative plugins and their host event boundary."""
import json
from types import SimpleNamespace
from unittest.mock import MagicMock

import pytest

from built_core import plugin_panels
from plugins import python_console, render_blender
from views.extension_api import ExtensionApp


from tests.shell_isolation import restore_shell_patches  # noqa: F401


@pytest.fixture
def window(native_host):
    return native_host.window


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
    from plugins import _blender_panel
    monkeypatch.setattr(_blender_panel.subprocess, "Popen", process_class)
    proc = process_class.return_value
    panel = render_blender.create_panel(ExtensionApp(window, "render_blender"), lambda: None)
    panel.event("width", "commit", 384)
    panel.event("engine", "commit", "cycles")
    panel.event("render", "invoke", None)
    assert process_class.call_args.args[0] == ["mock-blender", "--background"]
    assert write_job.call_args.kwargs["width"] == 384
    assert write_job.call_args.kwargs["engine"] == "cycles"
    assert window._chrome_jobs == [panel.job]
    (panel.work / "blender.log").write_bytes(b"BUILT stats Rendering 32 / 64 samples\n")
    proc.poll.return_value = None
    panel._poll_process()
    assert panel.fraction == panel.job.fraction == .5
    if cancel:
        panel.job.cancel()
        proc.kill.assert_called_once()
    else:
        from tests.plugin_images import write_png
        write_png(panel.work / "render.png", 8, 8)
    proc.poll.return_value = 0
    panel._poll_process()
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
    plugin_panels.dispatch(win, json.dumps(dict(event, event_id=2, control="_panel", event="closed")))
    scene.version += 1
    plugin_panels.dispatch(win, json.dumps(dict(event, event_id=3)))
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


def test_plugin_entry_points_open_the_registered_native_panels(window, monkeypatch):
    opened = []
    monkeypatch.setattr(plugin_panels, "open_panel",
                        lambda win, key: opened.append((win, key)) or True)
    python_console.PythonConsoleTool().on_activate(window.viewport)
    actions = []
    app = ExtensionApp(window, "render_blender")
    monkeypatch.setattr(app, "add_menu_action", lambda title, callback: actions.append(callback))
    render_blender.setup(app)
    assert len(actions) == 1
    actions[0]()
    assert opened == [(window, "python_console"), (window, "render_blender")]


def test_failed_panel_keeps_its_last_rows_and_retries():
    def sections():
        if broken:
            raise SystemExit("unavailable")
        return [plugin_panels.section("main", "Main", plugin_panels.item("log", "ok"))]

    win = SimpleNamespace(viewport=SimpleNamespace(scene=SimpleNamespace(version=0)))
    view = MagicMock(_plugin_document="doc")
    win._doc = SimpleNamespace(stand_in=view)
    model = SimpleNamespace(id="test", title="Test", slot="dock", sections=sections)
    binding = plugin_panels.Binding(win, "owner", lambda *_: model, view)
    view._plugin_bindings = {binding.instance: binding}

    broken = False
    binding.push()
    good = sections()
    broken = True
    binding.invalidate()
    binding.push()
    rows = json.loads(view.renderer.ui_plugin_panel.call_args.args[0])["sections"]
    assert rows[:-1] == good
    assert rows[-1]["items"][1] == plugin_panels.item("button", "_retry", "Retry")

    broken = False
    plugin_panels.dispatch(win, json.dumps(dict(api=1, revision=0, document="doc",
        plugin="owner", panel="test", instance=binding.instance, event_id=1,
        control="_retry", event="invoke", value=None)))
    binding.push()
    assert json.loads(view.renderer.ui_plugin_panel.call_args.args[0])["sections"] == good


def test_releasing_the_view_collects_a_panel_that_retains_its_invalidator():
    import gc
    import weakref

    win = SimpleNamespace(viewport=SimpleNamespace(scene=SimpleNamespace(version=0)))
    view = SimpleNamespace(_plugin_document="doc")
    def factory(app, invalidate):
        return SimpleNamespace(id="p", title="Panel", slot="dock", invalidate=invalidate)
    binding = plugin_panels.Binding(win, "owner", factory, view)
    view._plugin_bindings = {binding.instance: binding}
    ref = weakref.ref(binding)
    del binding, view
    gc.collect()
    assert ref() is None
