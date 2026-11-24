# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Native Project tree commands with the same selection used by the viewport."""
import pytest

from tests.native_tree import action, group_rows
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from tests.test_browser_layer_order import _container, _group
from tests.test_project_occurrences import _two_planes
from tests.test_tree_links import _visible


@pytest.fixture
def model(native_host):
    level = _container("Level", [_group("Wall"), _group("Slab")])
    house = _container("House", [level, _group("Roof")])
    tree = _group("Tree")
    vp = native_host.viewport
    vp.scene.groups[:] = [house, tree]
    vp.scene.version += 1
    return native_host, vp, house, level, tree


@pytest.mark.parametrize("multi", [False, True])
def test_row_isolation_and_show_all_restore_the_view_and_selection(model, multi):
    host, vp, house, level, tree = model
    vp.scene.select([tree, level] if multi else [tree])
    before = set(vp.scene.selection)
    action(host, "isolaterows", "i:" + level.uid)
    assert _visible(vp.scene, level) and not _visible(vp.scene, house.children[1])
    assert _visible(vp.scene, tree) == multi
    action(host, "showall", "")
    assert all(_visible(vp.scene, g) for g in (level, house.children[1], tree))
    assert set(vp.scene.selection) == before


def test_row_eye_leaves_other_rows_selected_and_edit_context_closed(model):
    host, vp, house, _, tree = model
    vp.scene.select([tree])
    action(host, "roweye", "i:" + house.uid)
    assert house.hidden and set(vp.scene.selection) == {tree}
    assert vp.scene.edit_group is None


def test_into_and_beside_drops_restore_parents_in_one_undo_each(model):
    host, vp, house, level, tree = model
    action(host, "into", "i:" + tree.uid, "i:" + level.uid)
    assert tree in level.children and tree not in vp.scene.groups
    action(host, "beside", "i:" + tree.uid, "i:" + level.uid)
    assert tree in house.children and tree not in level.children
    assert vp.history.undo() and tree in level.children
    assert vp.history.undo() and vp.scene.groups == [house, tree]


def test_row_menu_and_delete_key_remove_picked_rows_together(model):
    from core.i18n import tr
    host, vp, house, level, tree = model
    vp.scene.select([tree, level])
    assert tr("Delete") in group_rows(host, tree)
    action(host, "deleterows", "i:" + tree.uid)
    assert tree not in vp.scene.groups and level not in house.children
    assert vp.history.undo() and vp.scene.groups == [house, tree] and house.children[0] is level
    assert not vp.history.undo_stack


def test_deleting_parent_and_child_only_deletes_the_parent(model):
    host, vp, house, level, tree = model
    vp.scene.select([house, level])
    action(host, "deleterows", "i:" + level.uid)
    assert vp.scene.groups == [tree]
    assert vp.history.undo() and house.children[0] is level


@pytest.mark.parametrize("start_on_plane", [False, True])
def test_a_mixed_plane_and_occurrence_delete_restores_both_in_order(model, start_on_plane):
    host, vp, house, _, tree = model
    plane, second = _two_planes(vp.scene)
    vp.scene.select([tree, plane])
    action(host, "deleterows", "section:" + str(plane.uid) if start_on_plane else "i:" + tree.uid)
    assert vp.scene.section_planes == [second] and vp.scene.groups == [house]
    assert vp.history.undo() and vp.scene.groups == [house, tree]
    assert vp.scene.section_planes == [plane, second] and plane.active


def test_locked_ancestors_and_locked_descendants_cannot_be_deleted(model):
    host, vp, house, level, tree = model
    house.locked = True
    vp.scene.select([tree, level])
    action(host, "deleterows", "i:" + tree.uid)
    assert vp.scene.groups == [house] and level in house.children
    assert vp.history.undo()
    house.locked = False
    level.children[0].locked = True
    action(host, "deleterows", "i:" + house.uid)
    assert vp.scene.groups == [house, tree] and not vp.history.undo_stack


def test_read_only_and_stale_rows_cannot_delete_the_selection(model):
    host, vp, house, _, tree = model
    vp.scene.select([tree])
    action(host, "deleterows", "i:missing")
    vp.scene.worksession_read_only = True
    action(host, "deleterows", "i:" + tree.uid)
    assert vp.scene.groups == [house, tree] and not vp.history.undo_stack


def test_deleting_a_tag_rechecks_the_document_after_confirmation(native_host):
    from core.layers import Layer
    from views import chrome_dialogs
    from core.group import Group
    win = native_host.window
    scene = win.viewport.scene
    scene.layers.append(Layer("Glass"))
    group = Group(name="Pane")
    group.layer = "Glass"
    scene.groups.append(group)
    action(native_host, "deletetag", "preview-tag:Glass")
    scene.version += 1
    assert chrome_dialogs.answer(win, "delete") and scene.layer("Glass") is not None
    assert group.layer == "Glass"


def test_unchanged_project_rows_reuse_the_model_index(monkeypatch):
    from tests.test_project_drawing_filters import Project
    from unittest.mock import Mock
    from views import wgpu_ui
    project = Project()
    project.scene.groups[:] = [_group("Building"), _group("Tree")]
    project.scene.version += 1
    model = Mock(wraps=wgpu_ui._tree_model)
    monkeypatch.setattr(wgpu_ui, "_tree_model", model)
    first = project.rows()
    assert project.rows() == first and model.call_count == 1
    project.scene.groups.append(_group("Park"))
    project.scene.version += 1
    project.rows()
    assert model.call_count == 2
