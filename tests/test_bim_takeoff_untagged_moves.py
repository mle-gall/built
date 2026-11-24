# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""A Move on an untagged model does not walk it for BIM tags.

Since tags are found inside groups (#1030), the takeoff — run after every
edit, even with the BIM panel hidden — walks every group: 26 ms per edit
on the 21 406 groups of #158, none of them tagged (0.1 ms before). A
command that keeps paint (Move, Rotate, Scale, Flip) keeps every tag
too, so a takeoff that found nothing still finds nothing; anything else
— a tag, an edit outside the history — walks again."""
from __future__ import annotations

from tests.shell_isolation import restore_shell_patches  # noqa: F401
from ingetrazo_core import Vec3 as V
from views import wgpu_ui

from core.group import Group  # noqa: E402
from core.history import MoveGroupCommand  # noqa: E402
from core.mesh import Mesh  # noqa: E402


def _box(x):
    m = Mesh()
    p = [V(x, 0, 0), V(x + 1, 0, 0), V(x + 1, 1, 0), V(x, 1, 0),
         V(x, 0, 1), V(x + 1, 0, 1), V(x + 1, 1, 1), V(x, 1, 1)]
    for q in ([0, 3, 2, 1], [4, 5, 6, 7], [0, 1, 5, 4], [1, 2, 6, 5],
              [2, 3, 7, 6], [3, 0, 4, 7]):
        m.add_face([p[i] for i in q])
    return m


def _model(scene):
    """Two containers holding a box each, nothing tagged."""
    outs = []
    for i in range(2):
        inner = Group(_box(3.0 * i), f"box {i}")
        outer = Group(Mesh(), f"holder {i}")
        outer.adopt([inner])
        outs.append(outer)
    scene.groups += outs
    scene.version += 1
    return outs


def _counting(monkeypatch):
    import core.bim as bim
    calls = []
    real = bim.collect_objects
    monkeypatch.setattr(bim, "collect_objects",
                        lambda s, *a: calls.append(1) or real(s, *a))
    return calls


def measure(host, auto=False):
    wgpu_ui._measure(host.view, host.window, auto)
    return host.viewport.scene.__dict__.get("_inspector_takeoff")


def test_a_move_on_an_untagged_model_skips_the_walk(native_host, monkeypatch):
    vp = native_host.viewport
    outs = _model(vp.scene)
    assert measure(native_host).count == 0
    calls = _counting(monkeypatch)
    vp.history.execute(MoveGroupCommand(outs[0], V(1, 0, 0)))
    assert measure(native_host, auto=True).count == 0
    assert vp.history.undo()
    assert measure(native_host, auto=True).count == 0
    assert not calls


def test_a_tag_after_the_move_is_found(native_host, monkeypatch):
    from core.bim import tag_group
    vp = native_host.viewport
    outs = _model(vp.scene)
    measure(native_host)
    vp.history.execute(MoveGroupCommand(outs[1], V(0, 2, 0)))
    measure(native_host, auto=True)
    calls = _counting(monkeypatch)
    child = outs[0].children[0]
    tag_group(child, "IfcColumn", "C-1")
    vp.scene.version += 1
    takeoff = measure(native_host, auto=True)
    assert calls == [1] and takeoff.count == 1
    assert abs(takeoff.quantities(child)["m3"] - 1.) < 1e-6
    vp.history.execute(MoveGroupCommand(outs[0], V(1, 0, 0)))
    measure(native_host, auto=True)
    assert calls == [1, 1] and abs(takeoff.quantities(child)["m3"] - 1.) < 1e-6


def test_another_project_is_measured_on_its_own(native_host):
    from core.bim import tag_group
    win = native_host.window
    first = win.active_document()
    _model(win.viewport.scene)
    empty = measure(native_host)
    win.new_document()
    scene = win.viewport.scene
    g = Group(_box(0.), "tagged")
    tag_group(g, "IfcBeam", "V-1")
    scene.groups.append(g)
    scene.version += 1
    assert measure(native_host, auto=True) is None
    assert measure(native_host).count == 1
    win.activate_document(first)
    assert measure(native_host, auto=True) is empty and empty.count == 0


def test_an_empty_takeoff_maps_no_tree(native_host, monkeypatch):
    from core import outliner
    _model(native_host.viewport.scene)
    def unexpected(*_):
        raise AssertionError("measuring an empty takeoff walked the Browser parents")
    monkeypatch.setattr(outliner, "parents", unexpected)
    assert measure(native_host).count == 0
