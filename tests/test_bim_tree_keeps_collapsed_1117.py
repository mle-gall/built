# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The takeoff table keeps the branches the user folded (#1117).

Any change of content rebuilds the table, and opening a group is one: every
folded branch of the tree came open again on picking a row."""
from __future__ import annotations

from ingetrazo_core import Vec3 as V
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from tests.test_tree_links import _View
from views import wgpu_ui

from core.bim import tag_group  # noqa: E402
from core.group import Group  # noqa: E402
from core.mesh import Mesh  # noqa: E402


def _box(x):
    m = Mesh()
    p = [V(x, 0, 0), V(x + 1, 0, 0), V(x + 1, 1, 0), V(x, 1, 0),
         V(x, 0, 1), V(x + 1, 0, 1), V(x + 1, 1, 1), V(x, 1, 1)]
    for idx in ((3, 2, 1, 0), (4, 5, 6, 7), (0, 1, 5, 4),
                (1, 2, 6, 5), (2, 3, 7, 6), (3, 0, 4, 7)):
        m.add_face([p[i] for i in idx])
    return m


def _model(scene):
    """floor1 ( walls ( W1 ) ), floor2 ( W2 ), with the walls tagged."""
    floors = []
    for i, name in enumerate(("floor1", "floor2")):
        leaf = Group(_box(i * 3))
        leaf.name = f"W{i + 1}"
        tag_group(leaf, "IfcWall", leaf.name)
        outer = Group(Mesh(), name=name)
        if i == 0:
            walls = Group(Mesh(), name="walls")
            walls.children.append(leaf)
            outer.children.append(walls)
        else:
            outer.children.append(leaf)
        scene.groups.append(outer)
        floors.append(outer)
    scene.version += 1
    return floors


def rows(view, win):
    wgpu_ui._tree(view, win)
    return {text[0]: state for text, state in view.renderer.calls["ui_set_tree"][0]}


def toggle(view, win, key):
    wgpu_ui._tree_action(win, view, "toggle" + wgpu_ui.SEP + key)
    return rows(view, win)


def test_opening_a_group_leaves_the_folded_branches_folded(native_host):
    from views.tree_navigation import open_to
    win, vp, view = native_host.window, native_host.viewport, _View()
    first, second = _model(vp.scene)
    walls, = first.children
    view._ui_exp = {"n:", "i:" + first.uid, "c:" + walls.uid, "i:" + second.uid}
    wgpu_ui._measure(view, win)
    before = rows(view, win)
    assert before["c:" + walls.uid][8] and before["i:" + second.uid][8]
    toggle(view, win, "c:" + walls.uid)
    toggle(view, win, "i:" + second.uid)
    cv = vp.scene.content_version
    assert open_to(vp, [first, walls])
    vp.scene.select([walls.children[0]])
    assert vp.scene.content_version != cv
    assert "context:fold" in rows(view, win)
    assert open_to(vp, [])
    restored = rows(view, win)
    assert restored["i:" + first.uid][8]
    assert not restored["c:" + walls.uid][8] and not restored["i:" + second.uid][8]


def test_a_new_branch_can_open_without_reopening_a_folded_tagged_group(native_host):
    win, scene, view = native_host.window, native_host.viewport.scene, _View()
    _, floor = _model(scene)
    tag_group(floor, "IfcBuildingStorey", "tagged floor")
    scene.version += 1
    view._ui_exp = {"n:", "i:" + floor.uid}
    assert rows(view, win)["i:" + floor.uid][8]
    toggle(view, win, "i:" + floor.uid)
    extra = Group(_box(9), name="W3")
    tag_group(extra, "IfcWall", "W3")
    outer = Group(name="floor3")
    outer.adopt([extra])
    scene.groups.append(outer)
    scene.version += 1
    assert not rows(view, win)["i:" + floor.uid][8]
    after = toggle(view, win, "i:" + outer.uid)
    assert after["i:" + outer.uid][8] and "c:" + extra.uid in after
    assert not after["i:" + floor.uid][8]
