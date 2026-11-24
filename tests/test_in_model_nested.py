# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The tray's «In model» list counts the copies nested inside groups (#599,
@chakkreetiya: a component put into a group from the Browser dropped off
the list), and Select Instances opens the group that holds them, as the
Browser does, since the view selects inside one open group at a time."""
from __future__ import annotations

from tests.shell_isolation import restore_shell_patches  # noqa: F401
from views.wgpu_library import ComponentsProvider, Ctx
from ingetrazo_core import Matrix4 as QMatrix4x4, Vec3 as V

from core.group import Group, copy_group
from core.mesh import Mesh


def _quad(size=1.0, z=0.0):
    m = Mesh()
    m.add_face([V(0, 0, z), V(size, 0, z), V(size, size, z), V(0, size, z)])
    return m


def _component(mesh, name, dx=0.0):
    g = Group(mesh, name=name)
    xf = QMatrix4x4()
    xf.translate(dx, 0.0, 0.0)
    g.xform = xf
    return g


def _rows(win):
    provider = ComponentsProvider()
    ctx = Ctx(win, win.view)
    return {item["name"]: item for item in provider.items(ctx) if item["source"] == "model"}


def _select(win, row):
    ComponentsProvider().run(Ctx(win, win.view), "select", row)


def test_a_copy_inside_a_group_is_listed(native_host):
    win = native_host.window
    scene = win.viewport.scene
    varilla = _component(_quad(), "Varilla")
    muro = Group(_quad(3.0), name="Muro")
    muro.children = [varilla]
    scene.groups.append(muro)
    rows = _rows(win)
    assert "Varilla" in rows
    assert rows["Varilla"]["uses"] == 1
    _select(win, rows["Varilla"])
    assert set(scene.selection) == {varilla}


def test_copies_at_the_top_and_nested_make_one_row(native_host):
    win = native_host.window
    scene = win.viewport.scene
    silla = _component(_quad(), "Silla")
    arriba = copy_group(silla, V(5, 0, 0))
    dentro = copy_group(silla, V(10, 0, 0))
    mesa = Group(_quad(2.0), name="Comedor")
    mesa.children = [dentro]
    scene.groups.extend([silla, arriba, mesa])
    rows = _rows(win)
    assert rows["Silla"]["uses"] == 3
    assert set(ComponentsProvider()._row(Ctx(win, win.view), rows["Silla"])[2]) == {silla, arriba, dentro}
    assert "Comedor" not in rows            # a plain group: no row


def test_a_component_inside_each_copy_of_another_is_one_row(native_host):
    """Each copy of «Caja» carries its «Tornillo»: one row, a copy per box,
    and Select Instances takes the one in the first box (a tie)."""
    win = native_host.window
    scene = win.viewport.scene
    tornillo = _component(_quad(0.2), "Tornillo")
    caja = _component(_quad(2.0), "Caja")
    caja.children = [tornillo]
    otra = copy_group(caja, V(5, 0, 0))
    scene.groups.extend([caja, otra])
    rows = _rows(win)
    assert rows["Tornillo"]["uses"] == 2
    assert all(g.mesh is tornillo.mesh for g in ComponentsProvider()._row(Ctx(win, win.view), rows["Tornillo"])[2])
    assert rows["Caja"]["uses"] == 2
    _select(win, rows["Tornillo"])
    assert scene.edit_group is caja
    assert set(scene.selection) == {tornillo}


def test_select_instances_opens_the_group_that_holds_them(native_host):
    win = native_host.window
    scene = win.viewport.scene
    varilla = _component(_quad(), "Varilla")
    otra = copy_group(varilla, V(2, 0, 0))
    muro = Group(_quad(3.0), name="Muro")
    muro.children = [varilla, otra]
    scene.groups.append(muro)
    rows = _rows(win)
    _select(win, rows["Varilla"])
    assert scene.edit_group is muro
    assert set(scene.selection) == {varilla, otra}


def test_copies_in_several_groups_select_where_most_are(native_host):
    win = native_host.window
    scene = win.viewport.scene
    silla = _component(_quad(), "Silla")
    arriba = copy_group(silla, V(5, 0, 0))
    dentro = copy_group(silla, V(10, 0, 0))
    comedor = Group(_quad(2.0), name="Comedor")
    comedor.children = [dentro]
    scene.groups.extend([silla, arriba, comedor])
    rows = _rows(win)
    _select(win, rows["Silla"])
    assert scene.edit_group is None         # two at the top beat one
    assert set(scene.selection) == {silla, arriba}
    message = win.statusBar().currentMessage()
    assert "2" in message and "1" in message


def test_a_tie_keeps_the_group_already_open(native_host):
    win = native_host.window
    scene = win.viewport.scene
    silla = _component(_quad(), "Silla")
    dentro = copy_group(silla, V(10, 0, 0))
    comedor = Group(_quad(2.0), name="Comedor")
    comedor.children = [dentro]
    scene.groups.extend([silla, comedor])
    win.viewport.begin_group_edit(comedor)
    rows = _rows(win)
    _select(win, rows["Silla"])
    assert scene.edit_group is comedor
    assert set(scene.selection) == {dentro}


def test_a_locked_group_around_them_is_not_opened(native_host):
    win = native_host.window
    scene = win.viewport.scene
    varilla = _component(_quad(), "Varilla")
    muro = Group(_quad(3.0), name="Muro")
    muro.children = [varilla]
    muro.locked = True
    otro = Group(_quad(1.0), name="Suelo")
    scene.groups.extend([muro, otro])
    scene.selection.add(otro)
    rows = _rows(win)
    _select(win, rows["Varilla"])
    assert scene.edit_group is None
    assert set(scene.selection) == {otro}   # untouched


def test_all_at_the_top_select_as_before(native_host):
    from core.i18n import tr
    win = native_host.window
    scene = win.viewport.scene
    silla = _component(_quad(), "Silla")
    scene.groups.extend([silla, copy_group(silla, V(5, 0, 0))])
    rows = _rows(win)
    _select(win, rows["Silla"])
    assert scene.edit_group is None
    assert len(scene.selection) == 2
    assert win.statusBar().currentMessage() == tr(
        "{n} entities selected", n=2)
