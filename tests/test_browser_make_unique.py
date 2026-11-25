# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The Browser's right-click on component rows (#87, #620): «Make Unique»,
every chosen copy a group of its own with a name of its own, as ONE undo
step; «Select Instances (n)», every copy of the same components — nested
ones too — selected in the view. Inside a component, Make Unique is greyed
with the reason."""
from __future__ import annotations

from tests.native_tree import action, group_rows
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from built_core import Matrix4 as QMatrix4x4, Vec3 as V

from built_core.group import Group, copy_group
from built_core.i18n import tr
from built_core.mesh import Mesh


def _quad(x=0.0):
    m = Mesh()
    m.add_face([V(x, 0, 0), V(x + 1, 0, 0), V(x + 1, 1, 0), V(x, 1, 0)])
    return m


def _setup(scene):
    silla = Group(_quad(), name="Silla")
    silla.xform = QMatrix4x4()
    b = copy_group(silla, V(2, 0, 0))
    c = copy_group(silla, V(4, 0, 0))
    comedor = Group(_quad(8.0), name="Comedor")
    comedor.xform = QMatrix4x4()
    comedor.component = False
    d = copy_group(silla, V(6, 0, 0))
    comedor.children = [d]                      # a copy inside a group
    muro = Group(_quad(12.0), name="Muro")
    scene.groups[:] = [silla, b, c, comedor, muro]
    scene.version += 1
    return silla, b, c, d, comedor, muro


def test_the_menu_offers_both_on_a_component_and_neither_on_a_group(native_host):
    silla, b, c, d, comedor, muro = _setup(native_host.viewport.scene)
    entries = group_rows(native_host, b)
    assert entries[tr("Make Unique")] == ""
    assert tr("Select Instances ({n})", n=4) in entries
    assert tr("Make Unique") not in group_rows(native_host, muro)


def test_make_unique_on_several_rows_is_one_undo_step_with_own_names(native_host):
    vp = native_host.viewport
    silla, b, c, *_ = _setup(vp.scene)
    vp.scene.select([b])
    action(native_host, "toggleitem", "i:" + c.uid)
    revision = vp.history.revision
    action(native_host, "unique", "i:" + b.uid)
    assert b.is_component() and c.is_component()
    assert b.mesh is not silla.mesh and c.mesh is not silla.mesh
    assert b.mesh is not c.mesh
    assert {b.name, c.name} == {"Silla #2", "Silla #3"}
    assert silla.is_component() and silla.name == "Silla"
    assert vp.history.revision == revision + 1
    assert vp.history.undo()
    assert b.mesh is silla.mesh and c.mesh is silla.mesh
    assert b.name == c.name == "Silla"


def test_select_instances_takes_every_copy_nested_ones_too(native_host, monkeypatch):
    vp = native_host.viewport
    scene = vp.scene
    silla, b, c, d, comedor, muro = _setup(scene)
    said = []
    monkeypatch.setattr(vp, "flash_status", lambda text, *args: said.append(text))
    vp.begin_group_edit(comedor)
    action(native_host, "instances", "i:" + d.uid)
    assert scene.edit_group is None
    assert set(scene.selection) == {silla, b, c}
    assert said[-1] == tr("{n} copies selected; {m} more are inside other groups", n=3, m=1)


def test_inside_a_component_make_unique_is_greyed_with_the_reason(native_host, monkeypatch):
    vp = native_host.viewport
    tornillo = Group(_quad(20.0), name="Tornillo")
    tornillo.xform = QMatrix4x4()
    mueble = Group(_quad(18.0), name="Mueble")
    mueble.xform = QMatrix4x4()
    mueble.children = [tornillo]
    vp.scene.groups.append(mueble)
    vp.scene.version += 1
    why = tr("Components keep their parts: Make Unique first to change what is inside one")
    assert group_rows(native_host, tornillo)[tr("Make Unique")] == why
    said = []
    monkeypatch.setattr(vp, "flash_status", lambda text, *args: said.append(text))
    revision = vp.history.revision
    action(native_host, "unique", "i:" + tornillo.uid)
    assert said == [why] and tornillo.is_component()
    assert vp.history.revision == revision


def test_collapsed_definition_uses_only_the_copies_in_its_branch(native_host):
    from built_render import tree_group_rows

    scene = native_host.viewport.scene
    a, b, c, nested, *_ = _setup(scene)
    native_host.view._ui_tree = {"unfiled": [a, b, c]}
    key = f"d::{id(a.mesh)}"
    assert {verb for verb, _, _ in tree_group_rows(scene, [a, b, c])} == {"instances", "unique", "paintrows"}
    action(native_host, "unique", key)
    assert len({id(g.mesh) for g in (a, b, c, nested)}) == 4
    assert native_host.viewport.history.undo()
    assert all(g.mesh is nested.mesh for g in (a, b, c))
