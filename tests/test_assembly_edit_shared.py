# SPDX-License-Identifier: GPL-3.0-or-later
"""Drawing inside an assembly: a component that holds other components.

Its own geometry is the definition its copies share, like a component with no
parts. What is drawn inside reaches every copy, each at its own place, and
undo takes it all back with the assembly's matrix where it was."""
from __future__ import annotations

import pytest
from PySide6.QtCore import QPointF, Qt
from ingetrazo_core import Matrix4 as QMatrix4x4, Vec3 as V
from PySide6.QtWidgets import QApplication

from core.edits import build_add_edge
from core.group import Group
from core.mesh import Mesh
from core.scene import Scene

_app = QApplication.instance() or QApplication([])
Z = 9.0


def _box():
    m = Mesh()
    c = [V(x, y, z) for z in (0, 1) for y in (0, 1) for x in (0, 1)]
    for q in ([0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4],
              [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]):
        m.add_face([c[i] for i in q])
    return m


def _at(x, turn=0.0):
    m = QMatrix4x4()
    m.translate(x, 0, 0)
    m.rotate(turn, 0, 0, 1)
    return m


def _assembly(own_faces, turn=0.0):
    """Two copies of one definition, each holding a copy of one part."""
    part, mesh = _box(), _box() if own_faces else Mesh()
    copies = []
    for name, x in (("A", 10.0), ("B", 20.0)):
        kid = Group(part, name="part")
        kid.xform = QMatrix4x4()
        g = Group(mesh, name=name)
        g.adopt([kid])
        g.xform = _at(x, turn)
        copies.append(g)
    return copies


def _drawn(scene):
    """World x of what was drawn, by copy (the boxes stand below Z)."""
    seen = {}
    for g, m in scene.placements():
        for v in g.mesh.vertices:
            w = m.map(v.position) if m is not None else v.position
            if abs(w.z() - Z) < 0.5:
                seen.setdefault(g.name, set()).add(round(w.x(), 3))
    return seen


def _window(*groups):
    from views.main_window import MainWindow
    win = MainWindow()
    win.viewport.scene.groups.extend(groups)
    win.viewport.scene.version += 1
    return win, win.viewport, win.viewport.scene


def _close(win):
    win._saved_version = win.viewport.scene.version
    win.close()


def _draw(vp, a, b):
    vp.history.execute(build_add_edge(vp.scene, a, b, detect_faces=True))


@pytest.mark.parametrize("own_faces", [True, False])
def test_a_line_drawn_in_an_assembly_reaches_every_copy(own_faces):
    a, b = _assembly(own_faces)
    win, vp, scene = _window(a, b)
    try:
        vp.begin_group_edit(a)
        _draw(vp, V(10, 0, Z), V(11, 0, Z))
        vp.end_group_edit()
        assert _drawn(scene) == {"A": {10.0, 11.0}, "B": {20.0, 21.0}}
        assert a.mesh is b.mesh
        assert a.xform == _at(10.0) and b.xform == _at(20.0)
    finally:
        _close(win)


def test_a_turned_assembly_keeps_its_parts_where_they_are():
    a, b = _assembly(True, turn=90.0)
    win, vp, scene = _window(a, b)
    corners = sorted((round(p.x(), 3), round(p.y(), 3)) for g, m in
                     scene.placements() if g.name == "part"
                     for p in (m.map(v.position) for v in g.mesh.vertices))
    try:
        vp.begin_group_edit(a)
        _draw(vp, V(10, 0, Z), V(10, 1, Z))
        vp.end_group_edit()
        after = sorted((round(p.x(), 3), round(p.y(), 3)) for g, m in
                       scene.placements() if g.name == "part"
                       for p in (m.map(v.position) for v in g.mesh.vertices))
        assert after == corners
        assert a.mesh is b.mesh and a.xform == _at(10.0, 90.0)
    finally:
        _close(win)


@pytest.mark.parametrize("own_faces", [True, False])
def test_undo_and_redo_are_one_step_and_keep_the_copies_shared(own_faces):
    a, b = _assembly(own_faces)
    win, vp, scene = _window(a, b)
    steps = len(vp.history.undo_stack)
    try:
        vp.begin_group_edit(a)
        _draw(vp, V(10, 0, Z), V(11, 0, Z))
        vp.end_group_edit()
        assert len(vp.history.undo_stack) == steps + 1
        vp.history.undo()
        assert _drawn(scene) == {}
        assert a.mesh is b.mesh and a.xform == _at(10.0)
        assert a.children[0].xform == QMatrix4x4()
        vp.history.redo()
        assert _drawn(scene) == {"A": {10.0, 11.0}, "B": {20.0, 21.0}}
        assert a.xform == _at(10.0)
        assert a.children[0].xform == QMatrix4x4()
    finally:
        _close(win)


def test_opening_an_assembly_and_leaving_changes_nothing():
    a, b = _assembly(False)
    win, vp, _scene = _window(a, b)
    mesh = a.mesh
    steps = len(vp.history.undo_stack)
    try:
        vp.begin_group_edit(a)
        vp.end_group_edit()
        assert a.mesh is mesh and a.xform == _at(10.0) and a.axes is None
        assert a.children[0].xform == QMatrix4x4()
        assert len(vp.history.undo_stack) == steps
    finally:
        _close(win)


def test_a_shared_part_opened_inside_an_assembly_still_shares():
    a, b = _assembly(True)
    win, vp, scene = _window(a, b)
    try:
        vp.begin_group_edit(a)
        vp.begin_group_edit(a.children[0])
        _draw(vp, V(10, 0, Z), V(11, 0, Z))
        vp.end_group_edit()
        assert _drawn(scene)["part"] == {10.0, 11.0, 20.0, 21.0}
        assert a.xform == _at(10.0) and a.children[0].xform == QMatrix4x4()
        assert a.children[0].mesh is b.children[0].mesh
    finally:
        _close(win)


def test_a_part_and_its_assembly_edited_together_undo_as_one_step():
    a, b = _assembly(True)
    win, vp, scene = _window(a, b)
    steps = len(vp.history.undo_stack)
    try:
        vp.begin_group_edit(a)
        _draw(vp, V(10, 0, Z), V(11, 0, Z))
        vp.begin_group_edit(a.children[0])
        _draw(vp, V(12, 0, Z), V(13, 0, Z))
        vp.end_group_edit()
        full = {"A": {10.0, 11.0}, "B": {20.0, 21.0},
                "part": {12.0, 13.0, 22.0, 23.0}}
        assert _drawn(scene) == full
        assert len(vp.history.undo_stack) == steps + 1
        vp.history.undo()
        assert _drawn(scene) == {}
        assert a.xform == _at(10.0) and a.children[0].xform == QMatrix4x4()
        vp.history.redo()
        assert _drawn(scene) == full
        assert a.xform == _at(10.0) and a.children[0].xform == QMatrix4x4()
    finally:
        _close(win)


def _parts_at(scene):
    """World origin of every part and sub-part, sorted."""
    o = V(0, 0, 0)
    return sorted((g.name, round(m.map(o).x(), 3), round(m.map(o).y(), 3),
                   round(m.map(o).z(), 3)) for g, m in scene.placements()
                  if g.name in ("part", "sub"))


def _move(win, vp, a):
    from core.history import MoveGroupCommand
    vp.history.execute(MoveGroupCommand(a.children[0], V(0, 5, 0)))


def _delete(win, vp, a):
    from core.history import DeleteGroupCommand
    vp.history.execute(DeleteGroupCommand(a.children[0]))


def _draw_and_delete(win, vp, a):
    _draw(vp, V(10, 0, Z), V(11, 0, Z))
    _delete(win, vp, a)


def _explode(win, vp, a):
    from core.history import ExplodeGroupCommand
    win._execute_make(ExplodeGroupCommand(a.children[0]))


@pytest.mark.parametrize("turn", [0.0, 90.0])
@pytest.mark.parametrize("edit", [_move, _delete, _draw_and_delete, _explode])
def test_parts_changed_inside_an_assembly_undo_and_redo_in_place(edit, turn):
    """Deleting, moving or exploding a part happens with the matrix down in
    the parts; leaving brings it back up. Undo used to restore a deleted
    part with the matrix still in it, so it landed one placement away."""
    a, b = _assembly(True, turn=turn)
    if edit is _explode:
        for g in (a, b):
            part = g.children[0]
            sub = Group(part.mesh, name="sub")
            sub.xform = QMatrix4x4()
            sub.xform.translate(0, 0, 3)
            part.mesh = Mesh()
            part.adopt([sub])
    win, vp, scene = _window(a, b)
    start, drawn0 = _parts_at(scene), _drawn(scene)
    parts0 = [QMatrix4x4(c.xform) for c in a.children]
    steps = len(vp.history.undo_stack)
    try:
        vp.begin_group_edit(a)
        edit(win, vp, a)
        vp.end_group_edit()
        left, drawn1 = _parts_at(scene), _drawn(scene)
        assert len(vp.history.undo_stack) == steps + 1
        assert a.xform == _at(10.0, turn) and a.mesh is b.mesh
        for _ in range(2):
            vp.history.undo()
            assert (_parts_at(scene), _drawn(scene)) == (start, drawn0)
            assert a.xform == _at(10.0, turn)
            assert [QMatrix4x4(c.xform) for c in a.children] == parts0
            assert a.mesh is b.mesh
            vp.history.redo()
            assert (_parts_at(scene), _drawn(scene)) == (left, drawn1)
            assert a.xform == _at(10.0, turn) and a.mesh is b.mesh
    finally:
        _close(win)


def test_a_copied_group_of_parts_is_made_unique_on_the_way_in():
    a, b = _assembly(True)
    a.component = b.component = False
    win, vp, scene = _window(a, b)
    try:
        vp.begin_group_edit(a)
        _draw(vp, V(10, 0, Z), V(11, 0, Z))
        vp.end_group_edit()
        assert _drawn(scene) == {"A": {10.0, 11.0}}
        assert a.mesh is not b.mesh
    finally:
        _close(win)


def test_an_assembly_placed_once_has_nothing_to_share_back():
    a, _b = _assembly(True)
    scene = Scene()
    scene.groups.append(a)
    scene.begin_group_edit(a)
    assert scene._edit_share is None
    scene.end_group_edit()


def test_a_plain_group_of_parts_keeps_baking_on_the_way_in():
    kid = Group(_box(), name="part")
    kid.xform = QMatrix4x4()
    g = Group(Mesh(), name="G")
    g.component = False
    g.adopt([kid])
    g.xform = _at(5.0, 90.0)
    scene = Scene()
    scene.groups.append(g)
    scene.begin_group_edit(g)
    assert scene._edit_share is None
    direct = scene.mesh.__dict__.get("_instance_definition") is g.mesh
    scene.end_group_edit()
    assert g.xform == (_at(5.0, 90.0) if direct else QMatrix4x4())


# ---- Nested transforms: rotated and mirrored parents, two levels deep ------

def _rot(x, y, turn):
    m = QMatrix4x4()
    m.translate(x, y, 0)
    m.rotate(turn, 0, 0, 1)
    return m


def _mirror(x, y, sx=-1.0, sy=1.0):
    m = QMatrix4x4()
    m.translate(x, y, 0)
    m.scale(sx, sy, 1)
    return m


def _deep():
    """Two copies of an outer assembly — one turned, one mirrored — each
    holding two copies of an inner assembly — one turned, one mirrored —
    each holding a part. Every level has geometry of its own."""
    part, inner, outer = _box(), _box(), _box()
    tops = []
    for name, xf in (("R1", _rot(10, 0, 30)), ("R2", _mirror(-10, 5))):
        kids = []
        for kname, kxf in (("A", _rot(2, 0, 90)), ("A", _mirror(0, 4, 1, -1))):
            p = Group(part, name="part")
            p.xform = QMatrix4x4()
            p.xform.translate(0, 0, 2)
            a = Group(inner, name=kname)
            a.adopt([p])
            a.xform = kxf
            kids.append(a)
        r = Group(outer, name=name)
        r.adopt(kids)
        r.xform = xf
        tops.append(r)
    return tops


def _matrices(groups):
    """Every placement's own matrix and axes, copied, depth first."""
    from core.purge import iter_groups
    return [(g.name, QMatrix4x4(g.xform) if g.xform is not None else None,
             None if g.axes is None else QMatrix4x4(g.axes))
            for g in iter_groups(groups)]


def _world_of(scene, mesh):
    """World matrix of every placement drawing ``mesh``."""
    return [m for g, m in scene.placements() if g.mesh is mesh]


def _drawn_xy(scene):
    """World (x, y) of what was drawn, over every placement."""
    seen = set()
    for g, m in scene.placements():
        for v in g.mesh.vertices:
            w = m.map(v.position) if m is not None else v.position
            if abs(w.z() - Z) < 0.5:
                seen.add((round(w.x(), 3), round(w.y(), 3)))
    return seen


def _expected(scene, drawn_in, points):
    """Where ``points`` (world, drawn inside placement ``drawn_in``) land
    in every copy of its definition."""
    w = next(m for g, m in scene.placements() if g is drawn_in)
    inv, ok = w.inverted()
    assert ok
    local = [inv.map(p) for p in points]
    return {(round(m.map(p).x(), 3), round(m.map(p).y(), 3))
            for m in _world_of(scene, drawn_in.mesh) for p in local}


def test_a_line_drawn_two_levels_deep_reaches_every_copy_in_its_own_frame():
    tops = _deep()
    win, vp, scene = _window(*tops)
    r1 = tops[0]
    target = r1.children[1]              # the mirrored copy in the turned one
    p, q = V(11, 3, Z), V(12, 4, Z)
    want = _expected(scene, target, [p, q])
    assert len(want) == 8                # four copies, two ends each
    start = _matrices(tops)
    steps = len(vp.history.undo_stack)
    try:
        vp.begin_group_edit(r1)
        vp.begin_group_edit(target)
        _draw(vp, p, q)
        vp.end_group_edit()
        assert scene.edit_group is None
        assert _drawn_xy(scene) == want
        assert _matrices(tops) == start
        assert len(vp.history.undo_stack) == steps + 1
        inner = target.mesh
        assert all(a.mesh is inner for r in tops for a in r.children)
        for _ in range(2):
            vp.history.undo()
            assert _drawn_xy(scene) == set()
            assert _matrices(tops) == start
            vp.history.redo()
            assert _drawn_xy(scene) == want
            assert _matrices(tops) == start
            assert all(a.mesh is inner for r in tops for a in r.children)
    finally:
        _close(win)


def test_esc_level_by_level_from_two_deep_folds_into_one_step():
    tops = _deep()
    win, vp, scene = _window(*tops)
    r2 = tops[1]                         # the mirrored outer copy
    target = r2.children[0]
    p, q = V(-12, 7, Z), V(-12, 8, Z)
    p2, q2 = V(-9, 3, Z), V(-8, 3, Z)
    want_inner = _expected(scene, target, [p, q])
    want_outer = _expected(scene, r2, [p2, q2])
    start = _matrices(tops)
    steps = len(vp.history.undo_stack)
    try:
        vp.begin_group_edit(r2)
        _draw(vp, p2, q2)                # on the outer assembly's own geometry
        vp.begin_group_edit(target)
        _draw(vp, p, q)
        vp.end_one_group_edit()          # Esc: up to the outer assembly
        assert scene.edit_group is r2
        vp.end_one_group_edit()          # Esc: out
        assert scene.edit_group is None
        assert _drawn_xy(scene) == want_inner | want_outer
        assert _matrices(tops) == start
        assert len(vp.history.undo_stack) == steps + 1
        vp.history.undo()
        assert _drawn_xy(scene) == set()
        assert _matrices(tops) == start
        vp.history.redo()
        assert _drawn_xy(scene) == want_inner | want_outer
        assert _matrices(tops) == start
    finally:
        _close(win)


@pytest.mark.parametrize("edit", ["delete", "move"])
def test_a_part_changed_two_levels_deep_undoes_in_place(edit):
    from core.history import DeleteGroupCommand, MoveGroupCommand
    tops = _deep()
    win, vp, scene = _window(*tops)
    r2 = tops[1]
    target = r2.children[1]              # mirrored in mirrored
    start, parts0 = _matrices(tops), _parts_at(scene)
    try:
        vp.begin_group_edit(r2)
        vp.begin_group_edit(target)
        if edit == "delete":
            vp.history.execute(DeleteGroupCommand(target.children[0]))
        else:
            vp.history.execute(MoveGroupCommand(target.children[0], V(0, 5, 0)))
        vp.end_group_edit()
        left = _parts_at(scene)
        assert left != parts0
        for _ in range(2):
            vp.history.undo()
            assert _parts_at(scene) == parts0
            assert _matrices(tops) == start
            vp.history.redo()
            assert _parts_at(scene) == left
    finally:
        _close(win)


# ---- The Metal view follows without a repack -------------------------------

class _Metal:
    """The Metal view of ``vp``'s document, packed once; every later
    ``set_scene`` (what the window runs on a scene change) must stay a row
    delta, draw what a fresh pack draws, and — ``local`` — not rescan
    every placement."""

    def __init__(self, vp, monkeypatch):
        from views.wgpu_view import WgpuView
        self.view = view = WgpuView()
        view.source_viewport = vp
        view.camera = vp.camera
        view.set_scene(vp.scene)
        self.scene = vp.scene
        monkeypatch.setattr(
            "views.wgpu_view.pack_scene",
            lambda *a, **k: (_ for _ in ()).throw(AssertionError("repack")))

    def settle(self, local=True, compare=True):
        from tests.test_wgpu_updates import assert_matches_fresh_pack
        self.view.set_scene(self.scene)
        updates = self.view._instance_updates
        if local:
            assert updates.last_local, "rescanned every placement"
        if compare:
            assert_matches_fresh_pack(updates, self.scene)

    def close(self):
        self.view.renderer = None
        self.view.close()


def test_metal_follows_a_shared_assembly_edit_and_its_undo_incrementally(
        monkeypatch):
    a, b = _assembly(True, turn=90.0)
    win, vp, scene = _window(a, b)
    metal = _Metal(vp, monkeypatch)
    try:
        vp.begin_group_edit(a)
        metal.settle(local=False, compare=False)   # entering moves nothing
        _draw(vp, V(10, 0, Z), V(10, 1, Z))
        metal.settle()
        vp.end_group_edit()
        metal.settle()
        assert a.mesh is b.mesh
        vp.history.undo()
        metal.settle()
        vp.history.redo()
        metal.settle()
    finally:
        metal.close()
        _close(win)


def test_metal_follows_a_deep_shared_edit_and_its_undo_incrementally(
        monkeypatch):
    tops = _deep()
    win, vp, scene = _window(*tops)
    metal = _Metal(vp, monkeypatch)
    try:
        vp.begin_group_edit(tops[0])
        metal.settle(local=False, compare=False)
        vp.begin_group_edit(tops[0].children[1])
        metal.settle(local=False, compare=False)
        _draw(vp, V(11, 3, Z), V(12, 4, Z))
        # The outer level still draws its definition where its world copy
        # stands: the same picture, compared once everything is left.
        metal.settle(compare=False)
        vp.end_group_edit()
        metal.settle()
        vp.history.undo()
        metal.settle()
        vp.history.redo()
        metal.settle()
    finally:
        metal.close()
        _close(win)


def test_metal_follows_a_part_deleted_in_an_assembly_without_a_repack(
        monkeypatch):
    """A part deleted inside: no row swap covers it, so the rows are
    rescanned — but nothing is packed again."""
    from core.history import DeleteGroupCommand
    a, b = _assembly(True, turn=90.0)
    win, vp, scene = _window(a, b)
    metal = _Metal(vp, monkeypatch)
    try:
        vp.begin_group_edit(a)
        metal.settle(local=False, compare=False)
        vp.history.execute(DeleteGroupCommand(a.children[0]))
        metal.settle(local=False)
        vp.end_group_edit()
        metal.settle(local=False)
        vp.history.undo()
        metal.settle(local=False)
        vp.history.redo()
        metal.settle(local=False)
    finally:
        metal.close()
        _close(win)


# ---- What stays as it was on trunk -----------------------------------------

def test_a_linked_assembly_stays_read_only(tmp_path, monkeypatch):
    """Parts of a linked file are edited in their own document: going into
    a copy of a link, or a part of it, offers the file and enters nothing."""
    from PySide6.QtWidgets import QMessageBox
    from core import links
    from formats import igz
    src = Scene()
    kid = Group(_box(), name="part")
    kid.xform = QMatrix4x4()
    asm = Group(_box(), name="asm")
    asm.adopt([kid])
    src.groups.append(asm)
    path = tmp_path / "Site.igz"
    igz.save_scene(src, path)
    win, vp, scene = _window()
    try:
        link = links.make_link(scene, path, host_path=tmp_path / "host.igz")
        copy = links.make_link(scene, path, host_path=tmp_path / "host.igz")
        copy.xform.translate(20, 0, 0)
        scene.groups.extend([link, copy])
        scene.version += 1
        asked = []
        monkeypatch.setattr(QMessageBox, "exec",
                            lambda box: asked.append(box.text()) or 0)
        before = _matrices(scene.groups)
        meshes = [g.mesh for g, _m in scene.placements()]
        steps = len(vp.history.undo_stack)
        from core.purge import iter_groups
        for g in list(iter_groups([link, copy])):
            vp.begin_group_edit(g)
            assert scene.edit_group is None
        assert asked
        assert _matrices(scene.groups) == before
        assert [g.mesh for g, _m in scene.placements()] == meshes
        assert len(vp.history.undo_stack) == steps
    finally:
        _close(win)


# ---- Faces, scaled parents and inference, two levels deep ------------------

def _scaled(m, sx, sy):
    out = QMatrix4x4(m)
    out.scale(sx, sy, 1)               # z kept: what is drawn stays at Z
    return out


def _deep_scaled():
    """:func:`_deep` with every placement also stretched, unevenly: the
    outer copies turned and stretched, or mirrored and stretched; the inner
    ones turned and squeezed, or mirrored and stretched across."""
    tops = _deep()
    for r, (sx, sy) in zip(tops, ((2.0, 1.0), (1.5, 0.5))):
        r.xform = _scaled(r.xform, sx, sy)
        for a, (ax, ay) in zip(r.children, ((0.5, 0.5), (1.0, 3.0))):
            a.xform = _scaled(a.xform, ax, ay)
    return tops


def _loop(vp, points):
    for p, q in zip(points, points[1:] + points[:1]):
        _draw(vp, p, q)


@pytest.mark.parametrize("build", [_deep, _deep_scaled])
@pytest.mark.parametrize("shape", ["line", "face"])
def test_a_line_or_a_face_drawn_two_deep_reaches_every_copy(build, shape):
    """A new loose line, or a closed loop that makes a face, drawn inside
    one copy of an assembly inside another — the parents turned, mirrored
    or stretched unevenly — lands in every copy in its own frame, and
    undoes and redoes as one step with every matrix as it was."""
    tops = build()
    win, vp, scene = _window(*tops)
    target = tops[1].children[1]            # mirrored in mirrored
    pts = ([V(-11, 3, Z), V(-12, 4, Z)] if shape == "line" else
           [V(-11, 3, Z), V(-10, 3, Z), V(-10, 4, Z), V(-11, 4, Z)])
    want = _expected(scene, target, pts)
    assert len(want) == 4 * len(pts)
    inner = target.mesh
    faces0 = len(inner.faces)
    start = _matrices(tops)
    steps = len(vp.history.undo_stack)
    try:
        vp.begin_group_edit(tops[1])
        vp.begin_group_edit(target)
        if shape == "line":
            _draw(vp, *pts)
        else:
            _loop(vp, pts)
        vp.end_group_edit()
        added = 1 if shape == "face" else 0
        assert _drawn_xy(scene) == want
        assert len(inner.faces) == faces0 + added
        assert all(a.mesh is inner for r in tops for a in r.children)
        assert _matrices(tops) == start
        assert len(vp.history.undo_stack) == steps + 1
        for _ in range(2):
            vp.history.undo()
            assert _drawn_xy(scene) == set()
            assert len(inner.faces) == faces0
            assert _matrices(tops) == start
            vp.history.redo()
            assert _drawn_xy(scene) == want
            assert len(inner.faces) == faces0 + added
            assert _matrices(tops) == start
    finally:
        _close(win)


@pytest.mark.parametrize("build", [_deep, _deep_scaled])
def test_inference_inside_a_nested_copy_finds_its_parts_where_they_stand(build):
    """Inside a copy, two levels down, the cursor over a corner of the part
    it holds snaps to that corner where the part is drawn in the world —
    not where the definition or another copy would put it."""
    from core.purge import iter_groups
    tops = build()
    win, vp, scene = _window(*tops)
    win.show()
    win.resize(1200, 800)
    target = tops[0].children[0]            # turned in turned (stretched)
    part = target.children[0]
    try:
        vp.camera.set_view("iso")
        vp.begin_group_edit(tops[0])
        vp.begin_group_edit(target)
        _app.processEvents()
        win._activate_tool("line")
        _app.processEvents()
        world = next(m for g, m in scene.placements() if g is part)
        corners = [world.map(v.position) for v in part.mesh.vertices]
        # The top corner nearest the iso eye (+X, −Y, +Z): in sight.
        corner = max(corners, key=lambda p: p.x() - p.y() + 2 * p.z())
        px, py = vp._world_to_pixel(corner)
        vp._last_mouse_pos = QPointF(px, py)
        vp._refresh_snap()
        snap = vp.inference.snap(px, py, Qt.NoModifier)
        assert snap is not None and snap.kind == "endpoint", snap
        assert (snap.point - corner).length() < 1e-6
        assert all(g.xform is not None for g in iter_groups(tops))
    finally:
        vp.end_group_edit()
        _close(win)
