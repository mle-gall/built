# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Lock and Unlock from the right-click on the model (#782).

The Browser's padlock was the only way to lock a group, and the only way
back: a locked group refuses every click, so its right-click must find it
on its own (as a locked reference image's does) and offer Unlock."""
from __future__ import annotations

import pytest
from ingetrazo_core import Matrix4 as QMatrix4x4, Vec3 as QVector3D
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from views import context_menu
from core.group import Group  # noqa: E402
from core.history import NativeCommand  # noqa: E402
from core.layers import Layer  # noqa: E402
from core.mesh import Mesh  # noqa: E402


def V(x, y, z=0.0):
    return QVector3D(float(x), float(y), float(z))


def _group(scene, name, x0=0.0):
    m = Mesh()
    m.add_face([V(0, 0), V(2, 0), V(2, 2), V(0, 2)])
    g = Group(m, name)
    xf = QMatrix4x4()
    xf.translate(x0, 0, 0)
    g.xform = xf
    scene.groups.append(g)
    scene.version += 1
    return g


@pytest.fixture
def win(native_host):
    win = native_host.window
    vp = win.viewport
    vp.camera.set_aspect(1000, 600)
    vp.camera.set_view("top")
    vp.camera.target = V(3, 1, 0)
    vp.camera.distance = 12
    return win


def test_lock_locks_every_selected_group_in_one_step(win):
    scene = win.viewport.scene
    a, b = _group(scene, "muro"), _group(scene, "losa", 4)
    scene.select([a, b])
    menu = context_menu.build(win)
    assert menu.find("lock").enabled
    assert context_menu.run(win, None, menu.kind, "lock", menu.sel)
    assert a.locked and b.locked
    assert a not in scene.selection and b not in scene.selection
    assert win.viewport.history.undo()           # one step for both
    assert not a.locked and not b.locked


def test_lock_leaves_the_edit_of_the_group_it_locks(win):
    scene = win.viewport.scene
    g = _group(scene, "muro")
    win.viewport.begin_group_edit(g)
    assert scene.edit_group is g
    context_menu.run(win, None, "group", "lock", [g])
    assert scene.edit_group is None and g.locked


def test_no_lock_entry_without_a_group(win):
    scene = win.viewport.scene
    face = scene.mesh.add_face([V(0, 0), V(1, 0), V(1, 1), V(0, 1)])
    scene.select([face])
    assert context_menu.build(win).find("lock") is None


def test_the_locked_group_under_the_cursor_is_found_and_unlocked(win):
    vp = win.viewport
    scene = vp.scene
    g = _group(scene, "muro")
    px = vp._world_to_pixel(V(1, 1, 0))
    assert vp.pick_group(*px) is g                # the block is built
    vp.history.execute(NativeCommand.lock([g], True))
    assert vp.pick_group(*px) is None             # still not pickable
    assert vp.pick_locked_group(*px) is g         # but the menu finds it
    empty = vp._world_to_pixel(V(3, 1, 0))
    assert vp.pick_locked_group(*empty) is None   # off the group
    menu = context_menu.build(win, locked_group=g)
    assert menu.find("unlockgroup").label == "Unlock “muro”"
    assert context_menu.run(win, None, menu.kind, "unlockgroup", menu.sel, locked_group=g)
    assert not g.locked and g in scene.selection
    assert vp.history.undo()                       # undoable
    assert g.locked


def test_an_unlocked_group_is_not_offered_for_unlock(win):
    vp = win.viewport
    _group(vp.scene, "muro")
    px = vp._world_to_pixel(V(1, 1, 0))
    assert vp.pick_locked_group(*px) is None


def test_a_hidden_or_layer_locked_group_is_not_offered(win):
    vp = win.viewport
    scene = vp.scene
    g = _group(scene, "muro")
    g.locked = True
    g.layer = "Capa"
    scene.layers.append(Layer("Capa", locked=True))
    scene.version += 1
    px = vp._world_to_pixel(V(1, 1, 0))
    assert vp.pick_locked_group(*px) is None       # its layer is locked
    scene.layers[-1].locked = False
    scene.version += 1
    assert vp.pick_locked_group(*px) is g
    scene.layers[-1].visible = False
    scene.version += 1
    assert vp.pick_locked_group(*px) is None       # its layer is hidden


def test_a_loose_face_in_front_hides_the_locked_group(win):
    vp = win.viewport
    scene = vp.scene
    g = _group(scene, "muro")
    g.locked = True
    scene.mesh.add_face([V(0, 0, 1), V(2, 0, 1), V(2, 2, 1), V(0, 2, 1)])
    scene.version += 1
    px = vp._world_to_pixel(V(1, 1, 0))
    assert vp.pick_locked_group(*px) is None
