"""run_python (the AI bridge and the console's executor) is one undoable step
for edits BELOW the top level too. A ride catwalk rebuild adopted 432 new
parts into a nested container: the bridge said ``changed: False``, undo had
nothing to undo, and the agent wrote its own undo command (2026-11-14)."""
from __future__ import annotations

import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from ingetrazo_core import Vec3 as QVector3D

from core import ai  # noqa: E402
from core.group import Group  # noqa: E402
from core.mesh import Mesh  # noqa: E402


def _tri(x=0.0):
    m = Mesh()
    m.add_face([QVector3D(x, 0, 0), QVector3D(x + 1, 0, 0), QVector3D(x, 1, 0)])
    return m


def _nested():
    from types import SimpleNamespace
    from core.history import History
    from core.scene import Scene
    leaf = Group(_tri(), name="old part")
    holder = Group(name="holder")
    holder.adopt([leaf])
    top = Group(name="top")
    top.adopt([holder])
    scene = Scene(groups=[top])
    return SimpleNamespace(scene=scene, history=History(scene),
                           notify_scene_changed=lambda: None), holder, leaf


def test_nested_adopt_is_one_undoable_step():
    vp, holder, leaf = _nested()
    scope = {"__name__": "__ai__", "holder": holder, "Group": Group, "make": _tri}
    r = ai.run_transactional(vp, "holder.adopt([Group(make(5), name='new part')])", scope)
    assert r["error"] is None and r["changed"] and "warning" not in r
    assert [c.name for c in holder.children] == ["new part"]
    assert vp.history.undo()
    assert holder.children == [leaf]
    assert vp.history.redo()
    assert [c.name for c in holder.children] == ["new part"]


def test_mesh_swap_and_hide_are_undone():
    vp, holder, leaf = _nested()
    old_mesh = leaf.mesh
    scope = {"__name__": "__ai__", "leaf": leaf, "make": _tri}
    r = ai.run_transactional(vp, "leaf.mesh = make(3); leaf.hidden = True", scope)
    assert r["changed"]
    assert vp.history.undo()
    assert leaf.mesh is old_mesh and leaf.hidden is False


def test_error_rolls_back_the_nested_tree():
    vp, holder, leaf = _nested()
    scope = {"__name__": "__ai__", "holder": holder, "Group": Group}
    r = ai.run_transactional(vp, "holder.adopt([]); raise ValueError('boom')", scope)
    assert r["error"] is not None and not r["changed"]
    assert holder.children == [leaf]


def test_in_place_mesh_edit_reports_change_and_warns():
    vp, holder, leaf = _nested()
    scope = {"__name__": "__ai__", "leaf": leaf, "QVector3D": QVector3D}
    r = ai.run_transactional(vp, "leaf.mesh.move_vertex(leaf.mesh.vertices[0], QVector3D(0, 0, 1))", scope)
    assert r["changed"] and "in place" in r["warning"]


def test_a_read_only_script_leaves_no_step():
    vp, holder, leaf = _nested()
    depth = len(vp.history.undo_stack)
    r = ai.run_transactional(vp, "print(len(holder.children))", {"__name__": "__ai__", "holder": holder})
    assert r["stdout"].strip() == "1" and not r["changed"]
    assert len(vp.history.undo_stack) == depth


def test_failure_warns_if_an_existing_mesh_was_edited_in_place():
    vp, holder, leaf = _nested()
    result = ai.run_transactional(vp,
        "leaf.hidden = True; leaf.mesh.move_vertex(leaf.mesh.vertices[0], QVector3D(0, 0, 1)); "
        "raise ValueError('failed')", {"leaf": leaf})
    assert result["error"] and "in place" in result["warning"]
    assert not leaf.hidden and not vp.history.undo_stack
