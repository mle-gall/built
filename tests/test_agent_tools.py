# SPDX-License-Identifier: GPL-3.0-or-later
from tests.raster_samples import solid_raster
from tests.shell_isolation import restore_shell_patches  # noqa: F401
import threading
from types import SimpleNamespace

import pytest
from built_core import Matrix4 as QMatrix4x4, Vec3 as V
from built_core.group import Group
from built_core.mesh import Mesh
from built_core.layers import Layer
from plugins.ai_bridge import _Bridge
from views.extension_api import ExtensionApp
from tests.test_ai_bridge import _ask


@pytest.fixture
def model(native_host, monkeypatch):
    monkeypatch.setenv("BUILT_AI_PORT", "0")
    win = native_host.window
    scene = win.viewport.scene
    scene.groups.clear()
    mesh = Mesh()
    mesh.add_face([V(0, 0, 0), V(2, 0, 0), V(0, 2, 0)])
    child = Group(mesh, name="Child")
    parent = Group(Mesh(), name="Parent")
    parent.children = [child]
    parent.xform = QMatrix4x4()
    parent.xform.translate(10, 0, 0)
    child.ifc = {"class": "IfcWall", "custom": {"a": 1}}
    scene.groups.append(parent)
    layer = Layer("Walls")
    scene.layers.append(layer)
    child.layer = layer.name
    bridge = _Bridge(win.viewport)
    yield win, bridge, parent, child, layer
    bridge.stop()


def call(bridge, tool, **args):
    job = dict(req=dict(id=1, tool=tool, args=args), done=threading.Event())
    bridge._run_on_main(job)
    assert job["done"].is_set()
    assert job["reply"]["ok"], job["reply"]
    return job["reply"]["result"]


def test_registry_extension_socket_and_unload(model):
    win, bridge, *_ = model
    app = ExtensionApp(win, "demo")
    seen = []
    off = app.add_agent_tool("hello", lambda text: seen.append(threading.current_thread()) or text,
                             description="Echo", input_schema={"type": "object"})
    with pytest.raises(ValueError):
        app.add_agent_tool("hello", lambda: None, description="Duplicate")
    with pytest.raises(ValueError):
        app.add_agent_tool("undo", lambda: None, description="Duplicate")
    bridge.start()
    assert _ask(bridge, "hello", {"text": "hi"})["result"] == "hi"
    assert seen == [threading.main_thread()]
    assert "hello" in {s["name"] for s in call(bridge, "list_tools")["tools"]}
    off(); off()
    assert "hello" not in {s["name"] for s in call(bridge, "list_tools")["tools"]}
    app.add_agent_tool("bye", lambda: None, description="Bye")
    app.remove_agent_tools()
    assert "bye" not in bridge._extension_tools
    assert "unknown tool 'hello'" in _ask(bridge, "hello")["error"]


@pytest.mark.parametrize("failure", ["exception", "object", "nan"])
def test_mutation_rolls_back_on_error_or_non_json_result(model, failure):
    win, bridge, *_ = model
    app = ExtensionApp(win, "demo")
    def broken():
        win.viewport.scene.groups.append(Group(Mesh(), name="Temporary"))
        if failure == "exception":
            raise ValueError("broken")
        return object() if failure == "object" else float("nan")
    app.add_agent_tool("broken", broken, description="Broken", mutates=True)
    bridge.start()
    before = len(win.viewport.history.undo_stack)
    result = _ask(bridge, "broken")
    assert not result["ok"]
    assert [g.name for g in win.viewport.scene.groups] == ["Parent"]
    assert len(win.viewport.history.undo_stack) == before


def test_mutation_undo_redo(model):
    win, bridge, *_ = model
    def add():
        win.viewport.scene.groups.append(Group(Mesh(), name="Added"))
        return {"added": True}
    ExtensionApp(win, "demo").add_agent_tool("add", add, description="Add", mutates=True)
    before = len(win.viewport.history.undo_stack)
    assert call(bridge, "add") == {"added": True}
    assert len(win.viewport.history.undo_stack) == before + 1
    call(bridge, "undo")
    assert len(win.viewport.scene.groups) == 1
    call(bridge, "redo")
    assert len(win.viewport.scene.groups) == 2


def test_tree_entity_selection_and_layers(model):
    win, bridge, parent, child, layer = model
    tree = call(bridge, "query_tree", limit=1)
    assert tree["rows"][0]["uid"] == parent.uid
    assert tree["next_offset"] == 1
    entity = call(bridge, "query_entity", uid=child.uid)
    assert entity["path"] == ["Parent"]
    assert entity["bounds"] == {"min": [10, 0, 0], "max": [12, 2, 0]}
    assert entity["ifc"] == child.ifc
    assert entity["faces"] == 1 and entity["edges"] == 3
    assert entity["placement_matrix"][12] == 10
    assert call(bridge, "query_tree", root_uid=child.uid, depth=0)["rows"][0]["depth"] == 0
    twin = Group(child.mesh, name="Twin")
    win.viewport.scene.groups.append(twin)
    assert call(bridge, "query_entity", uid=twin.uid)["shared_definition_key"] == entity["shared_definition_key"]
    child.hidden = True
    assert child.uid not in [r["uid"] for r in call(bridge, "query_tree", include_hidden=False)["rows"]]
    child.hidden = False
    result = call(bridge, "select", pattern="Chi*")
    assert result["bounds"] == entity["bounds"] and result["groups"][0]["class"] == "IfcWall"
    assert len(call(bridge, "select", uids=[parent.uid], mode="add")["groups"]) == 2
    assert len(call(bridge, "select", uids=[parent.uid], mode="remove")["groups"]) == 1
    assert not call(bridge, "select", mode="clear")["groups"]
    rows = call(bridge, "query_layers")["layers"]
    assert next(r for r in rows if r["uid"] == layer.uid)["groups"] == 1
    call(bridge, "set_layer_visibility", uids_or_names=[layer.uid], visible=False)
    assert not layer.visible
    assert call(bridge, "undo")["ok"]
    assert win.viewport.scene.layer("Walls").visible


def test_layer_usage_includes_untagged_groups_and_shared_faces(model):
    win, bridge, parent, child, layer = model
    layer.name = "A"                      # a label; entities carry the key
    child.layer = layer.key
    win.viewport.scene.groups.append(Group(child.mesh))
    child.mesh.faces[0].attrs["layer"] = None
    rows = {r["name"]: r for r in call(bridge, "query_layers")["layers"]}
    assert rows["Layer 0"]["groups"] == 2
    assert rows["Layer 0"]["faces"] == 1
    assert rows["A"]["groups"] == 1


def test_camera_zoom_and_screenshot(model, monkeypatch):
    win, bridge, parent, child, _ = model
    before = len(win.viewport.history.undo_stack)
    for preset in win.viewport.camera._STANDARD_VIEWS:
        assert call(bridge, "set_camera", preset=preset, ortho=True)["preset"] == preset
    camera = call(bridge, "set_camera", eye=[10, -10, 10], target=[10, 0, 0], up=[0, 0, 1], ortho=False)
    assert camera["target"] == pytest.approx([10, 0, 0])
    call(bridge, "zoom_to", uids=[child.uid])
    assert 10 <= call(bridge, "query_camera")["target"][0] <= 12
    from built_render import SurveyRaster
    monkeypatch.setattr(win.viewport, "render_image", lambda w, h: solid_raster(w, h, (0, 0, 0)))
    shot = call(bridge, "screenshot", uids=[child.uid], width=64, height=64)
    assert shot["camera"] == call(bridge, "query_camera")
    assert len(win.viewport.history.undo_stack) == before


def test_walkthrough_camera_verbs_keep_orbit_distance_and_document_history(model):
    win, bridge, *_ = model
    depth = len(win.viewport.history.undo_stack)
    before = call(bridge, "set_camera", eye=[0, 0, 2], target=[0, 4, 2])
    posed = call(bridge, "set_camera", eye=[1, 2, 3], direction=[0, 1, 0])
    assert posed["target"] == pytest.approx([1, 6, 3])
    turned = call(bridge, "set_camera", turn=[90, 0])
    assert turned["eye"] == pytest.approx(posed["eye"])
    moved = call(bridge, "set_camera", translation=[3, 2, 1])
    assert moved["target"] == pytest.approx([8, 4, 4])
    assert len(win.viewport.history.undo_stack) == depth
    assert before["eye"] == pytest.approx([0, 0, 2])


def test_screenshot_frames_shared_camera_and_captures_metal(model, monkeypatch, tmp_path):
    from built_render import SurveyRaster
    win, bridge, _, child, _ = model
    targets = []

    def capture(w, h, **options):
        targets.append(win.viewport.camera.target.x())
        image = solid_raster(w, h, (18, 52, 86))
        return image

    monkeypatch.setattr(win.view, "render_image", capture)
    monkeypatch.setattr("plugins.ai_bridge.tempfile.gettempdir", lambda: str(tmp_path))
    for args in ({"uids": [child.uid]}, {}):
        shot = call(bridge, "screenshot", width=90, height=65, **args)
        image = SurveyRaster.open(shot["path"])
        assert (image.width, image.height, image.rgba()[:4]) == (90, 65, bytes((18, 52, 86, 255)))
    assert 10 <= targets[0] <= 12
    assert targets[0] == targets[1]


def test_existing_nested_metadata_and_mesh_are_transactional(model):
    win, bridge, parent, child, layer = model
    original = child.mesh.capture_state()
    def edit(fail=False):
        child.name = "Edited"
        child.ifc["custom"]["a"] = 7
        child.mesh.add_edge(V(0, 0, 0), V(5, 5, 5))
        layer.name = "Renamed"
        win.viewport.scene.plugin_data["demo"] = {"value": 3}
        if fail:
            raise ValueError("rollback")
        return True
    ExtensionApp(win, "demo").add_agent_tool("edit", edit, description="Edit", mutates=True)
    bridge.start()
    assert not _ask(bridge, "edit", {"fail": True})["ok"]
    assert child.name == "Child" and child.ifc["custom"]["a"] == 1
    assert child.mesh.capture_state() == original
    assert layer.name == "Walls" and not win.viewport.scene.plugin_data
    assert call(bridge, "edit")
    call(bridge, "undo")
    assert child.name == "Child" and child.ifc["custom"]["a"] == 1
    assert child.mesh.capture_state() == original
    call(bridge, "redo")
    assert child.name == "Edited" and child.ifc["custom"]["a"] == 7


def test_noop_mutating_tool_preserves_redo(model):
    win, bridge, *_ = model
    app = ExtensionApp(win, "demo")
    app.add_agent_tool("nothing", lambda: None, description="Nothing", mutates=True)
    before = len(win.viewport.history.undo_stack)
    assert call(bridge, "nothing") is None
    assert len(win.viewport.history.undo_stack) == before


def test_tree_walk_is_cached_until_the_document_changes():
    from types import SimpleNamespace
    from core.agent_inspection import InspectionTools
    from built_core.group import Group
    from built_core.mesh import Mesh
    from built_core.scene import Scene
    scene = Scene()
    scene.groups.append(Group(Mesh(), 'A'))
    tools = InspectionTools()
    tools._viewport = SimpleNamespace(scene=scene)
    first = tools._groups()
    assert tools._groups() is first
    scene.groups.append(Group(Mesh(), 'B'))
    scene.version += 1
    rows = tools._groups()
    assert rows is not first and [g.name for g, _, _ in rows] == ['A', 'B']
