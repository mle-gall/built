# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Linked documents in the real window: link one, its tags in the Tags
panel's folders, saving the source in another tab reloads the host, and
going into the linked copy offers to open its file."""
from __future__ import annotations

import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import pytest  # noqa: E402
from PySide6.QtCore import QEventLoop, QTimer  # noqa: E402
from ingetrazo_core import Vec3 as V
from PySide6.QtWidgets import QApplication, QMessageBox  # noqa: E402

from core import links  # noqa: E402
from formats import igz  # noqa: E402

_app = QApplication.instance() or QApplication([])


def _settle(ms: int = 0) -> None:
    loop = QEventLoop()
    QTimer.singleShot(ms, loop.quit)
    loop.exec()


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
def win():
    from views.main_window import MainWindow
    w = MainWindow()
    yield w
    for d in w.documents():
        d.saved_content = d.viewport.scene.content_version
    w.close()


def _faces(g):
    from core.group import iter_placements
    return sum(len(p.mesh.faces) for p, _m in iter_placements(g))


def test_link_place_and_reload_when_the_source_changes(win, tmp_path):
    src = _source(tmp_path / "Site.igz", faces=1)
    win.link_igz_path(src, at=(0, 0, 0))
    scene = win.viewport.scene
    (g,) = links.linked_groups(scene)
    assert _faces(g) == 1
    assert scene.layer("Site / Wood") is not None
    assert str(src.resolve()) in win._link_watcher()._fs.files()
    # The source changes on disk (another tab's save, another app).
    _source(src, faces=3)
    win._link_watcher()._changed(str(src.resolve()))
    _settle(win._link_watcher().SETTLE_MS * 4)   # the OS event restarts it
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
    monkeypatch.setattr("views.chrome_dialogs.ask", answer)
    win.viewport.begin_group_edit(g)
    assert "Site.igz" in asked["text"]
    assert win.viewport.scene.edit_group is None        # never entered
    assert any(d.same_file(src) for d in win.documents())


def test_save_as_placeholder_and_reopen(win, tmp_path):
    src = _source(tmp_path / "Site.igz", faces=2)
    win.link_igz_path(src, at=(0, 0, 0))
    host = tmp_path / "Host.igz"
    win._do_save(host)
    win.open_path(host)
    _settle()
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
    assert links.is_packed(win.viewport.scene, path)
    assert {'query_links', 'link_document', 'reload_link', 'query_project', 'set_project'} <= {t['name'] for t in BUILTIN_TOOLS}


@pytest.mark.parametrize("choice", [True, False, None])
def test_link_document_action_shows_nested_choice(win, tmp_path, monkeypatch, choice):
    from PySide6.QtWidgets import QCheckBox, QFileDialog
    from core.i18n import tr
    from views import filedialogs

    path = _source(tmp_path / "building.igz")
    placements = []
    monkeypatch.setattr(win, "link_igz_path", lambda path, include_nested:
                        placements.append((path, include_nested)))
    monkeypatch.setattr(filedialogs, "remember", lambda path: None)

    def choose(dialog):
        assert dialog.testOption(QFileDialog.Option.DontUseNativeDialog)
        checkbox = next(box for box in dialog.findChildren(QCheckBox)
                        if box.text() == tr("Include nested linked files"))
        assert checkbox.isChecked()
        if choice is None:
            return QFileDialog.DialogCode.Rejected
        checkbox.setChecked(choice)
        dialog.selectFile(str(path))
        return QFileDialog.DialogCode.Accepted

    monkeypatch.setattr(QFileDialog, "exec", choose)
    win._act_link_document.trigger()
    assert placements == ([] if choice is None else [(str(path), choice)])


def test_a_reloaded_link_keeps_the_layers_rows(win, tmp_path):
    """A linked file saved elsewhere reloads without tearing the Layers
    tree down: same row objects, folders left folded as they were. In
    the composite (hundreds of linked folders) the rebuild was a blink
    of the whole tree on every save of a source."""
    from core import layers as L
    src = _source(tmp_path / "Site.igz", faces=1)
    win.link_igz_path(src, at=(0, 0, 0))
    scene = win.viewport.scene
    panel = win.layers_dock.panel
    panel.refresh()
    folder = next(f for f in L.folders_of(scene) if f.startswith("Site"))
    panel.item_for(folder, "folder").setExpanded(False)
    _settle()
    rows = {id(it) for it in panel._all_items()}
    rebuilt = []
    original = type(panel).refresh
    panel.refresh = lambda *a, **k: (rebuilt.append(1), original(panel, *a, **k))
    _source(src, faces=3)                     # same tags, more geometry
    win._link_watcher()._changed(str(src.resolve()))
    _settle(win._link_watcher().SETTLE_MS * 4)
    (g,) = links.linked_groups(scene)
    assert _faces(g) == 3
    assert not rebuilt
    assert {id(it) for it in panel._all_items()} == rows
    assert not panel.item_for(folder, "folder").isExpanded()


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
    monkeypatch.setattr("views.chrome_forms.ask_text", lambda *args, done, **kw: done(json.dumps(subsets[1])))
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
