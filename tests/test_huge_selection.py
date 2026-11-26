# SPDX-License-Identifier: GPL-3.0-or-later
"""A huge selection costs its work once per change, never per frame.

Select All on a big model (600k faces and edges) made every frame take
seconds: the face dots were rebuilt on every camera move (the eye was part
of their key), and every frame walked the whole selection several times
(type tests, identity sets) before finding nothing had changed. The marks
are now keyed on the selection's native revision, the dot colour of the
side the camera sees is picked on the GPU, the hover has its own small
upload, and the per-kind reads come from the selection store's own index.
"""
from __future__ import annotations

import math
from types import SimpleNamespace

import numpy as np
import pytest

from built_core import ElementSet, VERTEX, Vec3 as V
from built_core.group import Group
from built_core.mesh import Mesh
from tests.test_metal_parity_input import view  # noqa: F401  (fixture)


class Counting(ElementSet):
    """A selection that counts the walks over it."""
    walks = 0

    def __iter__(self):
        type(self).walks += 1
        return super().__iter__()


class Recorder(SimpleNamespace):
    """A renderer that records what is uploaded."""

    def __init__(self):
        super().__init__(calls=[])
        for name in ("set_highlight", "set_face_marks", "set_hover_marks", "set_preview_mesh", "set_lines"):
            setattr(self, name, self._record(name))

    def _record(self, name):
        def record(*args, **kwargs):
            self.calls.append((name, args))
        return record

    def names(self):
        return [c[0] for c in self.calls]


def grid(mesh, n, z=0.0, x0=0.0):
    faces = []
    for i in range(n):
        for j in range(n):
            faces.append(mesh.add_face([V(x0 + i, j, z), V(x0 + i + 1, j, z), V(x0 + i + 1, j + 1, z), V(x0 + i, j + 1, z)]))
    return faces


def refresh(view):
    view.scene.version += 1
    view.set_scene(view.scene)


def test_selection_store_keeps_each_kind_in_order():
    sel = ElementSet()
    mesh = Mesh()
    a, b = grid(mesh, 1)[0], grid(mesh, 1, z=1)[0]
    edge = mesh.edges[0]
    g = Group(name="g")
    sel.update([a, g, edge, b])
    assert sel.values_of("face") == [a, b]
    assert sel.values_of("placement") == [g]
    assert sel.values_of("edge") == [edge]
    assert sel.element_keys("face") == [a.key, b.key]
    rev = sel.revision
    sel.discard_identities([a.key, 12345])
    assert sel.values_of("face") == [b] and sel.revision != rev
    assert sel.snapshot()[:3] == (3, 1, 1)
    rev = sel.revision
    sel.discard_identities([])
    assert sel.revision == rev
    other = ElementSet([edge])
    sel.replace(other)
    assert sel.values_of("face") == [] and sel.values_of("edge") == [edge]
    sel.update([a, b])
    sel.intersection_update([b])
    assert sel.values_of("face") == [b] and sel.values_of("edge") == []
    sel.clear()
    assert all(sel.values_of(k) == [] for k in ("face", "edge", "placement", "vertex", "other"))
    with pytest.raises(ValueError):
        sel.values_of("nonsense")


def test_store_marks_and_stale_keys_in_one_call():
    mesh = Mesh()
    a, b = grid(mesh, 2)[:2]
    b.attrs["color"] = (0.95, 0.45, 0.16)
    store = mesh._store
    from core._native import qt_hypot
    corners, rows, counts, missing = store.mark_triangles([a.key, b.key, 99], qt_hypot(), (1.0, 0.0, 0.0))
    assert list(counts) == [2, 2] and missing == [99]
    # Table rows (what a loaded file's faces carry); paint set since lives
    # in the mesh's own attributes, which the view reads first.
    assert list(rows) == [-1, -1] and b.key in mesh._attr_own
    expected = [tuple(p.toTuple()) for t in a.triangulate() for p in t]
    assert np.allclose(corners[:6], np.array(expected) - (1, 0, 0))
    edge = mesh.edges[0]
    ends, missing = store.mark_segments([edge.key, 7], (0, 0, 0))
    assert missing == [7] and np.allclose(ends, [edge.a.toTuple(), edge.b.toTuple()])
    assert store.stale_keys("face", [a.key, 5]) == [5]
    mesh.remove_face(a)
    assert store.stale_keys("face", [a.key, b.key]) == [a.key]


def test_unchanged_selection_walks_nothing_per_frame(view):
    faces = grid(view.scene.mesh, 12)
    view.scene.selection = Counting()
    if not isinstance(view.scene.selection, Counting):
        pytest.skip("the scene keeps its own selection object")
    refresh(view)
    view.scene.select(faces + list(view.scene.mesh.edges))
    view.renderer = rec = Recorder()
    view.sync_selection()
    assert rec.names().count("set_face_marks") == 1
    assert rec.names().count("set_preview_mesh") == 1
    Counting.walks = 0
    rec.calls.clear()
    for _ in range(4):
        view.camera.orbit(20.0, 5.0, 600)          # the camera moves
        view.sync_selection()
        view._selection_box_vertices()
    assert Counting.walks == 0                     # no walk over the selection
    assert rec.calls == []                         # and nothing uploaded


def test_hover_change_uploads_only_the_hover(view):
    faces = grid(view.scene.mesh, 6)
    refresh(view)
    view.scene.select(faces)
    view.renderer = rec = Recorder()
    view.sync_selection()
    rec.calls.clear()
    view.hover_rows = (0,)
    view.sync_selection()
    assert "set_face_marks" not in rec.names() and "set_preview_mesh" not in rec.names()
    assert rec.names().count("set_hover_marks") == 1
    assert rec.names().count("set_highlight") == 1


def test_selection_change_reuploads_the_marks(view):
    faces = grid(view.scene.mesh, 3)
    refresh(view)
    view.scene.select(faces[:2])
    view.renderer = rec = Recorder()
    view.sync_selection()
    first = [a for n, a in rec.calls if n == "set_face_marks"][-1][0]
    view.scene.select(faces[:5])
    view.sync_selection()
    second = [a for n, a in rec.calls if n == "set_face_marks"][-1][0]
    assert len(first) == 2 * 6 * 28 and len(second) == 5 * 6 * 28
    view.scene.clear_selection()
    view.sync_selection()
    assert len([a for n, a in rec.calls if n == "set_face_marks"][-1][0]) == 0


def test_face_marks_carry_both_sides_and_skip_hidden_faces(view):
    from views.document_interaction import SELECTION_DOT_ALT_COLOR, SELECTION_DOT_COLOR
    from views.wgpu_parity import face_marks
    plain, orange, hidden = grid(view.scene.mesh, 3)[:3]
    orange.attrs["color"] = (0.95, 0.45, 0.16)
    hidden.attrs["hidden"] = True
    refresh(view)
    marks = np.frombuffer(bytes(face_marks(view, [plain, orange, hidden])), VERTEX)
    assert len(marks) == 12                         # the hidden face has none
    front = {tuple(c[:3]) for c in marks["color"]}
    assert len(front) == 2
    # The back's dot colour rides where a normal would be (preview.wgsl).
    assert np.allclose(marks["normal"][0], SELECTION_DOT_COLOR[:3], atol=1e-6)
    assert np.allclose(marks["normal"][-1], SELECTION_DOT_COLOR[:3], atol=1e-6)
    alt = tuple(int(c * 255) for c in SELECTION_DOT_ALT_COLOR[:3])
    assert alt in front
    assert view._selected_mark_count == 12


def test_face_marks_sit_on_a_face_inside_an_open_moved_group(view):
    from built_core import Matrix4
    from views.wgpu_parity import face_marks
    group = Group(Mesh(), name="moved")
    m = Matrix4()
    m.translate(10, 0, 0)
    group.xform = m
    grid(group.mesh, 1)
    view.scene.groups.append(group)
    refresh(view)
    view.scene.begin_group_edit(group)
    refresh(view)
    face = view.scene.mesh.faces[0]
    marks = np.frombuffer(bytes(face_marks(view, [face])), VERTEX)
    xs = marks["pos"][:, 0]
    # Where the face is drawn (the store's model frame), not at the
    # definition's own origin.
    expected = [p.toTuple()[0] for t in face.triangulate() for p in t]
    assert np.allclose(xs, expected)


def test_many_selected_groups_show_one_box(view):
    from views.wgpu_edit_context import selection_box_vertices
    part = Mesh()
    grid(part, 1)
    groups = []
    for i in range(300):
        g = Group(part, name=f"p{i}")
        m = __import__("built_core").Matrix4()
        m.translate(2.0 * i, 0, 0)
        g.xform = m
        groups.append(g)
    view.scene.groups.extend(groups)
    refresh(view)
    view.scene.select(groups)
    key, vertices = selection_box_vertices(view)
    assert len(vertices) == 24                      # 12 edges, one box
    xs = vertices["pos"][:, 0]
    assert xs.min() == pytest.approx(0.0) and xs.max() == pytest.approx(2.0 * 299 + 1)
    view.scene.select(groups[:1])
    one = len(selection_box_vertices(view)[1])
    view.scene.select(groups[:3])
    key, vertices = selection_box_vertices(view)
    assert one and len(vertices) == 3 * one         # a few: one box each


def test_loose_row_counts_from_the_selection_counts():
    from built_core.scene import Scene
    from built_core import project_context_rows
    scene = Scene()
    faces = grid(scene.mesh, 3)
    scene.select(faces[:4] + list(scene.mesh.edges)[:5])
    rows = project_context_rows(scene, "", set())
    loose = next(r for r in rows if r[0][0] == "geometry:loose")
    assert 9 in loose[1]


def test_select_all_is_a_view_change(native_host):
    win = native_host.window
    scene = win.viewport.scene
    grid(scene.mesh, 2)
    version, view_version = scene.version, scene.view_version
    win._on_select_all()
    assert scene.selection
    assert scene.version > version                 # the selection moved (issue #38)
    assert scene.version - scene.view_version == version - view_version   # the document did not


def test_undo_of_an_edit_on_a_huge_selection(view):
    from built_core.history import EraseSelectionCommand
    faces = grid(view.scene.mesh, 15)
    refresh(view)
    view.scene.select(faces)
    view.renderer = Recorder()
    view.sync_selection()
    count = len(view.scene.mesh.faces)
    view.renderer = None                            # the edit itself is not what is tested
    view.source_viewport.history.execute(EraseSelectionCommand([], list(view.scene.selection)))
    assert len(view.scene.mesh.faces) < count
    view.scene.clear_selection()
    refresh(view)
    view.source_viewport.history.undo()
    refresh(view)
    view.renderer = Recorder()
    assert len(view.scene.mesh.faces) == count
    view.scene.select(list(view.scene.mesh.faces))
    view.sync_selection()
    marks = [a for n, a in view.renderer.calls if n == "set_face_marks"][-1][0]
    assert len(marks) == count * 6 * 28


def test_verbs_for_a_selection_of_many_edges(native_host):
    import built_render
    win = native_host.window
    scene = win.viewport.scene
    grid(scene.mesh, 12)                            # 312 edges, past the bulk threshold
    win._on_select_all()
    verbs = built_render.inspector_verbs(win, list(scene.selection))
    assert verbs                                    # read without error, in bulk
    from built_core.layers import selection_layers
    assert selection_layers(scene) == {"Layer 0"}
