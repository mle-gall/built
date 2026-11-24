# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""A Delete, a Paste and their undo touch the objects that came or went,
not the model (#158, «sin lag»).

On the plant of #158 deleting one object took 3.4 s and pasting one 3.5 s:
the per-placement passes made all 21 406 placements again, the static
buffers re-sent everything after the object, the Browser made its 21 405
rows again and the materials panel read 1.5 million face paints. Each
pass now keeps what belongs to the objects that did not change; every
answer here is compared with one made afresh."""
from __future__ import annotations

import math
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from ingetrazo_core import Matrix4 as QMatrix4x4, Vec3 as V


def _prisma(n=12, r=1.0, h=2.0):
    """A smooth n-sided prism: soft vertical edges, a curved side."""
    from core.mesh import Mesh
    m = Mesh()
    ring = [(r * math.cos(2 * math.pi * i / n), r * math.sin(2 * math.pi * i / n))
            for i in range(n)]
    for i in range(n):
        (x0, y0), (x1, y1) = ring[i], ring[(i + 1) % n]
        m.add_face([V(x0, y0, 0), V(x1, y1, 0), V(x1, y1, h), V(x0, y0, h)])
    for e in m.edges:
        if abs(e.a.z() - e.b.z()) > 1e-9:
            e.soft = True
    return m


def _square(x0, y0, s=1.0, color=None, opacity=None):
    from core.mesh import Mesh
    m = Mesh()
    f = m.add_face([V(x0, y0, 0), V(x0 + s, y0, 0), V(x0 + s, y0 + s, 0),
                    V(x0, y0 + s, 0)])
    if f.normal().z() < 0:
        f.flip()
    if color is not None:
        f.attrs["color"] = list(color)
    if opacity is not None:
        f.attrs["opacity"] = opacity
    return m


def _placed(proto, name, x, y=0.0, turn=0.0):
    from core.group import Group
    g = Group(proto, name=name)
    m = QMatrix4x4()
    m.translate(x, y, 0.0)
    m.rotate(turn, 0.0, 0.0, 1.0)
    g.xform = m
    return g


def _scene(win):
    """Instanced prisms of two prototypes, a container holding two more
    (nested placements), and plain groups — one of them glass."""
    from core.group import Group
    vp = win.viewport
    sc = vp.scene
    sc.guides.clear()
    protos = [_prisma(12), _prisma(7, r=0.5, h=3.0)]
    gs = [_placed(protos[k % 2], f"P{k}", 3.0 * k, (k % 3) * 1.5, 17.0 * k)
          for k in range(8)]
    box = Group(None, name="caja")
    m = QMatrix4x4()
    m.translate(0.0, 9.0, 0.0)
    box.xform = m
    box.children = [_placed(protos[0], "dentro0", 0.0),
                    _placed(protos[1], "dentro1", 3.0)]
    plain = [Group(_square(-4.0, 2.0 * k, color=(0.2, 0.3 + 0.1 * k, 0.8)),
                   name=f"S{k}") for k in range(4)]
    glass = Group(_square(-7.0, 0.0, color=(0.6, 0.8, 0.9), opacity=0.4),
                  name="vidrio")
    sc.groups += gs[:4] + [plain[0], box, plain[1], glass] + gs[4:] + plain[2:]
    sc.version += 1
    return vp, gs, box, plain, glass


def _fresh(vp, fn, *names):
    keep = {n: getattr(vp, n, None) for n in names}
    for n in names:
        setattr(vp, n, None)
    try:
        return fn()
    finally:
        for n, v in keep.items():
            setattr(vp, n, v)


def _lazy(vp):
    t = vp._pick_lazy_table()
    return ([id(g) for g in t[0]], t[1].tolist(), t[2].tolist(),
            t[3].tolist(), sorted(t[4]))


def _steps(vp, gs, box, plain):
    """Delete a placement, a container and a plain group; paste a copy of
    a placement and of the container; undo it all, one by one — yielding
    after each."""
    from core.group import copy_group
    from core.history import DeleteGroupCommand, InsertGroupCommand
    h = vp.history
    for cmd in (DeleteGroupCommand(gs[2]), DeleteGroupCommand(box),
                DeleteGroupCommand(plain[1]),
                InsertGroupCommand(copy_group(gs[5], V(0.0, -6.0, 0.0))),
                InsertGroupCommand(copy_group(gs[0], V(1.0, -9.0, 0.0)))):
        h.execute(cmd)
        yield type(cmd).__name__
    for _ in range(5):
        h.undo()
        yield "undo"


def test_the_lists_and_the_lazy_table_read_as_made_afresh(native_host):
    vp, gs, box, plain, glass = _scene(native_host)
    vp._PICK_LAZY_MIN_FACES = 1
    _lazy(vp)
    placements = vp._placements()
    vp._static_draw_groups(placements)
    for what in _steps(vp, gs, box, plain):
        got = _lazy(vp)
        assert got == _fresh(vp, lambda: _lazy(vp), "_pick_lazy_memo"), what
        placements = vp._placements()
        stat = [id(g) for g in vp._static_draw_groups(placements)]
        fresh = _fresh(vp, lambda: [id(g) for g in vp._static_draw_groups(
            placements)], "_static_groups_memo")
        assert stat == fresh, what


def test_an_object_kept_is_one_whose_whole_subtree_reads_the_same(native_host):
    vp, gs, box, plain, glass = _scene(native_host)
    old = vp._placements_epoch()
    m = QMatrix4x4(box.children[1].xform)
    m.translate(0.0, 0.5, 0.0)
    box.children[1].xform = m              # inside the container
    vp.scene.version += 1
    kept = vp._epoch_kept_tops(old)
    assert id(box) not in kept
    assert {id(g) for g in vp.scene.groups} - kept == {id(box)}
    old = vp._placements_epoch()
    vp.scene.layers[0].visible = not vp.scene.layers[0].visible
    vp.scene.version += 1
    assert vp._epoch_kept_tops(old) is None   # the rest changed too


def test_the_browser_takes_rows_out_and_puts_them_in_as_a_rebuild_would(native_host):
    from core.group import copy_group
    from core.history import DeleteGroupCommand, InsertGroupCommand
    from tests.native_tree import FilingRows
    vp, groups, box, *_ = _scene(native_host)
    browser = FilingRows(native_host, ["i:" + box.uid])
    browser.read()
    model = browser.model
    h = vp.history
    commands = [DeleteGroupCommand(groups[2]), DeleteGroupCommand(box),
                InsertGroupCommand(copy_group(groups[5], V(0, -6, 0))),
                InsertGroupCommand(copy_group(box, V(4, 0, 0)))]
    for cmd in commands:
        h.execute(cmd)
        assert browser.read() == browser.read(fresh=True)
        assert browser.model is model
    for _ in commands:
        assert h.undo()
        assert browser.read() == browser.read(fresh=True)
        assert browser.model is model
    from core.outliner import instance_counts
    n = instance_counts(vp.scene.groups)[id(groups[1].mesh)]
    def badges():
        return {t[0]: t[3] for t, _ in browser.read()}
    keys = ["i:" + groups[1].uid, "c:" + box.children[1].uid]
    before = badges()
    assert all(before[k] == f"×{n}" for k in keys)
    h.execute(InsertGroupCommand(copy_group(groups[1], V(0, 5, 0))))
    browser.view._ui_exp.add(f"d::{id(groups[1].mesh)}")
    after = badges()
    assert all(after[k] == f"×{n + 1}" for k in keys)
    assert browser.model is model


def test_a_rename_between_two_deletes_makes_the_rows_read_again(native_host):
    from core.history import DeleteGroupCommand
    from tests.test_tree_links import _View
    from views import wgpu_ui
    vp, groups, *_ = _scene(native_host)
    view = _View()
    view._ui_exp = {"n:"}
    wgpu_ui._tree(view, native_host.window)
    vp.history.execute(DeleteGroupCommand(groups[2]))
    groups[4].name = "External rename"
    vp.scene.version += 1
    vp.history.execute(DeleteGroupCommand(groups[3]))
    wgpu_ui._tree(view, native_host.window)
    names = {t[0]: t[1] for t, _ in view.renderer.calls["ui_set_tree"][0]}
    assert names["i:" + groups[4].uid] == "External rename"
    assert all("i:" + g.uid not in names for g in groups[2:4])


def test_the_library_reads_only_new_or_repainted_meshes(native_host, monkeypatch):
    from core import geometry_arrays
    from core.group import copy_group
    from core.history import DeleteGroupCommand, InsertGroupCommand, SetFaceColorCommand
    from views import wgpu_library as lib

    win = native_host.window
    vp, _, _, plain, _ = _scene(win)
    ctx = lib.Ctx(win, win.view)
    provider = lib.MaterialsProvider()
    read = []
    scan = geometry_arrays.face_attributes

    def spy(mesh):
        read.append(id(mesh))
        return scan(mesh)

    monkeypatch.setattr(geometry_arrays, "face_attributes", spy)
    def colours():
        return {row["name"] for row in provider._model_items(ctx)}

    start = colours()
    assert "#3366CC" in start
    read.clear()
    vp.history.execute(DeleteGroupCommand(plain[1]))
    assert "#3366CC" not in colours() and not read
    vp.history.undo()
    assert colours() == start and read == [id(plain[1].mesh)]
    read.clear()
    pasted = copy_group(plain[2], V(0, 30, 0))
    vp.history.execute(InsertGroupCommand(pasted))
    assert colours() == start and read == [id(pasted.mesh)]
    read.clear()
    vp.history.execute(SetFaceColorCommand(list(plain[0].mesh.faces), (0.9, 0.1, 0.1)))
    assert "#E61A1A" in colours()
    if getattr(plain[0].mesh, "_store", None) is not None:
        assert read == [id(plain[0].mesh)]
    else:
        assert len(read) > 1


def test_unchanged_swatches_and_component_rows_keep_their_keys(native_host):
    from core.history import DeleteGroupCommand
    from views import wgpu_library as lib

    win = native_host.window
    vp, gs, _, _, _ = _scene(win)
    ctx = lib.Ctx(win, win.view)
    def rows():
        materials = [row["key"] for row in lib.MaterialsProvider()._model_items(ctx)]
        components = {row["key"]: row["uses"] for row in lib.ComponentsProvider().items(ctx)
                      if row["source"] == "model"}
        return materials, components
    mats, initial = rows()
    vp.history.execute(DeleteGroupCommand(gs[5]))
    after_mats, after = rows()
    assert after_mats == mats
    assert list(after) == list(initial)
    assert sum(initial.values()) == sum(after.values()) + 1
    family = next(row for row in lib.ComponentsProvider()._model(ctx) if row[2] and row[2][0].mesh is gs[1].mesh)
    assert len(family[2]) == 1 + sum(g.mesh is gs[1].mesh for g in vp.scene.groups)


def test_only_groups_only_commands_carry_the_groups_mark_forward(native_host):
    from core.group import copy_group
    from core.history import (CompoundCommand, DeleteGroupCommand,
                              InsertGroupCommand, SetFaceColorCommand)
    vp, gs, box, plain, glass = _scene(native_host)
    h = vp.history
    cv = lambda: vp.scene.content_version            # noqa: E731
    h.feed.readers.mark("mesh_paint", cv())
    h.execute(DeleteGroupCommand(gs[0]))
    h.execute(CompoundCommand([InsertGroupCommand(copy_group(gs[1]))]))
    h.undo()
    assert h.feed.readers.current("mesh_paint", cv())
    h.execute(SetFaceColorCommand(list(plain[0].mesh.faces), (1, 0, 0)))
    from core.protocol.keys import mesh_key
    assert (not h.feed.readers.current("mesh_paint", cv())
            or mesh_key(plain[0].mesh) in h.feed.readers.dirty_paint_meshes())


def test_inserting_a_nested_link_rebuilds_the_filing_projection(native_host, tmp_path):
    from core.group import Group
    from core.history import InsertGroupCommand
    from tests.native_tree import FilingRows
    browser = FilingRows(native_host)
    browser.read()
    before = browser.model
    link = Group(name="Detached link")
    link.ext = {"link": {"path": str(tmp_path / "Annex.igz")}}
    holder = Group(name="Site")
    holder.children = [link]
    native_host.viewport.history.execute(InsertGroupCommand(holder))
    rows = browser.read()
    assert rows == browser.read(fresh=True)
    assert any(t[0] == "n:link:Annex" for t, _ in rows)
    assert browser.model is not before
    assert native_host.viewport.history.undo()
    assert browser.read() == browser.read(fresh=True)


def test_two_panes_readdress_a_repeated_uid_and_undo_independently(native_host):
    from core.group import Group
    from core.history import InsertGroupCommand
    from tests.native_tree import FilingRows

    def branch(name):
        top = Group(name=name)
        top.children = [Group(name=name + " leaf", uid="shared-leaf")]
        return top

    vp = native_host.viewport
    left, right = branch("Left"), branch("Right")
    vp.history.execute(InsertGroupCommand(left))
    panes = [FilingRows(native_host, ["i:" + g.uid for g in (left, right)]) for _ in range(2)]

    def leaves(pane):
        rows = pane.read()
        assert rows == pane.read(fresh=True)
        return {t[0]: t[1] for t, _ in rows if t[0].startswith("c:")}

    assert all(leaves(p) == {"c:shared-leaf": "Left leaf"} for p in panes)
    models = [p.model for p in panes]
    vp.history.execute(InsertGroupCommand(right))
    for p in reversed(panes):
        addressed = leaves(p)
        assert set(addressed.values()) == {"Left leaf", "Right leaf"}
        assert all(key.startswith("c:@[") for key in addressed)
    assert vp.history.undo()
    for p, model in zip(panes, models):
        assert leaves(p) == {"c:shared-leaf": "Left leaf"}
        assert p.model is model


def test_worker_prepared_tree_can_receive_edits_before_its_first_frame(native_host):
    from core.group import Group
    from core.history import InsertGroupCommand
    from tests.native_tree import FilingRows
    from views.wgpu_ui import prepare_tree_model

    vp = native_host.viewport
    vp.history.execute(InsertGroupCommand(Group(name="Site")))
    panes = [FilingRows(native_host) for _ in range(2)]
    for p in panes:
        # The worker prepares membership before the view can see the feed.
        p.view._ui_tree_model = ((id(vp.scene), vp.scene.content_version),
                                 prepare_tree_model(vp.scene))
    initial = panes[0].model
    panes[0].read()
    assert panes[0].model is initial
    vp.history.execute(InsertGroupCommand(Group(name="Tower")))
    for p in panes:
        rows = p.read()
        assert rows == p.read(fresh=True)
        assert "Tower" in {t[1] for t, _ in rows}
    assert panes[0].model is initial
