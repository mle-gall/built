# SPDX-License-Identifier: GPL-3.0-or-later
"""Assembly commands from the native inspector. Every fixture is synthetic."""
from types import SimpleNamespace

from core.group import Group
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from tests.test_native_inspector_panels import host, run  # noqa: F401
from tests.test_parts import _box, _copies, _part, _sheet, _stool
from views import inspector_bridge


def select(host, group):
    scene = host.viewport.scene
    scene.groups.append(group)
    scene.select([group])
    return scene


def snapshot(host, monkeypatch):
    data = {}
    monkeypatch.setattr(inspector_bridge, "_send", lambda _view, key, value: data.update({key: value}))
    host.view._panel_selection_sig = None
    inspector_bridge._selection(host.view, host.window)
    return data


def test_the_tray_lists_selects_hides_and_renames_parts(host, monkeypatch):
    assembly = _stool()
    scene = select(host, assembly)
    data = snapshot(host, monkeypatch)
    assert [c["name"] for c in data["selection"]["parts"]["children"]] == ["seat", "leg A", "leg B"]
    seat, a, b = assembly.children
    run(host, "parts.select", [assembly.uid, a.uid])
    assert scene.edit_group is assembly and scene.selection == {a}
    inspector_bridge._selection(host.view, host.window)
    assert len(data["selection"]["parts"]["children"]) == 3
    run(host, "parts.visible", [assembly.uid, b.uid, False])
    run(host, "parts.rename", [assembly.uid, seat.uid, "Seat board"])
    assert b.hidden and seat.name == "Seat board"
    host.viewport.history.undo()
    assert seat.name == "seat"
    host.viewport.history.undo()
    assert not b.hidden


def test_a_plain_group_offers_to_split(host, monkeypatch):
    group = Group(_box(.1,.1,.1), name="Two pieces")
    for f in _box(.1,.1,.1, at=(1,0,0)).faces:
        group.mesh.add_face(f.vertices)
    select(host, group)
    data = snapshot(host, monkeypatch)
    parts = data["selection"]["parts"]
    assert parts["children"] == [] and parts["can_split"]
    run(host, "parts.split", group.uid)
    inspector_bridge._selection(host.view, host.window)
    assert [c["name"] for c in data["selection"]["parts"]["children"]] == ["Piece 1", "Piece 2"]


def test_all_parts_visible_shows_every_hidden_part(host):
    group = _stool()
    select(host, group)
    a, b = group.children[:2]
    for g in (a, b):
        run(host, "parts.visible", [group.uid, g.uid, False])
    steps = len(host.viewport.history.undo_stack)
    run(host, "parts.visible", [group.uid, None, True])
    assert not any(g.hidden for g in group.children)
    assert len(host.viewport.history.undo_stack) == steps + 1
    host.viewport.history.undo()
    assert {c for c in group.children if c.hidden} == {a,b}


def test_the_tray_shows_a_surface_as_an_area(host, monkeypatch):
    holder = Group(name="Unit")
    skin, shelf = _part(_sheet(.6,.135),"skin"), _part(_box(.6,.4,.018),"shelf")
    holder.adopt([skin, shelf])
    scene = select(host,holder)
    data = snapshot(host, monkeypatch)
    run(host, "parts.measure", [holder.uid, scene.content_version])
    rows = data["partMeasures"]["rows"]
    assert rows[skin.uid][0].endswith("surface")
    assert "×" in rows[shelf.uid][0] and not rows[shelf.uid][0].endswith("surface")


def test_clipped_parts_wait_to_measure_the_selected_group(host, monkeypatch):
    from core import parts
    calls = []
    original = parts.part_rows
    monkeypatch.setattr(parts,"part_rows", lambda g: calls.append(g) or original(g))
    g = _stool()
    scene = select(host,g)
    data = snapshot(host, monkeypatch)
    assert "partMeasures" not in data and calls == []
    run(host,"parts.measure", [g.uid, scene.content_version])
    assert calls == [g] and len(data["partMeasures"]["rows"]) == 3


def test_the_tray_reads_a_shared_definition_once_per_refresh(host, monkeypatch):
    from core import parts
    group = _copies(30)
    scene = select(host, group)
    data = snapshot(host, monkeypatch)
    seen = []
    original = parts._mesh_paint_areas
    def read(mesh, cache=None):
        seen.append(cache is not None and ("paint",id(mesh)) in cache)
        return original(mesh,cache)
    monkeypatch.setattr(parts,"_mesh_paint_areas",read)
    run(host,"parts.measure",[group.uid,scene.content_version])
    assert len(data["partMeasures"]["rows"]) == 30
    assert len(seen) == 30 and seen.count(False) == 1


def test_the_tray_slider_is_one_undo_step_per_drag(host):
    from ingetrazo_core import AssemblyPreview
    from tests.test_explode_view import _world, _assembly
    source, group = _assembly()
    source.groups.remove(group)
    scene = select(host, group)
    packed = SimpleNamespace(row_key=lambda c: c.uid)
    index = {c.uid:[i] for i,c in enumerate(group.children)}
    start = _world(group)
    steps = len(host.viewport.history.undo_stack)
    drag = AssemblyPreview(scene,group,packed,index,"outward")
    for factor in (.1,.4,.9,1.2):
        offsets = drag.offsets(factor)
        assert {i for i,_ in offsets} == set(range(len(group.children)))
        assert any(any(abs(x)>0 for x in d) for _,d in offsets)
        assert _world(group) == start and len(host.viewport.history.undo_stack) == steps
    run(host,"parts.explode",[group.uid,1.2,"outward"])
    assert group.exploded == {"factor":1.2,"mode":"outward"}
    assert len(host.viewport.history.undo_stack) == steps + 1
    run(host,"parts.explode",[group.uid,0.,"outward"])
    assert _world(group) == start
    host.viewport.history.undo()
    host.viewport.history.undo()
    assert _world(group) == start and group.exploded is None
