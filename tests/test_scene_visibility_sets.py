# SPDX-License-Identifier: GPL-3.0-or-later
"""Saved visibility sets in the Scenes menu act on the live Project eyes."""
import built_core
import pytest

from built_core import layers
from built_core.group import Group
from built_core.history import History
from built_core.protocol.commands import run
from built_core.scene import Scene
from formats import igz
from built_core import visibility_set_rows


def command(history, verb, **kwargs):
    return run(history, "edit_visibility_set", verb=verb, **kwargs)["ok"]


def set_id(scene):
    row, = visibility_set_rows(scene)
    return row[0].removeprefix("visibility:apply:")


def test_apply_restores_across_branches_after_reopening_in_one_undo(tmp_path):
    scene = Scene()
    history = History(scene)
    west, east = Group(name="West"), Group(name="East")
    for parent in (west, east):
        child = Group(name="Joist", uid="same-source-part")
        parent.children.append(child)
        scene.groups.append(parent)
    west.children[0].hidden = True
    folder = built_core.tag_edits.add_folder(scene, "Plans")
    scene.folder_visibility[folder] = False
    scene.layers.append(layers.Layer("Walls", visible=False))
    assert command(history, "save", name="Print")
    key = set_id(scene)
    path = tmp_path / "branches.igz"
    igz.save_scene(scene, path)
    loaded = Scene()
    igz.load_into(loaded, path)
    history = History(loaded)
    west, east = loaded.groups
    west.children[0].hidden = False
    east.children[0].hidden = True
    loaded.folder_visibility[folder] = True
    loaded.layer("Walls").visible = True
    loaded.layers.append(layers.Layer("New work", visible=False))
    assert command(history, "apply", key=key)
    assert west.children[0].hidden and not east.children[0].hidden
    assert not loaded.folder_visibility[folder] and not loaded.layer("Walls").visible
    assert not loaded.layer("New work").visible
    assert len(history.undo_stack) == 1
    history.undo()
    assert not west.children[0].hidden and east.children[0].hidden
    assert loaded.folder_visibility[folder] and loaded.layer("Walls").visible
    history.redo()
    assert west.children[0].hidden and not east.children[0].hidden
    assert not command(history, "apply", key=key)


def test_saved_set_edits_and_noops():
    scene = Scene()
    history = History(scene)
    scene.layers.append(layers.Layer("Walls"))
    assert command(history, "save", name="Print")
    key = set_id(scene)
    assert not command(history, "update", key=key)
    assert not command(history, "rename", key=key, name="Print")
    with pytest.raises(ValueError, match="already"):
        command(history, "save", name="Print")
    assert len(history.undo_stack) == 1
    scene.layer("Walls").visible = False
    assert command(history, "update", key=key)
    assert command(history, "rename", key=key, name="Detail")
    assert visibility_set_rows(scene)[0][1] == "Detail"
    assert command(history, "delete", key=key)
    assert not command(history, "apply", key=key)
    assert visibility_set_rows(scene) == []
    history.undo()
    assert set_id(scene) == key
    scene.layer("Walls").visible = True
    assert command(history, "apply", key=key)
    assert not scene.layer("Walls").visible


def test_linked_tag_uses_a_host_override_and_leaves_the_source_alone(tmp_path):
    from built_core import links
    from tests.test_links import _source, _host_with_link

    path = _source(tmp_path / "Source.igz")
    original = path.read_bytes()
    scene, root = _host_with_link(tmp_path, path)
    tag = scene.layer("Source / Wood")
    history = History(scene)
    tag.visible = False
    assert command(history, "save", name="Coordination")
    tag.visible = True
    assert command(history, "apply", key=set_id(scene))
    assert not tag.visible
    assert scene.link_registry[links.link_of(root)["path"]]["overrides"]
    history.undo()
    assert tag.visible and path.read_bytes() == original


def test_scene_menu_creates_the_first_set_and_confirms_only_in_its_document():
    from types import SimpleNamespace
    from views import wgpu_ui

    scene = Scene()
    prompts = []
    view = SimpleNamespace(renderer=SimpleNamespace(ui_set_prompt=lambda *a: prompts.append(a)),
                           update=lambda: None)
    vp = SimpleNamespace(scene=scene, history=History(scene), notify_scene_changed=lambda: None)
    win = SimpleNamespace(viewport=vp, _doc=SimpleNamespace(stand_in=view))
    wgpu_ui._run(win, ["tab:visibility:save"])
    assert prompts[-1][0] == "Save visibility set"
    wgpu_ui._tree_action(win, view, wgpu_ui.SEP.join(["prompt", "yes", "Print"]))
    assert visibility_set_rows(scene)[0][1] == "Print"
    wgpu_ui._run(win, ["tab:visibility:delete:" + set_id(scene)])
    vp.scene = Scene()
    vp.history = History(vp.scene)
    wgpu_ui._tree_action(win, view, wgpu_ui.SEP.join(["prompt", "yes"]))
    assert len(visibility_set_rows(scene)) == 1 and not vp.history.undo_stack
