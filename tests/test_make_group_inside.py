# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Make Group and Make Component inside an open group (#490, @arifuw; #561,
@BastelBaus): what they make goes INTO the open group — they used to answer
«Leave the group first» — and the Browser's right-click offers both."""
from __future__ import annotations

from tests.shell_isolation import restore_shell_patches  # noqa: F401
from ingetrazo_core import Matrix4 as QMatrix4x4, Vec3 as V

from tests.native_tree import action, group_rows
from core import outliner
from core.group import Group, copy_group, world_mesh
from core.mesh import Mesh


def _quad(x=0.0, size=1.0, z=0.0):
    m = Mesh()
    m.add_face([V(x, 0, z), V(x + size, 0, z), V(x + size, size, z),
                V(x, size, z)])
    return m


def _placed(name, x=0.0):
    g = Group(_quad(), name=name)
    xf = QMatrix4x4()
    xf.translate(x, 0.0, 0.0)
    g.xform = xf
    g.component = False
    return g


def _points(group, groups):
    """World corners of ``group``'s faces, read from the model's tree."""
    m = outliner.world_matrix(groups, group)
    out = []
    for v in world_mesh(group).vertices:
        p = m.map(v.position) if m is not None and group.xform is None \
            else v.position
        out.append((round(p.x(), 6), round(p.y(), 6), round(p.z(), 6)))
    return sorted(out)


def _window(native_host):
    """A window on an empty model (it starts with a scale figure)."""

    win = native_host.window
    win.viewport.scene.groups.clear()
    win.viewport.scene.version += 1
    return win


def _open_wall(win):
    """A «Muro» group holding two placed groups, opened for editing."""
    scene = win.viewport.scene
    a, b = _placed("A", 2.0), _placed("B", 4.0)
    muro = Group(_quad(size=8.0, z=-1.0), name="Muro")
    xf = QMatrix4x4()
    xf.translate(10.0, 0.0, 0.0)            # a matrix on the container
    muro.xform = xf
    muro.component = False
    muro.children = [a, b]
    scene.groups.append(muro)
    win.viewport.begin_group_edit(muro)
    assert scene.edit_group is muro
    return muro, a, b


def test_grouping_two_groups_inside_an_open_group_keeps_it_inside(native_host):
    win = _window(native_host)
    scene = win.viewport.scene
    muro, a, b = _open_wall(win)
    before = {g.name: _points(g, scene.groups) for g in (a, b)}
    scene.selection.clear()
    scene.selection.update([a, b])
    win._on_make_group()
    assert len(muro.children) == 1
    nuevo = muro.children[0]
    assert nuevo.children == [a, b]
    assert nuevo not in scene.groups and scene.groups == [muro]
    # nothing moved in the world
    assert {g.name: _points(g, scene.groups) for g in (a, b)} == before
    win.viewport.history.undo()
    assert muro.children == [a, b]
    assert {g.name: _points(g, scene.groups) for g in (a, b)} == before


def test_grouping_loose_faces_inside_an_open_group(native_host):
    win = _window(native_host)
    scene = win.viewport.scene
    muro, a, b = _open_wall(win)
    faces = list(scene.mesh.faces)              # the wall's own face
    assert faces
    scene.selection.clear()
    scene.selection.update(faces)
    win._on_make_group()
    assert len(muro.children) == 3
    nuevo = [g for g in muro.children if g not in (a, b)][0]
    assert len(nuevo.mesh.faces) == 1
    assert not scene.mesh.faces                 # taken off the wall
    assert scene.groups == [muro]
    win.viewport.history.undo()
    assert muro.children == [a, b]
    assert len(scene.mesh.faces) == 1


def test_a_classic_group_opened_takes_the_new_group_as_a_child(native_host):
    """A plain group (no matrix, no children) gets the identity matrix a
    group holding groups always has, and stays a group."""
    win = _window(native_host)
    scene = win.viewport.scene
    m = _quad(size=2.0)
    m.add_face([V(5, 0, 0), V(6, 0, 0), V(6, 1, 0), V(5, 1, 0)])
    caja = Group(m, name="Caja")
    scene.groups.append(caja)
    win.viewport.begin_group_edit(caja)
    f = [f for f in scene.mesh.faces
         if min(v.position.x() for v in f.loop) >= 5][0]
    scene.selection.clear()
    scene.selection.add(f)
    win._on_make_group()
    assert len(caja.children) == 1
    assert caja.xform is not None and not caja.is_component()
    win.viewport.history.undo()
    assert caja.children == [] and caja.xform is None
    assert len(scene.mesh.faces) == 2


def test_make_component_inside_an_open_group(native_host):
    from ingetrazo_render import chrome_forms
    win = _window(native_host)
    scene = win.viewport.scene
    muro, a, b = _open_wall(win)
    scene.selection.clear()
    scene.selection.update(list(scene.mesh.faces))
    win._on_make_component()
    form = chrome_forms.current(win)
    form.receive("text", "Ladrillo")
    form.press("ok")
    comps = [g for g in muro.children if g.is_component()]
    assert [g.name for g in comps] == ["Ladrillo"]
    assert scene.groups == [muro]


def test_inside_a_component_it_still_refuses(native_host):
    win = _window(native_host)
    scene = win.viewport.scene
    comp = Group(_quad(size=2.0), name="Silla")
    comp.xform = QMatrix4x4()
    otra = copy_group(comp, V(5, 0, 0))
    scene.groups.extend([comp, otra])
    win.viewport.begin_group_edit(comp)
    assert outliner.editing_inside_component(scene)
    scene.selection.clear()
    scene.selection.update(list(scene.mesh.faces))
    win._on_make_group()
    assert comp.children == [] and len(scene.groups) == 2


def test_inside_a_group_inside_a_component_it_refuses(native_host):
    win = _window(native_host)
    scene = win.viewport.scene
    pata = _placed("Pata", 1.0)
    mesa = Group(_quad(size=3.0), name="Mesa")
    mesa.xform = QMatrix4x4()
    mesa.children = [pata]                      # a component of groups
    scene.groups.append(mesa)
    assert mesa.is_component()
    win.viewport.begin_group_edit(mesa)
    win.viewport.begin_group_edit(pata)
    assert outliner.editing_inside_component(scene)


def test_at_the_model_nothing_changes(native_host):
    win = _window(native_host)
    scene = win.viewport.scene
    a, b = _placed("A", 0.0), _placed("B", 3.0)
    scene.groups.extend([a, b])
    scene.selection.clear()
    scene.selection.update([a, b])
    win._on_make_group()
    assert len(scene.groups) == 1 and scene.groups[0].children == [a, b]


# --- the Browser's right-click ------------------------------------------------

def test_the_browser_groups_rows_inside_their_group(native_host):
    win = _window(native_host)
    scene = win.viewport.scene
    a, b = _placed("A", 2.0), _placed("B", 4.0)
    muro = Group(_quad(size=8.0), name="Muro")
    muro.xform = QMatrix4x4()
    muro.component = False
    muro.children = [a, b]
    scene.groups.append(muro)
    scene.select([a, b])
    action(native_host, "group", "i:" + a.uid)
    assert scene.groups == [muro]
    assert len(muro.children) == 1 and muro.children[0].children == [a, b]


def test_the_browser_makes_a_component_of_one_row(native_host):
    from ingetrazo_render import chrome_forms
    win = _window(native_host)
    scene = win.viewport.scene
    a = Group(_quad(), name="A")
    scene.groups.append(a)
    scene.select([a])
    action(native_host, "component", "i:" + a.uid)
    form = chrome_forms.current(win)
    form.receive("text", "Pieza")
    form.press("ok")
    assert scene.groups[0].is_component()
    assert scene.groups[0].name == "Pieza"


def test_the_browser_menu_offers_both(native_host):
    from core.i18n import tr
    win = _window(native_host)
    scene = win.viewport.scene
    a, b = _placed("A", 0.0), _placed("B", 3.0)
    scene.groups.extend([a, b])
    texts = group_rows(native_host, a)
    assert tr("Make Component") in texts
    assert tr("Make Group") not in texts
    scene.select([a, b])
    assert tr("Make Group") in group_rows(native_host, a)


# --- Explode inside an open group ---------------------------------------------

def test_explode_inside_an_open_group_keeps_what_comes_out_in_it(native_host):
    win = _window(native_host)
    scene = win.viewport.scene
    a, b = _placed("A", 2.0), _placed("B", 4.0)
    caja = Group(_quad(x=6.0), name="Caja")
    xf = QMatrix4x4()
    xf.translate(0.0, 0.0, 0.0)
    caja.xform = xf
    caja.component = False
    caja.children = [a, b]                      # a group of groups
    muro = Group(_quad(size=8.0, z=-1.0), name="Muro")
    muro.xform = QMatrix4x4()
    muro.component = False
    muro.children = [caja]
    scene.groups.append(muro)
    win.viewport.begin_group_edit(muro)
    before = {g.name: _points(g, scene.groups) for g in (a, b)}
    faces_before = len(scene.mesh.faces)
    scene.selection.clear()
    scene.selection.add(caja)
    win._on_explode_group()
    assert scene.groups == [muro]
    assert muro.children == [a, b]              # out of «Caja», still in
    assert len(scene.mesh.faces) == faces_before + 1   # its face, loose
    assert {g.name: _points(g, scene.groups) for g in (a, b)} == before
    win.viewport.history.undo()
    assert muro.children == [caja] and caja.children == [a, b]
    assert len(scene.mesh.faces) == faces_before


def test_explode_inside_a_component_still_refuses(native_host):
    win = _window(native_host)
    scene = win.viewport.scene
    pata = _placed("Pata", 1.0)
    mesa = Group(_quad(size=3.0), name="Mesa")
    mesa.xform = QMatrix4x4()
    mesa.children = [pata]
    scene.groups.append(mesa)
    win.viewport.begin_group_edit(mesa)
    scene.selection.clear()
    scene.selection.add(pata)
    win._on_explode_group()
    assert mesa.children == [pata]


# --- when the rows cannot be grouped, say why ---------------------------------

def test_locked_rows_grey_out_make_group_and_say_why(native_host):
    from core.i18n import tr
    win = _window(native_host)
    scene = win.viewport.scene
    a, b = _placed("A", 0.0), _placed("B", 3.0)
    a.locked = True
    scene.groups.extend([a, b])
    scene.select([a, b])
    why = tr("Locked or hidden rows cannot be grouped: unlock or show "
             "them first")
    assert group_rows(native_host, b)[tr("Make Group")] == why
    said = []
    win.viewport.flash_status = lambda text, *a_, **k: said.append(text)
    scene.select([a, b])
    action(native_host, "group", "i:" + a.uid)
    assert said == [why]
    assert len(scene.groups) == 2                 # nothing grouped


def test_rows_from_different_groups_say_so(native_host):
    from core.i18n import tr
    win = _window(native_host)
    scene = win.viewport.scene
    a, b = _placed("A", 0.0), _placed("B", 3.0)
    muro = Group(_quad(size=8.0), name="Muro")
    muro.xform = QMatrix4x4()
    muro.component = False
    muro.children = [b]
    scene.groups.extend([a, muro])
    said = []
    win.viewport.flash_status = lambda text, *a_, **k: said.append(text)
    scene.select([a, b])
    action(native_host, "group", "i:" + a.uid)
    assert said == [tr("Choose rows inside the same group to group "
                       "them")]


def test_rows_inside_a_component_grey_out_make_group_and_say_why(native_host):
    """The report: two rows inside an open component, Make Group did
    nothing — it is refused there, now greyed with the reason."""
    from core.i18n import tr
    win = _window(native_host)
    scene = win.viewport.scene
    a, b = _placed("A", 0.0), _placed("B", 3.0)
    comp = Group(_quad(size=8.0), name="Mueble")
    comp.xform = QMatrix4x4()
    comp.children = [a, b]                      # a component of groups
    scene.groups.append(comp)
    assert comp.is_component()
    win.viewport.begin_group_edit(comp)         # open, as in the report
    scene.select([a, b])
    why = tr("Components keep their parts: Make Unique first to change "
             "what is inside one")
    assert group_rows(native_host, b)[tr("Make Group")] == why
    assert group_rows(native_host, b)[tr("Make Component")] == why
    said = []
    win.viewport.flash_status = lambda text, *a_, **k: said.append(text)
    scene.select([a, b])
    action(native_host, "group", "i:" + a.uid)
    assert said == [why] and comp.children == [a, b]


# --- Explode from the Browser ---------------------------------------------------

def test_the_browser_explodes_each_kind_as_it_should_in_one_step(native_host):
    """A component holding groups leaves the groups; a group of groups
    leaves the inner groups; a plain group leaves its geometry loose — all
    the chosen rows at once, one undo step."""
    win = _window(native_host)
    scene = win.viewport.scene
    p1, p2 = _placed("P1", 0.0), _placed("P2", 1.5)
    comp = Group(_quad(size=3.0), name="Mueble")
    comp.xform = QMatrix4x4()
    comp.children = [p1, p2]                    # a component of groups
    q1 = _placed("Q1", 6.0)
    holder = Group(Mesh(), name="Conjunto")
    holder.xform = QMatrix4x4()
    holder.component = False
    holder.children = [q1]                      # a group of groups
    plain = Group(_quad(x=10.0), name="Losa")   # a plain group
    scene.groups.extend([comp, holder, plain])
    scene.version += 1
    loose_before = len(scene.mesh.faces)
    steps = len(win.viewport.history.undo_stack)
    scene.select([comp, holder, plain])
    action(native_host, "explode", "i:" + comp.uid)
    names = sorted(g.name for g in scene.groups)
    assert names == ["P1", "P2", "Q1"]
    assert len(scene.mesh.faces) == loose_before + 2   # Mueble's, Losa's
    assert len(win.viewport.history.undo_stack) == steps + 1
    win.viewport.history.undo()
    assert sorted(g.name for g in scene.groups) == \
        ["Conjunto", "Losa", "Mueble"]
    assert len(scene.mesh.faces) == loose_before


def test_explode_is_in_the_row_menu_and_greyed_inside_a_component(native_host):
    from core.i18n import tr
    win = _window(native_host)
    scene = win.viewport.scene
    a, b = _placed("A", 0.0), _placed("B", 3.0)
    comp = Group(_quad(size=8.0), name="Mueble")
    comp.xform = QMatrix4x4()
    comp.children = [a, b]
    plain = Group(_quad(x=12.0), name="Losa")
    scene.groups.extend([comp, plain])
    scene.version += 1
    assert group_rows(native_host, plain)[tr("Explode Group")] == ""
    assert group_rows(native_host, a)[tr("Explode Group")] == tr("Components keep their parts: Make Unique "
                              "first to change what is inside one")
