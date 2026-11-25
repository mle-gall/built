# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Picking an object up and putting it down touch what moved, not the
model (#158, «sin lag»).

On the plant of #158 a Move's pick-up and drop were each most of a second
of work that does not depend on the object: the static buffers re-sent
everything after the mover (0.4 GB of faces), the lazy pick table framed
all 20 500 placements again, the arc midpoints of every mesh were worked
out anew, and the pick index let go of static placements' bakes that the
draw then made over. Each answer here is checked against the one made
afresh, and the frame against what it has to show."""
from __future__ import annotations

from tests.shell_isolation import restore_shell_patches  # noqa: F401


import numpy as np  # noqa: E402
import pytest  # noqa: E402
from ingetrazo_core import Matrix4 as QMatrix4x4, Vec3 as V


AMARILLO = (1.0, 0.85, 0.0)


def _square(x0, y0, s=1.0, color=None):
    from core.mesh import Mesh
    m = Mesh()
    f = m.add_face([V(x0, y0, 0), V(x0 + s, y0, 0), V(x0 + s, y0 + s, 0),
                    V(x0, y0 + s, 0)])
    if f.normal().z() < 0:
        f.flip()
    if color is not None:
        f.attrs["color"] = list(color)
    return m


def _cube():
    from core.mesh import Mesh
    m = Mesh()
    p = [V(0, 0, 0), V(1, 0, 0), V(1, 1, 0), V(0, 1, 0),
         V(0, 0, 1), V(1, 0, 1), V(1, 1, 1), V(0, 1, 1)]
    for idx in ((0, 3, 2, 1), (4, 5, 6, 7), (0, 1, 5, 4), (1, 2, 6, 5),
                (2, 3, 7, 6), (3, 0, 4, 7)):
        m.add_face([p[i] for i in idx])
    return m


@pytest.fixture
def placed(native_host):
    from core.group import Group
    win = native_host.window
    v = win.viewport
    native_host.resize(1000, 700, 1.)
    v.scene.guides.clear()
    proto, glass = _cube(), _cube()
    for f in glass.faces:
        f.attrs["opacity"] = 0.5            # translucent: drawn static
    for i in range(6):
        for j in range(4):
            g = Group(glass if j == 0 else proto, name=f"C{i}{j}")
            m = QMatrix4x4()
            m.translate(3.0 * i, 3.0 * j, 0.0)
            g.xform = m
            v.scene.groups.append(g)
    v.scene.version += 1
    v.camera.set_view("top")
    v.camera.perspective = False
    v.camera.fit_to(V(-2, -2, 0), V(18, 12, 1))
    v._PICK_LAZY_MIN_FACES = 1
    yield v


def test_the_lazy_pick_table_is_kept_on_pick_up_and_its_rows_follow_a_drop(placed):
    from core.history import MoveGroupCommand
    vp = placed
    g = next(g for g in vp.scene.groups if g.name == "C21")
    table = vp._pick_lazy_table()
    table_epoch = vp._placements_epoch()
    # What begin/end_groups_preview do to the epoch (they need GL).
    vp._preview_groups = {id(g)}
    vp._preview_epoch = getattr(vp, "_preview_epoch", 0) + 1
    vp._tick = getattr(vp, "_tick", 0) + 1         # a new frame
    assert vp._placements_epoch() != table_epoch
    assert vp._pick_lazy_table() is table          # nothing moved yet
    vp._preview_groups = set()
    vp._preview_epoch += 1
    vp._tick += 1
    vp.history.execute(MoveGroupCommand(g, V(0.5, 7.0, 0.25)))
    kept = vp._pick_lazy_table()
    vp._pick_lazy_memo = None
    fresh = vp._pick_lazy_table()
    assert [id(x) for x in kept[0]] == [id(x) for x in fresh[0]]
    assert np.array_equal(kept[1], fresh[1])
    assert np.array_equal(kept[2], fresh[2])
    assert not np.array_equal(kept[1], table[1])   # the moved row did move
    assert table[1] is not kept[1]                 # and the old one is intact


def test_an_eviction_keeps_the_bakes_the_static_buffers_draw(placed,
                                                            monkeypatch):
    # The baking path: with the experimental engine a pick bakes nothing.
    import views.document_interaction as vpm
    monkeypatch.setattr(vpm, "_LAZY_NATIVE", False)
    vp = placed
    vp._PICK_LIVE_MAX_FACES = 12                   # two cubes
    by = {g.name: g for g in vp.scene.groups}
    assert not vp._instanced_eligible(by["C00"])               # glass
    assert vp._instanced_eligible(by["C01"])
    vp._input_serial = 0
    for name in ("C00", "C01", "C10", "C11", "C20", "C21", "C30"):
        g = next(g for g in vp.scene.groups if g.name == name)
        vp._input_serial += 1
        x, y = vp._world_to_pixel(g.xform.map(V(0.5, 0.5, 1.0)))
        assert vp.pick_group(x, y) is g             # baked by the index
    inst = getattr(vp, "_inst_chunks", {})
    live = getattr(vp, "_pick_live", None) or {}
    gone = [g for g in vp.scene.groups
            if g.name in ("C00", "C01", "C10", "C11", "C20", "C21")
            and id(g) not in live]
    assert gone, "the cap let some go"
    for g in gone:
        if vp._instanced_eligible(g):
            assert id(g) not in inst
        else:
            assert id(g) in inst                   # the draw still needs it


def test_arc_midpoints_are_kept_per_mesh_however_many_and_follow_the_content(native_host,
        monkeypatch):
    from core.group import Group
    from tests.controller_surface import ControllerSurface as Viewport
    win = native_host.window
    vp = win.viewport
    calls = []

    def mids(mesh):
        calls.append(id(mesh))
        return [V(0.0, 0.0, 0.0)]
    monkeypatch.setattr(type(vp), "_mesh_arc_midpoints",
                        staticmethod(mids))
    gs = []
    for k in range(2100):                      # more than 2 048 meshes
        g = Group(_square(0.0, 0.0, 0.1), name=f"S{k}")
        m = QMatrix4x4()
        m.translate(float(k), 0.0, 0.0)
        g.xform = m
        gs.append(g)
    vp.scene.groups += gs
    vp.scene.version += 1
    vp._arc_midpoints(0.0, 0.0)
    first = len(calls)
    assert first >= 2100
    kept = vp._arc_mid_cache[1]
    vp.scene.select([gs[0]])                   # a view-only change
    vp._arc_midpoints(0.0, 0.0)
    assert len(calls) == first
    assert vp._arc_mid_cache[1] is kept        # not even remapped
    m = QMatrix4x4()
    m.translate(0.0, 9.0, 0.0)
    gs[5].xform = m
    vp.scene.version += 1                      # content: remapped...
    vp._arc_midpoints(0.0, 0.0)
    assert len(calls) == first                 # ...not worked out again
    pts = {(round(p.x(), 6), round(p.y(), 6))
           for p, _ctx in vp._arc_mid_cache[1]}   # (point, snap context)
    assert (0.0, 9.0) in pts and (5.0, 0.0) not in pts


def test_a_moved_container_moves_the_rows_of_the_placements_inside_it(placed):
    from core.group import Group
    from core.history import MoveGroupCommand
    vp = placed
    inner = Group(_cube(), name="dentro")
    inner.xform = QMatrix4x4()
    box = Group(None, name="caja")
    m = QMatrix4x4()
    m.translate(0.0, 15.0, 0.0)
    box.xform = m
    box.children = [inner]
    vp.scene.groups.append(box)
    vp.scene.version += 1
    vp._pick_lazy_memo = None
    vp._pick_lazy_table()
    vp.history.execute(MoveGroupCommand(box, V(4.0, 0.0, 0.0)))
    kept = vp._pick_lazy_table()
    vp._pick_lazy_memo = None
    fresh = vp._pick_lazy_table()
    assert np.array_equal(kept[1], fresh[1])
    assert np.array_equal(kept[2], fresh[2])
