# SPDX-License-Identifier: GPL-3.0-or-later
"""Explode inside a component.

Components keep their parts: the menu's Explode asks for Make Unique first
inside one, as Make Group does. The command itself, run inside an assembly
(a component holding components), still has to undo in place: the assembly's
matrix was down in its parts when it ran, and comes back up on leaving."""
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from built_core import Matrix4 as QMatrix4x4, Vec3 as V
import pytest

from tests.native_tree import group_rows
from core.group import Group, copy_group, iter_placements
from core.history import ExplodeGroupCommand
from core.mesh import Mesh


def square():
    mesh = Mesh()
    mesh.add_face([V(0, 0, 0), V(1, 0, 0), V(1, 1, 0), V(0, 1, 0)])
    return Group(mesh)


def points(group):
    return sorted(tuple(round(c, 4) for c in (p.x(), p.y(), p.z()))
                  for g, matrix in iter_placements(group)
                  for f in g.mesh.faces for v in f.vertices
                  for p in [matrix.map(v) if matrix is not None else v])


def model(copies=True):
    target = Group()
    child = square()
    target.adopt([child])
    target.xform.translate(2, 3, 0)
    parent = Group()
    parent.adopt([target])
    parent.xform.translate(10, 7, 0)
    parent.xform.rotate(30, V(0, 0, 1))
    peer = copy_group(parent, V(20, 0, 0)) if copies else None
    return [parent] + ([peer] if peer else []), parent, peer, target, child


def _window(native_host, groups):
    win = native_host.window
    vp = win.viewport
    vp.scene.groups[:] = groups
    vp.scene.version += 1
    return win, vp


@pytest.mark.parametrize('copies', [False, True])
def test_explode_in_an_assembly_preserves_placement_and_undoes_in_place(native_host, copies):
    """Left the way the app leaves, through the viewport: with copies, the
    assembly's matrix comes back up out of its parts on leaving, and the
    explode undoes inside the step that does it."""
    groups, parent, peer, target, child = model(copies)
    win, vp = _window(native_host, groups)
    scene, hist = vp.scene, vp.history
    before = [points(g) for g in scene.groups]
    entry = QMatrix4x4(target.xform)
    counterpart = peer.children[0].children[0] if peer else None
    if counterpart:
        counterpart.ifc = {"name": "Peer child"}
    vp.begin_group_edit(parent)
    win._execute_make(ExplodeGroupCommand(target))
    assert hist.last_error is None and hist.last_rejected is None
    assert parent.children == [child]
    assert [points(g) for g in scene.groups] == before
    if peer:
        assert len(peer.children) == 1
        if getattr(scene.mesh, "_instance_definition", None) is parent.mesh:
            assert peer.children[0] is counterpart
            assert counterpart.ifc["name"] == "Peer child"
            assert not peer.children[0].children
            assert peer.children[0].mesh is child.mesh
        else:
            assert peer.children[0].children
    vp.end_group_edit()
    assert [points(g) for g in scene.groups] == before
    assert hist.undo()
    assert parent.children == [target]
    assert [points(g) for g in scene.groups] == before
    if peer:
        assert target.xform == entry and parent.mesh is peer.mesh
    assert hist.redo()
    assert parent.children == [child]
    assert [points(g) for g in scene.groups] == before


@pytest.mark.parametrize('copies', [False, True])
def test_the_menu_explode_inside_a_component_asks_for_make_unique(native_host, copies):
    groups, parent, peer, target, child = model(copies)
    win, vp = _window(native_host, groups)
    vp.begin_group_edit(parent)
    vp.scene.selection = {target}
    steps = len(vp.history.undo_stack)
    win._on_explode_group()
    assert parent.children == [target] and target.children == [child]
    assert len(vp.history.undo_stack) == steps
    from built_core.i18n import tr
    assert group_rows(native_host, target)[tr("Explode Group")]
