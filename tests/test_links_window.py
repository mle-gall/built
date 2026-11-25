# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Linked documents in the real window: link one, its tags in the Tags
panel's folders, saving the source in another tab reloads the host, and
going into the linked copy offers to open its file."""
from __future__ import annotations

import built_core

from pathlib import Path
from tests.shell_isolation import native_window, restore_shell_patches  # noqa: F401

import pytest
from built_core import Vec3 as V

from core import links
from formats import igz


def _source(path, faces=1):
    from core.layers import Layer
    from core.scene import Scene
    s = Scene()
    s.layers.append(Layer("Wood"))
    for i in range(faces):
        f = s.mesh.add_face([V(i, 0, 0), V(i + 1, 0, 0), V(i + 1, 1, 0),
                             V(i, 1, 0)])
        f.attrs = {"layer": "Wood"}
    igz.save_scene(s, path)
    return path


@pytest.fixture
def win(native_host):
    return native_host.window


def _faces(g):
    from core.group import iter_placements
    return sum(len(p.mesh.faces) for p, _m in iter_placements(g))


def _wait_for_reload(win, scene):
    import time
    watcher = win._link_watcher()
    deadline = time.monotonic() + 10
    while _faces(links.linked_groups(scene)[0]) != 3 and time.monotonic() < deadline:
        delay = watcher.poll()
        assert delay is not None
        time.sleep(min(delay / 1000, 0.05))


def test_link_place_and_reload_when_the_source_changes(native_window, tmp_path):
    win = native_window
    src = _source(tmp_path / "Site.igz", faces=1)
    win.link_igz_path(src, at=(0, 0, 0))
    scene = win.viewport.scene
    (g,) = links.linked_groups(scene)
    assert _faces(g) == 1
    assert scene.layer("Site / Wood") is not None
    assert src.resolve() in win._link_watcher().paths
    # The source changes on disk (another tab's save, another app).
    _source(src, faces=3)
    _wait_for_reload(win, scene)
    (g,) = links.linked_groups(scene)
    assert _faces(g) == 3


def test_going_into_a_linked_copy_offers_its_file(win, tmp_path, monkeypatch):
    src = _source(tmp_path / "Site.igz")
    win.link_igz_path(src, at=(0, 0, 0))
    (g,) = links.linked_groups(win.viewport.scene)
    asked = {}

    def answer(parent, title, text, buttons, **kw):
        asked["text"] = title
        return "open"
    monkeypatch.setattr("built_render.chrome_dialogs.ask", answer)
    win.viewport.begin_group_edit(g)
    assert "Site.igz" in asked["text"]
    assert win.viewport.scene.edit_group is None        # never entered
    assert any(d.same_file(src) for d in win.documents())


def test_save_as_placeholder_and_reopen(win, tmp_path):
    src = _source(tmp_path / "Site.igz", faces=2)
    win.link_igz_path(src, at=(0, 0, 0))
    host = tmp_path / "Host.built"
    assert win.run_command(f"save_to:{host}")
    assert win.close_document(ask=False)
    assert win.open_path(host)
    win._host.idle()
    (g,) = links.linked_groups(win.viewport.scene)
    assert _faces(g) == 2


def test_project_link_subset_and_agent_surface(win, tmp_path):
    from core.group import Group
    from core.scene import Scene
    from core.agent_tools import BUILTIN_TOOLS
    scene = Scene()
    scene.groups = [Group(name='A'), Group(name='B')]
    path = tmp_path / 'parts.igz'
    igz.save_scene(scene, path)
    win.link_igz_path(path, at=(0, 0, 0), select={'uids': [scene.groups[0].uid]})
    from tests.test_tree_links import _View
    from views import wgpu_ui
    view = _View()
    wgpu_ui._tree(view, win)
    key = next(k for k, n in view._ui_tree["nodes"].items() if n["kind"] == "link")
    def inspector():
        wgpu_ui._tree(view, win)
        _, sections = wgpu_ui._linked_inspector(view, win, view._ui_tree, key)
        return dict(dict(sections)["Linked file"])
    assert inspector()["Subset"] == "1 item" and inspector()["Stable UIDs"] == "1"
    assert view._ui_tree["nodes"][key]["path"] == str(path)
    wgpu_ui._tree_action(win, view, "linkpack" + wgpu_ui.SEP + "n:" + key)
    assert built_core.link_is_packed(win.viewport.scene, path)
    assert {'query_links', 'link_document', 'reload_link', 'query_project', 'set_project'} <= {t['name'] for t in BUILTIN_TOOLS}


@pytest.mark.parametrize("choice", [True, False, None])
def test_link_document_action_shows_nested_choice(win, tmp_path, monkeypatch, choice):
    from built_core.i18n import tr
    from built_render import chrome_forms
    from built_render.shell_files import ShellFileDialog

    path = _source(tmp_path / "building.igz")
    placements = []
    monkeypatch.setattr(ShellFileDialog, "getOpenFileName", lambda *args: (str(path), ""))
    monkeypatch.setattr(win, "link_igz_path", lambda path, **opts: placements.append((path, opts)))
    def choose(parent, title, label, items, done):
        assert items == [tr("Include nested linked files"), tr("Source geometry only")]
        done(None if choice is None else items[0 if choice else 1])
    monkeypatch.setattr(chrome_forms, "ask_choice", choose)
    assert win.run_command("link")
    assert placements == ([] if choice is None else [(str(path), {"select": None, "include_nested": choice})])


def test_a_reloaded_link_keeps_the_layers_rows(win, tmp_path):
    """A linked file saved elsewhere reloads without tearing the Layers
    tree down: same row objects, folders left folded as they were. In
    the composite (hundreds of linked folders) the rebuild was a blink
    of the whole tree on every save of a source."""
    src = _source(tmp_path / "Site.igz", faces=1)
    win.link_igz_path(src, at=(0, 0, 0))
    scene = win.viewport.scene
    from tests.test_project_drawing_filters import Project
    project = Project(scene=scene)
    project.win = win
    project.view._ui_lens = 1
    key = "n:link:Site"
    rows = project.rows()
    assert key in rows
    opened = project.action("toggle", key)
    assert any("Wood" in name for name, _ in opened.values())
    rows = project.action("toggle", key)
    assert not any("Wood" in name for name, _ in rows.values())
    _source(src, faces=3)
    _wait_for_reload(win, scene)
    assert _faces(links.linked_groups(scene)[0]) == 3
    assert project.rows().keys() == rows.keys()
    assert project.action("toggle", key).keys() == opened.keys()


def test_project_edits_only_the_addressed_link_subset_and_undoes_without_source(win, tmp_path, monkeypatch):
    import json
    from core.group import Group
    from core.scene import Scene
    from core.protocol import commands
    from tests.test_tree_links import _View
    from views import wgpu_ui as ui
    src = Scene()
    src.groups = [Group(name="Door"), Group(name="Window")]
    path = tmp_path / "library.igz"
    igz.save_scene(src, path)
    subsets = [{"uids": [g.uid]} for g in src.groups]
    for i in range(2):
        win.link_igz_path(path, at=(i, 0, 0), select=subsets[0])
    scene = win.viewport.scene
    first, second = links.linked_groups(scene)
    old_first, old_second = first.children[0], second.children[0]
    view = _View()
    ui._tree(view, win)
    monkeypatch.setattr("built_render.chrome_forms.ask_text", lambda *args, done, **kw: done(json.dumps(subsets[1])))
    ui._link_action(win, view, second, "linksubset")
    assert first.children == [old_first] and first.children[0] is old_first
    assert second.children[0].name == "Window" and links.link_of(second)["select"] == subsets[1]
    updated = second.children[0]
    path.unlink()  # undo and redo replay the captured definition
    history = win.viewport.history
    assert history.undo() and second.children[0] is old_second and links.link_of(second)["select"] == subsets[0]
    assert history.redo() and second.children[0] is updated and links.link_of(second)["select"] == subsets[1]
    assert first.children[0] is old_first
    before = history.revision
    with pytest.raises(ValueError):
        commands.run(history, "edit_link_subset", placement=scene.occurrence_path(second), text='{"bad": true}')
    assert history.revision == before and second.children[0] is updated


@pytest.mark.parametrize("nested", [True, False, None])
def test_link_selection_header_preserves_occurrence_and_nested_choice(win, tmp_path, monkeypatch, nested):
    from types import SimpleNamespace
    from core.group import Group
    from core.scene import Scene
    from tests.test_tree_links import _View
    from views import wgpu_ui
    from built_render import chrome_forms

    source = Scene()
    g = Group(name="Panel")
    source.groups.append(g)
    source.select([g])
    source_path = tmp_path / "Parts.igz"
    igz.save_scene(source, source_path)
    source_doc = SimpleNamespace(path=source_path, viewport=SimpleNamespace(scene=source))
    docs = win.documents()
    monkeypatch.setattr(win, "documents", lambda: [*docs, source_doc])
    opened = []
    def choose(parent, title, label, items, done):
        if items == [str(source_path)]:
            done(items[0])
        else:
            done(None if nested is None else items[0 if nested else 1])
    monkeypatch.setattr(chrome_forms, "ask_choice", choose)
    monkeypatch.setattr(win, "link_igz_path", lambda path, **kw: opened.append((path, kw)))
    wgpu_ui._tree_action(win, _View(), "linkselection" + wgpu_ui.SEP)
    assert opened == ([] if nested is None else [
        (str(source_path), {"select": {"uids": [source.occurrence_path(g)]}, "include_nested": nested})])


def test_project_header_moves_the_registry_next_to_the_host(win, tmp_path, monkeypatch):
    from core import bim_registry
    from built_core import project
    from formats import project as manifest
    from tests.test_tree_links import _View
    from views import wgpu_ui
    from built_render import shell_files

    path = tmp_path / "Design.builtproject"
    monkeypatch.setattr(shell_files.ShellFileDialog, "getSaveFileName", lambda *args: (str(path), ""))
    wgpu_ui._tree_action(win, _View(), "linkregistry" + wgpu_ui.SEP)
    assert path.is_file() and Path(project.path_for(win.viewport.scene)) == path
    assert manifest.load(path)["bim_registry"] == bim_registry.get(win.viewport.scene)
