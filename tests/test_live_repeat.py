# SPDX-License-Identifier: GPL-3.0-or-later
from built_core.protocol import queries
from built_core.scene import Scene
from formats import igz
from tests.test_live_recipes import setup_groups, apply, box, all_ids


def repeat(source, **fields):
    return dict(kind="repeat", source=source.uid, uid="0000000000000002",
                run={"points": [[0, 0, 0], [2, 0, 0]]}, rule={"divide": 4},
                **fields)


def test_live_repeat_instances_history_and_file(tmp_path):
    scene, history, (part,) = setup_groups(1)
    apply(history, "live_set", row=box(part))
    row = repeat(part, skips=[{"from": "start", "at": 1000}])
    apply(history, "live_set", row=row)
    assembly = scene.groups[-1]
    copies = list(assembly.children)
    assert len(copies) == 3
    assert all(c.mesh is part.mesh for c in copies)
    uids = [c.uid for c in copies]
    assert len(set(uids)) == 3
    apply(history, "live_set", row=dict(row, rule={"spacing": 500}))
    assert [c.uid for c in assembly.children] == uids
    apply(history, "live_set", row=box(part, size=[4, 2, 1]))
    assert all(c.mesh is part.mesh for c in assembly.children)
    assert all_ids(copies[0].mesh) == all_ids(part.mesh)
    apply(history, "undo")
    assert assembly.children[0].mesh is part.mesh
    apply(history, "redo")
    path = tmp_path / "repeat.built"
    igz.save_scene(scene, path)
    back = Scene()
    igz.load_into(back, path)
    assert queries.live_rows(back)[-1]["state"] == "ok"
    assert [c.uid for c in back.groups[-1].children] == uids
    apply(history, "live_release", uid=row["uid"])
    assert [c.uid for c in assembly.children] == uids
    assert len(queries.live_rows(scene)) == 1


def test_bad_rule_keeps_last_geometry():
    scene, history, (part,) = setup_groups(1)
    row = repeat(part)
    apply(history, "live_set", row=row)
    assembly = scene.groups[-1]
    old = list(assembly.children)
    apply(history, "live_set", row=dict(row, rule={"divide": 0}))
    assert queries.live_rows(scene)[0]["state"] == "invalid_rule"
    assert list(assembly.children) == old


def test_repeated_cutter_applies_source_and_each_station():
    import pytest
    from built_core.history import History
    from built_core.solids import solid_volume
    from tests.test_live_cut import make_member

    scene = Scene()
    history = History(scene)
    make_member(history, [[0, 0, 0], [4, 0, 0]])
    make_member(history, [[1, -1, 0], [1, 1, 0]])
    beam, tool = scene.groups
    original = solid_volume(beam)
    row = repeat(tool)
    row["rule"] = {"divide": 2}
    apply(history, "live_set", row=row)
    apply(history, "live_set", row=dict(kind="cut", uid="0000000000000003", owner=beam.uid,
                                        cutter={"repeat": row["uid"]}, operation="subtract"))
    assert solid_volume(beam) == pytest.approx(original - 3 * .045 ** 2 * .145)
    apply(history, "undo")
    assert solid_volume(beam) == pytest.approx(original)


def test_polar_stations_keep_the_source_and_its_rotation():
    import pytest
    scene, history, (source,) = setup_groups(1)
    apply(history, "move_placements", placements=[source.uid], delta=[2, 0, 0])
    row = repeat(source)
    row.update(run={"polar": {"centre": [0, 0, 0], "axis": [0, 0, 1], "angle": -90}},
               rule={"divide": 3})
    apply(history, "live_set", row=row)
    last = scene.groups[-1].children[-1]
    assert last.xform.data()[12:15] == pytest.approx([0, -2, 0], abs=1e-12)
    assert last.xform.data()[:3] == pytest.approx([0, -1, 0], abs=1e-12)


def test_repeat_exports_shared_body_and_stable_occurrences(tmp_path):
    import pytest
    ifc = pytest.importorskip("ifcopenshell")
    from formats.ifc import save_ifc
    scene, history, (source,) = setup_groups(1)
    source.ifc = {"class": "IfcMember", "name": "Strut"}
    apply(history, "live_set", row=box(source))
    apply(history, "live_set", row=repeat(source))
    path = tmp_path / "struts.ifc"
    guids = None
    for _ in range(2):
        save_ifc(scene, path)
        model = ifc.open(str(path))
        members = model.by_type("IfcMember")
        assert len(members) == 5
        ids = {m.GlobalId for m in members}
        assert len(ids) == 5
        if guids is not None:
            assert ids == guids
        guids = ids
        maps = model.by_type("IfcMappedItem")
        assert len(maps) >= 4
        assert len({m.MappingSource.id() for m in maps}) == 1


def test_between_updates_after_host_count_changes():
    scene, history, (upright, diagonal) = setup_groups(2)
    host = repeat(upright)
    apply(history, "live_set", row=host)
    between = dict(repeat(diagonal), uid="0000000000000003",
                   run={"between": host["uid"]}, orientation="alternate")
    apply(history, "live_set", row=between)
    assembly = scene.groups[-1]
    assert [g.xform.data()[12] for g in assembly.children] == [.25, .75, 1.25, 1.75]
    assert [g.xform.data()[0] for g in assembly.children] == [-1, 1, -1, 1]
    apply(history, "live_set", row=dict(host, rule={"divide": 2}))
    assert [g.xform.data()[12] for g in assembly.children] == [.5, 1.5]


def test_nested_repeat_includes_first_member_and_stable_children():
    scene, history, (part,) = setup_groups(1)
    apply(history, "live_set", row=box(part))
    inner = repeat(part)
    apply(history, "live_set", row=inner)
    portal = scene.groups[-1]
    outer = dict(repeat(portal), uid="0000000000000007", rule={"divide": 2})
    apply(history, "live_set", row=outer)
    nested = scene.groups[-1]
    assert [len(g.children) for g in nested.children] == [5, 5]
    old = [{g.uid for g in station.children} for station in nested.children]
    apply(history, "live_set", row=dict(inner, rule={"divide": 3}))
    assert [len(g.children) for g in nested.children] == [4, 4]
    assert all({g.uid for g in station.children} < prev for station, prev in zip(nested.children, old))


def test_arraying_existing_cutter_updates_the_cut_and_undo():
    import pytest
    from built_core.history import History
    from built_core.solids import solid_volume
    from tests.test_live_cut import make_member
    scene = Scene()
    history = History(scene)
    make_member(history, [[0, 0, 0], [4, 0, 0]])
    make_member(history, [[1, -1, 0], [1, 1, 0]])
    beam, tool = scene.groups
    original = solid_volume(beam)
    apply(history, "live_set", row=dict(kind="cut", uid="0000000000000003", owner=beam.uid,
                                        cutter={"body": tool.uid}, operation="subtract"))
    one = original - solid_volume(beam)
    apply(history, "live_set", row=dict(repeat(tool), rule={"divide": 2}))
    assert original - solid_volume(beam) == pytest.approx(3 * one)
    apply(history, "undo")
    assert original - solid_volume(beam) == pytest.approx(one)


def test_bound_edge_redivides_and_preserves_station_uids():
    import pytest
    from built_core import element_ids
    scene, history, (host, source) = setup_groups(2)
    apply(history, "live_set", row=box(host))
    edge = next(e for e in host.mesh.edges if abs(e.a.x() - e.b.x()) > .9)
    pid = host.mesh._store.id_of("edge", edge.key)
    bound = {"edge": {"path": [host.uid], "id": element_ids.id_text("edge", pid)}}
    apply(history, "live_set", row=dict(repeat(source), run=bound))
    assembly = scene.groups[-1]
    old = [g.uid for g in assembly.children]
    before = assembly.children[-1].xform.data()[12]
    apply(history, "live_set", row=box(host, size=[3, 1, 1]))
    assert assembly.children[-1].xform.data()[12] == pytest.approx(3 * before)
    assert [g.uid for g in assembly.children] == old


def test_polar_copy_field_spacing_and_fit_share_the_array_window():
    from tests.test_rotate_array import _Vp, _rotate_copy
    from tools.rotate import RotateTool
    scene, history, (source,) = setup_groups(1)
    apply(history, "live_set", row=box(source))
    scene.selection = [source]
    vp = _Vp(scene)
    tool = RotateTool()
    _rotate_copy(vp, tool, 90)
    assert tool.on_value(vp, 30.)
    first = scene.groups[-1]
    uids = [c.uid for c in first.children]
    assert len(uids) == 3
    assert tool.on_array_value(vp, 1, "fit_max", step=24.)
    assert len(scene.groups[-1].children) == 4
    assert [c.uid for c in scene.groups[-1].children][:3] == uids
    assert scene.groups[-1].uid == first.uid
    vp.history.undo()
    assert scene.groups == [source]


def test_notched_adaptive_parts_match_the_boolean_kernel(tmp_path):
    import pytest
    from built_core.group import Group
    from built_core.history import History
    from built_core.solids import run, solid_volume
    from scripts.repeat_storyboard import build

    build(tmp_path)
    scene = Scene()
    igz.load_into(scene, tmp_path / "portal.built")
    history = History(scene)
    row = next(r for r in queries.live_rows(scene) if r["uid"] == f"{210:016x}")
    assembly = next(g for g in scene.groups if g.uid == row["owner"])
    stocks = []
    for placed in assembly.children:
        stock = Group(placed.mesh)
        stock.xform = placed.xform
        stocks.append(stock)
    apply(history, "create_sweeps", sweeps=[{
        "profile": {"library": "basic/timber", "params": {"width": 120, "height": 55}},
        "path": {"points": [[-.2, .3, 3.39], [3.65, .3, 4.82]]}, "attachment": "centre",
    }])
    tool = scene.groups[-1]
    apply(history, "live_set", row=dict(kind="cut", owner=row["source"], uid=f"{399:016x}",
                                        operation="subtract", cutter={"body": tool.uid}))
    for stock, station in zip(stocks, assembly.children, strict=True):
        _, (reference,) = run("subtract", [tool, stock])
        assert solid_volume(station) == pytest.approx(solid_volume(reference), rel=1e-7)
        assert len(station.mesh.faces) == len(reference.mesh.faces) > 6
        def points(group):
            return sorted(tuple(round(k, 6) for k in group.xform.map(v.position).toTuple())
                          for v in group.mesh.vertices)
        # The kernel returns a group in model space.
        from built_core import Matrix4
        if reference.xform is None:
            reference.xform = Matrix4()
        assert points(station) == points(reference)


def test_failed_adaptive_intersections_keep_their_last_mesh_and_recover():
    from built_core.history import History
    from tests.test_live_cut import make_member

    scene = Scene()
    history = History(scene)
    make_member(history, [[0, 0, 0], [0, 2, 0]])
    make_member(history, [[-.2, 1, 0], [5, 1, 0]])
    source, cutter = scene.groups
    apply(history, "live_set", row=dict(kind="cut", uid="0000000000000d01", owner=source.uid,
                                       operation="intersect", cutter={"body": cutter.uid}))
    row = dict(repeat(source), run={"points": [[0, 0, 0], [4, 0, 0]]})
    apply(history, "live_set", row=row)
    assembly = scene.groups[-1]
    good = [g.mesh for g in assembly.children]
    identities = [g.uid for g in assembly.children]
    recipe = next(r for r in queries.live_rows(scene) if r["kind"] == "sweep" and r["owner"] == cutter.uid)
    recipe["path"]["points"][-1][0] = 2.5
    apply(history, "live_set", row=recipe)
    assert [bool(g.ext.get("built.repeat_state")) for g in assembly.children] == [False, False, True, True]
    assert [g.mesh for g in assembly.children[2:]] == good[2:]
    recipe["path"]["points"][-1][0] = 5
    apply(history, "live_set", row=recipe)
    assert not any(g.ext.get("built.repeat_state") for g in assembly.children)
    assert [g.uid for g in assembly.children] == identities
    assert all(r["state"] == "ok" for r in queries.live_rows(scene))


def test_repeat_preset_follows_fixed_rule_then_detaches(tmp_path, monkeypatch):
    from types import SimpleNamespace
    from built_render.wgpu_library import PresetsProvider

    monkeypatch.setenv("BUILT_SETTINGS_DIR", str(tmp_path))
    scene, history, (source,) = setup_groups(1)
    apply(history, "live_set", row=repeat(source))
    assembly = scene.groups[-1]
    scene.selection = [assembly]
    ctx = SimpleNamespace(scene=scene, vp=SimpleNamespace(history=history, notify_scene_changed=lambda: None))
    preset = dict(uid="00000000000ad001", name="Four bays", pack="This project", primitive="repeat",
                  values={"rule": {"divide": 4}, "orientation": "alternate"}, record={"object_type": "Portal"},
                  locks={"rule": "fixed", "object_type": "fixed"}, rev=1)
    PresetsProvider().run(ctx, "apply", {"source": "mine", "preset": preset})
    current = lambda: next(r for r in queries.live_rows(scene) if r["kind"] == "repeat")
    assert current()["repeat_preset"]["uid"] == preset["uid"]
    assert assembly.ifc["object_type"] == "Portal"
    apply(history, "live_set", row=dict(current(), orientation="fixed"))
    apply(history, "preset_update", rows=[dict(preset, values={"rule": {"divide": 6}, "orientation": "alternate"})])
    assert current()["repeat_preset"]["rev"] == 2 and current()["orientation"] == "fixed"
    assert len(assembly.children) == 6
    path = tmp_path / "preset.built"
    igz.save_scene(scene, path)
    loaded = Scene()
    igz.load_into(loaded, path)
    assert queries.live_rows(loaded)[0]["repeat_preset"] == current()["repeat_preset"]
    apply(history, "live_set", row=dict(current(), rule={"count": 3}))
    assert not current().get("repeat_preset") and "rule" in assembly.ifc["preset_note"]
    apply(history, "undo")
    assert current()["repeat_preset"]["rev"] == 2
