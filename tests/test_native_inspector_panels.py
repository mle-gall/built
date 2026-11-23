"""Native action routing through the shell's real document controllers."""

import json
import pytest

from tests.shell_isolation import restore_shell_patches  # noqa: F401
from core.group import Group
from views.inspector_bridge import action


@pytest.fixture
def host(tmp_path):
    from app_shell.host import Host

    h = Host(0, 1280, 800, 1.0, settings=str(tmp_path))
    yield h
    h.close()


def run(h, verb, args=None):
    action(h.window, h.view, json.dumps([verb, args]))


def test_records_provenance_patch_undo(host):
    from core import bim_registry
    from core.protocol.records import occurrence
    from core.protocol import commands

    vp = host.viewport
    s = vp.scene
    s.plugin_data["bim_registry"] = {
        "types": {"beam": {"ifc_class": "IfcBeam", "psets": {"Survey": {"Inspected": False}}}},
        "structure": {
            "nodes": {"deck": {"name": "Deck", "parent": None, "recipe": {"class": "IfcBeam"}}}
        },
    }
    a, b = Group(), Group()
    for i, g in enumerate([a, b]):
        g.ext = {"structure": {"node": "deck"}}
        g.ifc = {"class": "IfcBeam", "type": "beam", "psets": {"Survey": {"Count": i + 1}}}
    s.groups.extend([a, b])
    s.select([a, b])
    rows = {tuple(f["path"]): f for f in occurrence(a, bim_registry.get(s))["fields"]}
    assert rows["class",]["owner"] == "Filing node"
    assert rows["psets", "Survey", "Inspected"]["owner"] == "Type"
    assert rows["psets", "Survey", "Count"]["owner"] == "This object"
    before = [g.ifc for g in (a, b)]
    rev = vp.history.revision
    run(host, "record.patch", [[a.uid, b.uid], ["psets", "Survey", "Inspected"], True])
    assert vp.history.revision == rev + 1
    for i, g in enumerate((a, b)):
        assert g.ifc["psets"]["Survey"]["Inspected"]["value"] is True
        assert g.ifc["psets"]["Survey"]["Count"]["value"] == i + 1
    assert a.ifc["guid"] != b.ifc["guid"]
    commands.undo(vp.history)
    assert [a.ifc, b.ifc] == before
    with pytest.raises(ValueError):
        commands.run(
            vp.history, "patch_record_field", placements=[a.uid], path=["class"], value="IfcWall"
        )


def test_camera_preview_then_capture(host):
    view = host.window.tray.scenes.add_scene()
    before = view.fov_deg
    run(host, "camera.open")
    run(host, "camera.fov", 52.0)
    run(host, "camera.frame_aspect", 16 / 9)
    assert host.viewport.camera.fov_deg == 52.0 and view.fov_deg == before
    run(host, "camera.update", view.name)
    assert view.fov_deg == 52.0
    assert view.lens["frame_aspect"] == pytest.approx(16 / 9)


def test_parts_visibility_explode_reassemble_split(host):
    from tests.test_parts import _stool, _box

    s = host.viewport.scene
    g = _stool()
    s.groups.append(g)
    s.select([g])
    child = g.children[1]
    run(host, "parts.visible", [g.uid, child.uid, False])
    assert child.hidden
    run(host, "parts.visible", [g.uid, child.uid, True])
    assert not child.hidden
    run(host, "parts.explode", [g.uid, 0.5, "x"])
    assert g.exploded["factor"] == 0.5
    run(host, "parts.explode", [g.uid, 0.0, "x"])
    assert not g.exploded
    joined = Group(_box(1, 1, 1))
    for face in _box(1, 1, 1, at=(2, 2, 2)).faces:
        joined.mesh.add_face(face.vertices)
    s.groups.append(joined)
    s.select([joined])
    run(host, "parts.split", joined.uid)
    assert len(joined.children) == 2


def test_native_bevel_preview_then_apply(host, monkeypatch):
    from tools.fillet import FilletTool
    from tests.test_fillet import box
    from PySide6.QtGui import QVector3D

    vp = host.viewport
    for f in box().faces:
        vp.scene.mesh.add_face(f.vertices)
    mesh = vp.scene.mesh
    edge = mesh.edges[0]
    vp.scene.select([edge])
    for key in ("radius", "segments", "shape", "profile", "affect", "measure", "corners"):
        monkeypatch.setattr(FilletTool, key, getattr(FilletTool, key))
    host.window._activate_tool("fillet")
    tool = vp.active_tool
    rev = vp.history.revision
    run(host, "bevel.segments", 4)
    run(host, "bevel.amount", "10 mm")
    assert tool.radius == pytest.approx(0.01)
    assert tool.segments == 4 and tool.preview_faces()
    assert vp.history.revision == rev
    run(host, "bevel.apply")
    assert vp.history.revision == rev + 1


def test_terrain_path_projects_cursor_samples_and_exports(host, tmp_path, monkeypatch):
    from georef.datum import SceneDatum
    from georef.geopath import GeoPath
    from tests.test_profile import V, StubSampler
    from views import terrain_binding
    from views.filedialogs import file_dialogs

    vp, view = host.viewport, host.view
    s = vp.scene
    s.georef = SceneDatum(48.0, 2.0, 100.0)
    path = GeoPath([V(0, 0), V(10, 0)], name="Route")
    s.geo_paths.append(path)
    s.select([path])
    from types import SimpleNamespace

    s.photo_mesh = SimpleNamespace(visible=True)
    monkeypatch.setattr(
        terrain_binding,
        "PhotoMeshSampler",
        lambda survey, datum: StubSampler(datum, lambda x, y: 100 + x),
    )
    sent = {}
    monkeypatch.setattr(terrain_binding, "_send", lambda view, key, val: sent.update({key: val}))
    monkeypatch.setattr(vp, "drape", lambda v: v)
    run(host, "terrain.open")
    assert sent["terrain"]["samples"][-1] == [10.0, 110.0]
    assert len(sent["projected"]) == len(sent["terrain"]["samples"])
    assert sent["projected"][4] != sent["projected"][5]
    csv = tmp_path / "route.csv"
    monkeypatch.setattr(file_dialogs, "getSaveFileName", lambda *a: (str(csv), ""))
    run(host, "terrain.csv")
    assert "photogrammetric survey" in csv.read_text() and "110.0" in csv.read_text()
    run(host, "terrain.close")
    assert sent["terrain"] is None


def test_filing_rule_preview_and_undo(host):
    from core import bim_registry
    from core.protocol import commands
    from core.protocol.records import filing_rule

    vp = host.viewport
    registry = bim_registry.seed_defaults()
    original = {"class": "IfcBeam"}
    registry["structure"] = {"nodes": {"deck": {"name": "Deck", "parent": None, "order": 0, "recipe": original}}}
    vp.scene.plugin_data["bim_registry"] = registry
    g = Group()
    g.ext = {"structure": {"node": "deck"}}
    g.ifc = {"class": "IfcBeam"}
    vp.scene.groups.append(g)
    updated = {"class": "IfcMember"}
    preview = filing_rule(vp.scene, "deck", updated, expected=original)
    assert preview["changed"] == 1 and g.ifc["class"] == "IfcBeam"
    commands.run(vp.history, "set_filing_rule", node="deck", recipe=updated, expected=original)
    assert g.ifc["class"] == "IfcMember"
    with pytest.raises(ValueError, match="changed"):
        commands.run(vp.history, "set_filing_rule", node="deck", recipe=updated, expected=original)
    commands.undo(vp.history)
    assert g.ifc["class"] == "IfcBeam"


def test_property_add_and_reset_without_losing_siblings(host):
    from core.protocol import commands

    vp = host.viewport
    g = Group()
    g.ifc = {"class": "IfcWall", "psets": {"Survey": {"Count": 4}}}
    vp.scene.groups.append(g)
    kwargs = dict(placements=[g.uid], path=["psets", "Survey", "Verified"])
    commands.run(vp.history, "patch_record_field", **kwargs, value=True, property_kind="boolean")
    assert g.ifc["psets"]["Survey"]["Verified"]["value"] is True
    commands.run(vp.history, "patch_record_field", **kwargs, value=None)
    assert set(g.ifc["psets"]["Survey"]) == {"Count"}
