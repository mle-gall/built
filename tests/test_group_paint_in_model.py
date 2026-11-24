# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""A material painted on a group shows in the Library’s «In the model» (#133,
@fafecm: it only appeared once the group was exploded)."""
from __future__ import annotations

from ingetrazo_core import Vec3 as V
from tests.shell_isolation import restore_shell_patches  # noqa: F401


def test_a_painted_group_puts_its_material_in_the_model_list(native_host):
    from core.group import Group
    from core.mesh import Mesh
    from tools.paint import PaintTool
    from ingetrazo_render import wgpu_library as library

    win = native_host.window
    ctx = library.Ctx(win, win.view)
    mesh = Mesh()
    mesh.add_face([V(0, 0, 0), V(1, 0, 0), V(1, 1, 0), V(0, 1, 0)])
    group = Group(mesh, name="Caja")
    group.material = {"color": [0.8, 0.2, 0.2], "mat": "Ladrillo rojo", "opacity": 0.75}
    ctx.scene.groups.append(group)
    ctx.scene.version += 1
    provider = library.MaterialsProvider()
    row, = [r for r in provider._model_items(ctx) if r["name"] == "Ladrillo rojo"]
    assert (row["base"], row["hex"], row["uses"]) == ("colour", "#cc3333", 1)
    provider.run(ctx, "select", row)
    assert ctx.scene.selection == {group}
    provider.run(ctx, "apply", row)
    assert PaintTool.current_material.name == "Ladrillo rojo"
    assert PaintTool.current_material.opacity == 0.75
    assert "Ladrillo rojo" not in ctx.scene.materials

def test_closing_a_document_releases_its_material_survey():
    import gc
    import weakref
    from core.scene import Scene
    from ingetrazo_core import material_survey

    scene = Scene()
    mesh = scene.loose_mesh
    mesh.plugin_owner = scene
    material_survey(scene)
    reference = weakref.ref(scene)
    del mesh, scene
    gc.collect()
    assert reference() is None
