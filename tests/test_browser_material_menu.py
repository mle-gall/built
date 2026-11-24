# SPDX-License-Identifier: GPL-3.0-or-later
"""Paint offered by the Project row menu includes recent and model swatches."""
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from core.materials import Material
from tests.test_material_popup import choose, open_picker, rebuild, rows, tiles
from tools.paint import PaintTool


def test_empty_model_and_recent_deduplication(native_host, rows, monkeypatch):
    a = rows[0]
    scene = native_host.viewport.scene
    scene.materials = {}
    monkeypatch.setattr(PaintTool, "recent", [])
    assert tiles(open_picker(native_host, a)) == []
    scene.materials["Oak"] = Material("Oak", color=(.8,.6,.3))
    red = {"color":[1.,0.,0.]}
    pine = {"color":[.8,.6,.3], "mat":"Pine"}
    monkeypatch.setattr(PaintTool, "recent", [dict(pine, mat="Oak"), red, pine, red])
    wire = open_picker(native_host, a)
    assert [t["id"] for t in tiles(wire)] == ["m:Oak"]
    choose(native_host, "side", "s:recent")
    found = tiles(rebuild(native_host))
    assert sorted(t["tip"] for t in found) == ["#ff0000", "Pine"]
    choose(native_host, "open", next(t["id"] for t in found if t["tip"]=="Pine"))
    assert a.material["mat"] == "Pine" and scene.materials["Pine"].color == (.8,.6,.3)
    assert native_host.viewport.history.undo() and "Pine" not in scene.materials


def test_texture_and_pbr_apply_to_all_rows_in_one_step(native_host, rows):
    vp = native_host.viewport
    a,b,c = rows
    tex = {"path":"brick.png","sw":.5,"sh":.25}
    mat = Material("Brick", texture=tex, opacity=.4, pbr={"roughness":.5})
    vp.scene.materials[mat.name] = mat
    vp.scene.select([a,b])
    open_picker(native_host, a)
    before = vp.history.revision
    choose(native_host, "open", "m:Brick")
    assert vp.history.revision == before + 1 and a.material == b.material
    assert a.material["texture"] == tex and a.material["pbr"] == mat.pbr
    assert a.material["opacity"] == .4 and c.material is None
    assert vp.history.undo() and a.material is None and b.material is None
