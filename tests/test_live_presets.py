# SPDX-License-Identifier: GPL-3.0-or-later
from copy import deepcopy

from built_core import command_protocol as live
from built_core.history import History
from built_core.protocol import commands
from built_core.scene import Scene
from formats import igz
from tests.test_live_recipes import apply
from tests.test_persistent_ids import all_ids


def steel():
    return next(p for p, _ in live.preset_catalog() if p["name"] == "Steel rafter")


def test_preset_edit_is_one_step_and_roundtrips(tmp_path):
    scene = Scene()
    h = History(scene)
    preset = steel()
    uid = preset["uid"]
    member = dict(preset_uid=uid, path=dict(points=[[0, 0, 0], [4, 0, 0]]))
    apply(h, "create_sweeps", sweeps=[member, dict(member, rotation=90)])
    a, b = scene.groups
    assert a.ifc["material"] == "S355"
    assert a.ifc["class"] == "IfcBeam"
    assert a.name != b.name
    meshes = [a.mesh, b.mesh]
    ids = all_ids(a.mesh)
    preset["values"]["profile"]["library"] = "en/ipe200"
    preset["record"]["material"] = "S275"
    apply(h, "preset_update", rows=[preset])
    assert a.ifc["material"] == b.ifc["material"] == "S275"
    assert all_ids(a.mesh) == ids
    assert live.live_sweep(scene, b.uid)["rotation"] == 90
    assert live.live_sweep(scene, a.uid)["preset_rev"] == 2
    apply(h, "undo")
    assert a.mesh is meshes[0] and b.mesh is meshes[1]
    assert a.ifc["material"] == "S355"
    apply(h, "redo")
    path = tmp_path / "presets.built"
    igz.save_scene(scene, path)
    opened = Scene()
    igz.load_into(opened, path)
    assert live.live_sweep(opened, b.uid)["preset_uid"] == uid
    assert next(p for p, src in live.preset_catalog(opened) if src == "model")["rev"] == 2
    assert all_ids(opened.groups[0].mesh) == ids


def test_fixed_edit_detaches_and_invalid_pack_rolls_back():
    scene = Scene()
    h = History(scene)
    p = steel()
    apply(h, "create_sweeps", sweeps=[dict(preset_uid=p["uid"], path=dict(points=[[0, 0, 0], [1, 0, 0]]))])
    g, = scene.groups
    row = live.live_sweep(scene, g.uid)
    apply(h, "live_set", row=dict(row, rotation=30))
    assert live.live_sweep(scene, g.uid)["preset_uid"] == p["uid"]
    apply(h, "live_set", row=dict(row, profile=dict(library="basic/timber")))
    assert live.live_sweep(scene, g.uid).get("preset_uid") is None
    assert "profile" in g.ifc["preset_note"]
    apply(h, "undo")
    assert live.live_sweep(scene, g.uid)["preset_uid"] == p["uid"]
    assert not commands.run(h, "preset_delete", uid=p["uid"])["ok"]
    invalid = deepcopy(p)
    invalid["locks"]["rotation"] = "bad"
    assert not commands.run(h, "preset_update", rows=[dict(p, name="Changed"), invalid])["ok"]
    assert live.live_sweep(scene, g.uid)["preset_rev"] == 1


def test_future_primitive_slots_and_unused_preset_save(tmp_path):
    scene = Scene()
    h = History(scene)
    rows = [dict(steel(), uid=f"{i:016x}", primitive=kind, name=kind, values=values)
            for i, (kind, values) in enumerate([("cut", {"operation": "trim"}), ("repeat", {"rule": {"divide": 4}})], 1)]
    apply(h, "preset_update", rows=rows)
    path = tmp_path / "slots.built"
    igz.save_scene(scene, path)
    other = Scene()
    igz.load_into(other, path)
    assert {p["primitive"] for p, src in live.preset_catalog(other) if src == "model"} == {"cut", "repeat"}
    apply(h, "preset_delete", uid=rows[0]["uid"])
    apply(h, "undo")
    assert len([p for p, src in live.preset_catalog(scene) if src == "model"]) == 2


def test_capture_values_feed_tool_and_active_node_default():
    from built_core import ToolMachine
    from core import bim_registry
    from tests.test_structure import model

    scene, registry, *_ = model()
    registry["structure"]["active"] = "ref"
    scene.plugin_data["bim_registry"] = registry
    history = History(scene)
    p = steel()
    tool = ToolMachine("sweep")
    assert tool.set("preset", p)
    apply(history, "create_sweeps", sweeps=[dict(preset_uid=p["uid"], path={"points": [[0, 0, 0], [3, 0, 0]]})])
    assert bim_registry.get(scene)["structure"]["nodes"]["ref"]["recipe"]["default_preset"] == p["uid"]
    apply(history, "undo")
    assert "default_preset" not in bim_registry.get(scene)["structure"]["nodes"]["ref"]["recipe"]


def test_library_browse_and_duplicate(tmp_path, monkeypatch):
    from types import SimpleNamespace
    from built_render.wgpu_library import PresetsProvider

    monkeypatch.setenv("BUILT_SETTINGS_DIR", str(tmp_path))
    scene = Scene()
    history = History(scene)
    ctx = SimpleNamespace(scene=scene, core=lambda key, **kw: commands.run(history, key, **kw),
                          vp=SimpleNamespace(notify_scene_changed=lambda: None))
    provider = PresetsProvider()
    all_items = provider.items(ctx)
    assert {e["category"] for e in all_items} >= {"MEP", "Architecture", "Steel AISC", "Timber C16/C24"}
    pipe = next(e for e in all_items if e["name"] == "Pipe")
    assert pipe["preset"]["record"]["class"] == "IfcPipeSegment"
    provider.run(ctx, "duplicate", pipe)
    copy, = [p for p in provider.items(ctx) if p["source"] == "model"]
    assert copy["key"] != pipe["key"] and copy["name"] == "Pipe copy"
    provider.confirmed(ctx, "delete", copy["key"], "")
    assert not [p for p in provider.items(ctx) if p["source"] == "model"]
