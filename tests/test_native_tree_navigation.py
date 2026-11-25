# SPDX-License-Identifier: GPL-3.0-or-later
"""Occurrence row actions emitted by the native sidebar."""
import pytest

from built_core import outliner
from tests.native_tree import action
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from tests.test_project_occurrences import _building, _group, _two_planes


@pytest.fixture
def model(native_host):
    scene = native_host.viewport.scene
    building, tree = _building()
    scene.groups[:] = [building, tree]
    scene.version += 1
    return scene, building, tree


def row(group):
    return "i:" + group.uid


def test_clicking_a_nested_row_opens_its_containers_and_selects_it(native_host, model):
    scene, building, tree = model
    level = building.children[0]
    wall = level.children[0]
    action(native_host, "pickrow", row(wall))
    assert outliner.open_path(scene) == [building, level] and scene.selection == {wall}
    action(native_host, "pickrow", row(tree))
    assert scene.edit_group is None and scene.selection == {tree}


def test_moving_to_a_sibling_branch_steps_out_only_where_it_differs(native_host, model):
    scene, building, _ = model
    first, second = building.children
    action(native_host, "pickrow", row(first.children[1]))
    assert outliner.open_path(scene) == [building, first]
    action(native_host, "pickrow", row(second.children[0]))
    assert outliner.open_path(scene) == [building, second] and scene.selection == {second.children[0]}


def test_the_eye_hides_every_selected_row_at_once(native_host, model):
    scene, building, _ = model
    wall, slab = building.children[0].children
    action(native_host, "pickrow", row(wall))
    action(native_host, "toggleitem", row(slab))
    action(native_host, "roweye", row(slab))
    assert wall.hidden and slab.hidden
    assert native_host.viewport.history.undo() and not wall.hidden and not slab.hidden


def test_double_click_opens_and_esc_steps_out_one_level(native_host, model):
    scene, building, _ = model
    level = building.children[0]
    action(native_host, "open", row(level))
    assert outliner.open_path(scene) == [building, level]
    action(native_host, "stepout", row(level))
    assert scene.edit_group is building
    action(native_host, "stepout", row(building))
    assert scene.edit_group is None


def test_the_keyboard_alone_hides_opens_and_steps_out(native_host, model):
    scene, building, tree = model
    action(native_host, "roweye", row(tree))
    assert tree.hidden
    action(native_host, "roweye", row(tree))
    assert not tree.hidden
    action(native_host, "open", row(building))
    assert scene.edit_group is building
    action(native_host, "stepout", row(building))
    assert scene.edit_group is None


@pytest.mark.parametrize("restriction", ["hidden", "locked"])
def test_nothing_opens_through_a_restricted_parent(native_host, model, restriction):
    scene, building, _ = model
    setattr(building, restriction, True)
    for verb, group in [("pickrow", building.children[0].children[0]), ("open", building)]:
        action(native_host, verb, row(group))
        assert scene.edit_group is None and not scene.selection


def test_dropping_rows_on_a_group_is_one_undo_step(native_host, model):
    scene, building, tree = model
    dest = building.children[1]
    history = native_host.viewport.history
    revision = history.revision
    action(native_host, "into", row(tree), row(dest))
    assert tree in dest.children and tree not in scene.groups
    assert row(dest) in native_host.view._ui_exp
    assert history.revision == revision + 1
    assert history.undo() and tree in scene.groups and tree not in dest.children


def test_a_refused_drop_changes_nothing(native_host, model):
    scene, building, _ = model
    child = building.children[0]
    revision = native_host.viewport.history.revision
    action(native_host, "into", row(building), row(child))
    action(native_host, "into", row(child), "i:missing")
    assert native_host.viewport.history.revision == revision
    assert building in scene.groups and child in building.children


def test_tab_and_shift_tab_move_in_and_out(native_host, model):
    scene, _, tree = model
    fence = _group("Fence", (8, 0, 0))
    scene.groups.append(fence)
    action(native_host, "into", row(fence), row(tree))
    assert fence in tree.children
    action(native_host, "outdent", row(fence))
    assert fence in scene.groups and fence not in tree.children


def test_a_drop_reparents_in_one_undo_step(native_host, model):
    scene, building, tree = model
    parent = building.children[1]
    child = parent.children[0]
    history = native_host.viewport.history
    revision = history.revision
    action(native_host, "beside", row(tree), row(child))
    assert tree in parent.children and history.revision == revision + 1
    assert history.undo() and tree in scene.groups


def test_keyboard_on_a_plane_row(native_host, model):
    scene, _, _ = model
    a, b = _two_planes(scene)
    key = f"section:{b.uid}"
    action(native_host, "item", key)
    assert scene.selection == {b}
    action(native_host, "eye", key)
    assert b.hidden
    action(native_host, "sectioncut", key)
    assert b.active and not a.active


def test_f2_renames_the_addressed_plane(native_host, model):
    scene, _, _ = model
    a, b = _two_planes(scene)
    previous = b.name
    action(native_host, "rename", f"section:{b.uid}", "East cut")
    assert b.name == "East cut" and a.name != "East cut"
    assert native_host.viewport.history.undo() and b.name == previous
