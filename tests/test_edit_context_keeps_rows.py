# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Opening and closing a group keep the rows of what did not change (#158).

A group opened or closed changes the context and the loose mesh, and the
per-placement tables — the instanced pool, the silhouette table, the lazy
pick table — made all their rows again: ~0.9 s each way on the plant of
21 406 placements. They read only what a placement IS (mesh, matrix, box),
so the rows of every object that does not use the mesh being edited are
kept. Each kept table is checked against the one made afresh, at the root,
inside a group, inside a component (every copy of it is the loose mesh
then) and back out."""
from __future__ import annotations

import numpy as np
import pytest
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from ingetrazo_core import Matrix4 as QMatrix4x4, Vec3 as V


def _box(x0=0.0, y0=0.0, s=1.0, curved=False):
    from core.mesh import Mesh
    m = Mesh()
    p = [V(x0, y0, 0), V(x0 + s, y0, 0), V(x0 + s, y0 + s, 0),
         V(x0, y0 + s, 0), V(x0, y0, s), V(x0 + s, y0, s),
         V(x0 + s, y0 + s, s), V(x0, y0 + s, s)]
    for q in ((0, 3, 2, 1), (4, 5, 6, 7), (0, 1, 5, 4), (1, 2, 6, 5),
              (2, 3, 7, 6), (3, 0, 4, 7)):
        m.add_face([p[i] for i in q])
    if curved:                       # soft edges: the silhouette table
        for e in list(m.edges)[:6]:
            e.soft = True
    return m


def _placed(mesh, name, x, y):
    from core.group import Group
    g = Group(mesh, name=name)
    xf = QMatrix4x4()
    xf.translate(x, y, 0.0)
    g.xform = xf
    return g


@pytest.fixture
def vp(native_host):
    from core.group import Group
    native_host.resize(1000, 700, 1.)
    v = native_host.viewport
    sc = v.scene
    sc.guides.clear()
    post = _box(curved=True)
    lamp = _box(s=0.5, curved=True)
    for i in range(8):
        for j in range(3):
            sc.groups.append(_placed(post, f"P{i}{j}", 2.0 * i, 2.0 * j))
    for k in range(3):
        sc.groups.append(_placed(lamp, f"L{k}", 2.0 * k, 9.0))
    room = Group(_box(-6.0, -6.0, 2.0), name="room")        # a plain group
    room.children = [_placed(post, "inner", -5.0, -5.0)]
    sc.groups.append(room)
    sc.version += 1
    v.camera.set_view("iso")
    v.camera.fit_to(V(-7, -7, 0), V(17, 11, 2))
    v._PICK_LAZY_MIN_FACES = 1
    return v


def _tables(vp):
    vp._tick = getattr(vp, "_tick", 0) + 1
    return vp._pick_lazy_table()


def _fresh(vp):
    vp._pick_lazy_memo = None
    return _tables(vp)


def _same(kl, fl):
    assert [id(g) for g in kl[0]] == [id(g) for g in fl[0]]
    for i in (1, 2, 3):
        assert np.array_equal(kl[i], fl[i]), i


def _count_frames(vp, monkeypatch):
    calls = []
    real = type(vp)._placement_frame

    def spy(self, g):
        calls.append(id(g))
        return real(self, g)
    monkeypatch.setattr(type(vp), "_placement_frame", spy)
    return calls


def test_opening_and_closing_a_group_keeps_the_rest_and_matches_afresh(
        vp, monkeypatch):
    sc = vp.scene
    _tables(vp)
    room = next(g for g in sc.groups if g.name == "room")
    calls = _count_frames(vp, monkeypatch)
    sc.begin_group_edit(room)
    kept = _tables(vp)
    # Only the opened object's placements were framed again.
    tops = {vp._placement_top(g) for g in vp._placements()
            if id(g) in set(calls)}
    assert tops <= {id(room)}, len(tops)
    _same(kept, _fresh(vp))
    calls.clear()
    sc.end_group_edit()
    kept = _tables(vp)
    tops = {vp._placement_top(g) for g in vp._placements()
            if id(g) in set(calls)}
    assert tops <= {id(room)}, len(tops)
    _same(kept, _fresh(vp))


def test_inside_a_component_every_copy_is_made_again(vp, monkeypatch):
    # Every copy of a component being edited is the loose mesh: none of
    # their rows may be kept, the other objects' are.
    sc = vp.scene
    _tables(vp)
    lamp = next(g for g in sc.groups if g.name == "L1")
    calls = _count_frames(vp, monkeypatch)
    sc.begin_group_edit(lamp)
    kept = _tables(vp)
    framed = set(calls)
    names = {g.name for g in vp._placements() if id(g) in framed}
    assert not any(n.startswith("P") for n in names), names
    _same(kept, _fresh(vp))
    calls.clear()
    sc.end_group_edit()
    kept = _tables(vp)
    framed = set(calls)
    names = {g.name for g in vp._placements() if id(g) in framed}
    assert not any(n.startswith("P") for n in names), names
    _same(kept, _fresh(vp))


def test_hide_similar_components_makes_everything_again(vp):
    sc = vp.scene
    sc.hide_similar_components = True
    _tables(vp)
    sc.begin_group_edit(next(g for g in sc.groups if g.name == "L1"))
    kept = _tables(vp)
    _same(kept, _fresh(vp))
    sc.end_group_edit()
    kept = _tables(vp)
    _same(kept, _fresh(vp))


def test_the_library_keeps_its_material_scan_when_entering_and_leaving_a_group(native_host, monkeypatch):
    from core import geometry_arrays
    from core.group import Group
    from core.history import SetFaceColorCommand
    from views import wgpu_library as lib

    win = native_host.window
    vp = win.viewport
    sc = vp.scene
    post = _box(curved=True)
    for f in post.faces:
        f.attrs["color"] = [0.2, 0.4, 0.8]
    lamp = _box(s=0.5)
    for k in range(3):
        sc.groups.append(_placed(post, f"P{k}", 2.0 * k, 0.0))
        sc.groups.append(_placed(lamp, f"L{k}", 2.0 * k, 5.0))
    room = Group(_box(-6.0, -6.0, 2.0), name="room")
    room.children = [_placed(post, "inner", -5.0, -5.0)]
    xf = QMatrix4x4()
    xf.translate(0.0, 0.0, 1.0)
    room.xform = xf
    sc.groups.append(room)
    sc.version += 1

    ctx = lib.Ctx(win, win.view)
    provider = lib.MaterialsProvider()
    def colours():
        return {row["name"] for row in provider._model_items(ctx)}
    read = []
    original = geometry_arrays.face_attributes
    def spy(mesh):
        read.append(id(mesh))
        return original(mesh)
    monkeypatch.setattr(geometry_arrays, "face_attributes", spy)
    start = colours()
    assert "#3366CC" in start
    read.clear()
    vp.begin_group_edit(room)
    assert colours() == start
    assert id(post) not in read                 # the copies: kept
    vp.end_group_edit()
    assert colours() == start
    assert id(post) not in read
    # Inside a component, a paint: a command, so all is read again;
    # leaving shares it back to every copy, and it shows.
    vp.begin_group_edit(next(g for g in sc.groups if g.name == "L1"))
    vp.history.execute(SetFaceColorCommand(list(sc.mesh.faces),
                                           (0.9, 0.1, 0.1)))
    assert "#E61A1A" in colours()
    vp.end_group_edit()
    got = colours()
    assert "#E61A1A" in got and "#3366CC" in got
    # Undone (outside): the paint goes, and the panel says so.
    while vp.history.undo_stack:
        vp.history.undo()
    assert colours() == start
    # A paint the panel has not read yet, then a group opened: the
    # paint still shows — the open does not vouch for what came
    # before it.
    vp.history.execute(SetFaceColorCommand(list(post.faces)[2:3],
                                           (0.1, 0.8, 0.1)))
    vp.begin_group_edit(room)
    assert "#1ACC1A" in colours()
    vp.end_group_edit()
    assert "#1ACC1A" in colours()


def test_the_static_list_across_a_group_opened_and_closed_reads_as_made_afresh(
        vp, monkeypatch):
    sc = vp.scene
    glass = _box(-3.0, 4.0)
    for f in glass.faces:
        f.attrs["opacity"] = 0.4          # translucent: drawn static
    sc.groups.append(_placed(glass, "G0", 0.0, 0.0))
    sc.groups.append(_placed(glass, "G1", 1.5, 0.0))
    sc.version += 1

    def both():
        vp._tick = getattr(vp, "_tick", 0) + 1
        groups = vp._placements()
        kept = vp._static_draw_groups(groups)
        vp._static_groups_memo = None
        fresh = vp._static_draw_groups(groups)
        return [id(g) for g in kept], [id(g) for g in fresh]
    k, f = both()
    assert k == f and len(f) >= 3
    # Opening a group asks only the opened object's placements again.
    asked = []
    real = type(vp)._instanced_eligible
    monkeypatch.setattr(type(vp), "_instanced_eligible",
                        lambda self, g: asked.append(g.name) or real(self, g))
    sc.begin_group_edit(next(g for g in sc.groups if g.name == "room"))
    vp._tick += 1
    vp._static_draw_groups(vp._placements())
    assert set(asked) <= {"room", "inner"}, asked
    sc.end_group_edit()
    monkeypatch.setattr(type(vp), "_instanced_eligible", real)
    for name in ("room", "G1", "L0"):
        sc.begin_group_edit(next(g for g in sc.groups if g.name == name))
        k, f = both()
        assert k == f, name
        sc.end_group_edit()
        k, f = both()
        assert k == f, name
    # A component made translucent inside: on the way out its copies leave
    # the instanced passes for the static ones.
    from core.history import SetFaceOpacityCommand
    lamps = {id(g) for g in sc.groups if g.name.startswith("L")}
    assert not lamps & set(k)
    vp.begin_group_edit(next(g for g in sc.groups if g.name == "L1"))
    both()
    vp.history.execute(SetFaceOpacityCommand(list(sc.mesh.faces), 0.5))
    vp.end_group_edit()
    k, f = both()
    assert k == f
    assert lamps <= set(k)


def test_the_browser_reads_only_the_rows_a_group_opened_or_closed_changes(native_host, vp):
    from core.history import SetFaceColorCommand
    from tests.native_tree import FilingRows
    room = next(g for g in vp.scene.groups if g.name == "room")
    inner = room.children[0]
    lamp = next(g for g in vp.scene.groups if g.name == "L1")
    browser = FilingRows(native_host, ["i:" + room.uid])
    browser.read()
    model = browser.model
    for step in (lambda: vp.begin_group_edit(room), lambda: vp.begin_group_edit(inner),
                 vp.end_one_group_edit, vp.end_group_edit,
                 lambda: vp.begin_group_edit(lamp), vp.end_group_edit):
        step()
        got = browser.read()
        assert got == browser.read(fresh=True)
        assert browser.model is model
        if vp.scene.edit_group is not None:
            assert any(t[0] == "context:fold" for t, _ in got)
    vp.begin_group_edit(lamp)
    vp.history.execute(SetFaceColorCommand(list(vp.scene.mesh.faces), (.9, .1, .1)))
    assert browser.read() == browser.read(fresh=True)
    vp.end_group_edit()
    assert browser.read() == browser.read(fresh=True)


def test_plain_group_retention_counts_open_edits_as_one_step(vp):
    from core.group import Group
    from core.history import NativeCommand

    group = Group(name="before")
    vp.scene.groups.append(group)
    vp.history.max_steps = 1
    vp.begin_group_edit(group)
    for name in ("middle", "after"):
        vp.history.execute(NativeCommand.rename(group, name))
    vp._leave_group_edit(todos=True)
    assert len(vp.history.undo_stack) == 1
    assert vp.history.undo() and group.name == "before"
    assert vp.history.redo() and group.name == "after"
