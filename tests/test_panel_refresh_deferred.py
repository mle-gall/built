# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The Library and Project tree sample document changes when a frame needs them."""
from __future__ import annotations

import gc
import weakref
import pytest
from ingetrazo_core import Matrix4, Vec3
from ingetrazo_render import chrome_sheet
from tests.test_library_sheet import win, frame, tiles  # noqa: F401
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from views import wgpu_library as library


def _count_calls(monkeypatch, obj, name):
    calls = []
    original = getattr(obj, name)
    def counted(*args, **kwargs):
        calls.append(1)
        return original(*args, **kwargs)

    monkeypatch.setattr(obj, name, counted)
    return calls


@pytest.mark.parametrize("kind,provider", [
    ("components", library.ComponentsProvider), ("materials", library.MaterialsProvider),
])
def test_hidden_library_waits_until_shown(win, monkeypatch, kind, provider):
    calls = _count_calls(monkeypatch, provider, "items")
    for _ in range(3):
        win.viewport.scene.version += 1
        chrome_sheet.before_frame(win.view, win)
    assert calls == []
    library.open_library(win, kind)
    frame(win)
    assert len(calls) == 1


def test_a_burst_of_edits_refreshes_the_visible_panels_once(win, monkeypatch):
    from views import wgpu_ui
    p, group, _ = browser_model()
    comp = _count_calls(monkeypatch, library.ComponentsProvider, "items")
    tree = _count_calls(monkeypatch, wgpu_ui, "prepare_tree_model")
    library.open_library(win, "components")
    frame(win)
    comp.clear()
    for i in range(5):
        group.name = str(i)
        p.scene.version += 1
        win.viewport.scene.version += 1
    assert comp == tree == []
    frame(win)
    assert p.rows()["i:" + group.uid][0] == "4"
    frame(win)
    p.rows()
    assert len(comp) == len(tree) == 1


def test_a_selection_leaves_the_in_model_lists_as_they_are(win, monkeypatch):
    from core.group import Group
    scene = win.viewport.scene
    group = Group(name="Placed")
    group.xform = Matrix4()
    group.component = True
    scene.groups.append(group)
    scene.version += 1
    comp = _count_calls(monkeypatch, library.ComponentsProvider, "items")
    mat = _count_calls(monkeypatch, library.MaterialsProvider, "items")
    library.open_library(win, "components")
    frame(win)
    scene.select([group])
    frame(win)
    assert len(comp) == len(mat) == 1
    group.name = "After edit"
    scene.version += 1
    assert any("After edit" in str(row) for row in tiles(frame(win)))
    assert len(comp) == len(mat) == 2


def test_hidden_panels_let_the_previous_document_go(win):
    from core.group import Group
    from views import wgpu_ui
    group = Group(name="previous-document-marker")
    group.xform = Matrix4()
    group.component = True
    scene = win.viewport.scene
    scene.groups.append(group)
    scene.version += 1
    library.open_library(win, "components")
    assert any(group.name in str(row) for row in tiles(frame(win)))
    wgpu_ui._tree(win.view, win)
    # The recording renderer stores the Python arguments; the native renderer
    # consumes these immediately. Keep the presentation surface itself alive.
    view = win.view
    view.renderer.calls.clear()
    reference = weakref.ref(group.mesh)
    chrome_sheet.close(win)
    del group, scene
    win.new_document()
    assert win.close_document(win.documents()[0], ask=False)
    win._reclaim_closed_documents()
    gc.collect()
    assert view._closed and reference() is None


def browser_model():
    from core.group import Group
    from core.mesh import Mesh
    from ingetrazo_core import Vec3 as V
    from tests.test_project_drawing_filters import Project

    p = Project()
    mesh = Mesh()
    face = mesh.add_face([V(0, 0, 0), V(1, 0, 0), V(1, 1, 0)])
    group = Group(mesh, "Object")
    p.scene.groups.append(group)
    p.action("toggle", "n:")
    return p, group, face


def test_a_selection_notice_leaves_the_browser_rows_as_they_are(monkeypatch):
    from views import wgpu_ui
    p, group, _ = browser_model()
    scans = _count_calls(monkeypatch, wgpu_ui, "prepare_tree_model")
    p.scene.select([group])
    assert p.rows()["i:" + group.uid][1][9] and scans == []
    group.name = "Renamed"
    p.scene.version += 1
    assert p.rows()["i:" + group.uid][0] == "Renamed" and len(scans) == 1


def test_a_move_leaves_the_material_swatches_as_they_are(win, monkeypatch):
    from core import geometry_arrays
    from core.group import Group
    from core.mesh import Mesh
    from core.history import MoveGroupCommand, SetFaceColorCommand
    from ingetrazo_core import material_survey
    vp = win.viewport
    mesh = Mesh()
    face = mesh.add_face([Vec3(), Vec3(1, 0, 0), Vec3(1, 1, 0)])
    group = Group(mesh, "moved")
    vp.scene.groups.append(group)
    vp.scene.version += 1
    def survey():
        return material_survey(vp.scene, vp.history)
    survey()
    scans = _count_calls(monkeypatch, geometry_arrays, "face_attributes")
    vp.history.execute(MoveGroupCommand(group, Vec3(2, 0, 0)))
    survey()
    vp.history.undo()
    survey()
    assert scans == []
    vp.history.execute(SetFaceColorCommand([face], (1.0, 0.0, 0.0)))
    assert "#FF0000" in survey()[1] and scans


def test_a_move_leaves_the_browser_rows_as_they_are(monkeypatch):
    from core.history import MoveGroupCommand, SetFaceColorCommand
    from ingetrazo_core import Vec3 as V
    from views import wgpu_ui
    p, group, face = browser_model()
    history = p.win.viewport.history
    scans = _count_calls(monkeypatch, wgpu_ui, "prepare_tree_model")
    def move():
        history.execute(MoveGroupCommand(group, V(2, 0, 0)))
        return p.rows()

    move()
    assert history.undo()
    p.rows()
    assert scans == []
    history.execute(SetFaceColorCommand([face], (.9, 0., 0.)))
    p.rows()
    move()
    assert len(scans) == 1
    group.name = "External rename"
    p.scene.version += 1
    assert move()["i:" + group.uid][0] == group.name and len(scans) == 2
    p.win._outliner_kinds = [lambda g: None]
    move()
    assert len(scans) == 3


def test_a_geometry_edit_leaves_the_browser_rows_as_they_are(monkeypatch):
    from core.history import AddEdgeCommand, NativeCommand, SnapshotMutation
    from ingetrazo_core import Vec3 as V
    from views import wgpu_ui
    p, group, _ = browser_model()
    scans = _count_calls(monkeypatch, wgpu_ui, "prepare_tree_model")
    mesh = group.mesh
    history = p.win.viewport.history
    history.execute(SnapshotMutation(lambda s: mesh.add_edge(V(0,0,1), V(1,0,1)), mesh=mesh))
    p.rows()
    history.execute(AddEdgeCommand(V(3,0,0), V(4,0,0)))
    p.rows()
    assert history.undo()
    p.rows()
    assert not scans
    history.execute(NativeCommand.rename(group, "New name"))
    assert p.rows()["i:" + group.uid][0] == "New name" and len(scans) == 1


def test_two_browser_panes_each_receive_a_change_seen_by_the_other():
    from core.history import MoveGroupCommand, NativeCommand
    from ingetrazo_core import Vec3 as V
    from tests.test_tree_links import _View
    from views import wgpu_ui

    p, group, _ = browser_model()
    second = _View()
    second._ui_exp = {"n:"}

    def name():
        wgpu_ui._tree(second, p.win)
        return next(t[1] for t, _ in second.renderer.calls["ui_set_tree"][0] if t[0] == "i:" + group.uid)

    assert name() == "Object"
    h = p.win.viewport.history
    h.execute(NativeCommand.rename(group, "Door"))
    assert p.rows()["i:" + group.uid][0] == "Door"
    h.execute(MoveGroupCommand(group, V(1, 0, 0)))
    p.rows()
    assert name() == "Door"
