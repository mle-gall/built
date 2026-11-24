# SPDX-License-Identifier: GPL-3.0-or-later
"""Remaining Classic scene thumbnail and row-selection tests; shared scene commands."""
from __future__ import annotations

import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from tests.shell_isolation import restore_shell_patches  # noqa: F401
import pytest
from PySide6.QtCore import QSettings, Qt
from PySide6.QtWidgets import QApplication

_inst = QApplication.instance()
if _inst is None:
    _app = QApplication([])
elif not isinstance(_inst, QApplication):
    pytest.skip("a non-widget QGuiApplication is already active",
                allow_module_level=True)
else:
    _app = _inst

import core.extensions as extensions                              # noqa: E402
from core import scene_folders as SF                              # noqa: E402


@pytest.fixture
def win(tmp_path, monkeypatch):
    """A window with a throwaway INI, so no layout leaks in or out. The
    pictures are not drawn here (a render each): the tests that want them
    ask for them."""
    path = tmp_path / "prefs.ini"
    factory = lambda *a: QSettings(str(path), QSettings.IniFormat)  # noqa: E731
    import PySide6.QtCore as qc
    import views.main_window as mw
    from views.tray import ScenesPanel
    monkeypatch.setattr(qc, "QSettings", factory)
    monkeypatch.setattr(mw, "QSettings", factory)
    monkeypatch.setattr(extensions, "plugin_dirs", lambda: [tmp_path])
    monkeypatch.setattr(ScenesPanel, "_render_thumbnail",
                        lambda self, view: None)
    from views.main_window import MainWindow
    w = MainWindow()
    w.resize(1400, 900)
    w.show()
    for _ in range(3):
        _app.processEvents()
    yield w
    w._saved_version = w.viewport.scene.version   # no "save?" modal
    w.close()


def _scenes(win, *names):
    """Saved scenes with these names, through the panel's own button."""
    panel = win.scenes_dock.panel
    for name in names:
        panel.tree.clearSelection()
        view = panel._on_add()
        view.name = name
    panel.tree.clearSelection()
    panel._shape = None
    panel.refresh()
    return panel


def _view(win, name):
    return next(v for v in win.viewport.scene.saved_views if v.name == name)


def _folder(panel, name, parent=None):
    """A folder made with the panel's button, renamed to ``name``."""
    panel.tree.clearSelection()
    if parent is not None:
        panel._select_row(panel.folder_row(parent))
    before = set(SF.folders_of(panel._scene()))
    panel._on_add_folder()
    made = (set(SF.folders_of(panel._scene())) - before).pop()
    assert panel.rename_folder(made, name)
    panel.tree.clearSelection()
    return name


# ---- The list or the thumbnails ---------------------------------------------

def _wait(timeout_ms=5000, until=None):
    """Until the pictures are drawn, one per tick of the loop: a while,
    then on until the Scenes tab has none left to draw nor an old-picture
    timer running (a slow machine needs more than a fixed pause), and
    ``until()`` holds when given (a loaded machine can leave the queue
    empty for a moment before the next picture is put in it)."""
    import time
    from PySide6.QtTest import QTest
    QTest.qWait(300)
    panel = _app_panel()
    deadline = time.monotonic() + timeout_ms / 1000
    while (panel is not None and panel.isVisible()
           and (panel._thumb_queue or panel._old_timer.isActive()
                or (until is not None and not until()))
           and time.monotonic() < deadline):
        QTest.qWait(20)


def _app_panel():
    """The Scenes tab on screen, if there is one (a hidden tab draws
    nothing, so there is nothing to wait for)."""
    from views.tray import ScenesPanel
    from shiboken6 import isValid
    panels = [w for w in _app.allWidgets()
              if isinstance(w, ScenesPanel) and isValid(w)
              and w.isVisible()]
    return panels[-1] if panels else None


def test_each_scene_gets_its_picture_and_a_changed_one_is_drawn_again(
        win, monkeypatch):
    from PySide6.QtGui import QColor, QIcon, QPixmap
    from views.tray import ScenesPanel
    drawn = []

    def fake(self, view):
        drawn.append(view.name)
        pm = QPixmap(96, 64)
        pm.fill(QColor("red"))
        return QIcon(pm)
    monkeypatch.setattr(ScenesPanel, "_render_thumbnail", fake)
    panel = _scenes(win, "A", "B")
    _wait(until=lambda: len(drawn) >= 2)
    assert sorted(drawn) == ["A", "B"]
    row = panel.row_for(_view(win, "A"))
    assert row.icon(0).pixmap(96, 64).toImage().pixelColor(48, 32) == \
        QColor("red")
    drawn.clear()
    panel._shape = None
    panel.refresh()
    _wait()
    assert drawn == []                    # nothing changed: none drawn again
    win.viewport.camera.distance += 10.0
    panel.select_view(_view(win, "B"))
    panel._on_update()
    _wait(until=lambda: bool(drawn))
    assert drawn == ["B"]


def _painter(monkeypatch, drawn, colour):
    """Pictures of one colour (``colour[0]``), each one noted in ``drawn``."""
    from PySide6.QtGui import QColor, QIcon, QPixmap
    from views.tray import ScenesPanel

    def fake(self, view):
        drawn.append(view.name)
        pm = QPixmap(96, 64)
        pm.fill(QColor(colour[0]))
        return QIcon(pm)
    monkeypatch.setattr(ScenesPanel, "_render_thumbnail", fake)


def _colour(panel, view):
    return panel.row_for(view).icon(0).pixmap(96, 64).toImage().pixelColor(
        48, 32).name()


def _edit_model(win):
    """An edit of the model (not of the scenes), as a tool's would be."""
    win.viewport.scene.version += 1
    win.scenes_dock.panel.refresh()


def test_a_model_edited_redraws_the_pictures_once_it_is_still(
        win, monkeypatch):
    """Pictures went old when the model changed: now each is drawn again
    a moment after the last edit, the old one shown until then."""
    drawn, colour = [], ["#ff0000"]
    _painter(monkeypatch, drawn, colour)
    panel = _scenes(win, "A", "B")
    panel._old_timer.setInterval(150)
    _wait()
    assert sorted(drawn) == ["A", "B"]
    drawn.clear()
    colour[0] = "#0000ff"
    _edit_model(win)
    _app.processEvents()
    assert drawn == []                         # not at each stroke
    assert _colour(panel, _view(win, "A")) == "#ff0000"
    _edit_model(win)                           # another: it waits again
    _wait()
    _wait()
    assert sorted(drawn) == ["A", "B"]         # once each, not twice
    assert _colour(panel, _view(win, "A")) == "#0000ff"


def test_edits_of_the_scenes_redraw_only_the_scene_they_change(
        win, monkeypatch):
    """Adding, renaming, filing in folders: the model is the same, the
    other pictures stay."""
    drawn, colour = [], ["#ff0000"]
    _painter(monkeypatch, drawn, colour)
    panel = _scenes(win, "A", "B")
    panel._old_timer.setInterval(50)
    _wait()
    drawn.clear()
    panel.rename(_view(win, "A"), "C")
    _folder(panel, "F")
    panel.drop_rows(("folder", "F"), [panel.row_for(_view(win, "B"))])
    panel.tree.clearSelection()
    panel._on_add()
    _wait()
    _wait()
    assert "B" not in drawn
    assert drawn.count("C") == 1 and len(drawn) == 2   # renamed + new


# ---- With the scene tabs under the viewport (main 0.6.1) -------------------

def test_one_command_of_each_kind_for_scenes():
    """Main's AddSceneCommand (with a place and the folder now) and its
    RenameSceneCommand: no second class of the same name beside them."""
    import core.history as H
    import core.saved_views as S
    import core.history_commands as commands
    assert H.AddSceneCommand is commands.AddSceneCommand
    src = open(commands.__file__, encoding="utf8").read()
    assert src.count("class AddSceneCommand(") == 1
    assert "class RenameSceneCommand(" not in src
    assert hasattr(S, "RenameSceneCommand") and hasattr(S, "MoveSceneCommand")


def test_move_earlier_and_later_stay_within_the_marked_scene_folder(native_host):
    from tests.scene_controls import choose, rows
    from views.wgpu_ui import _run
    win = native_host.window
    scene = win.viewport.scene
    a, b, c = [win.tray.scenes.add_scene() for _ in range(3)]
    SF.add_folder(scene, "Plans", None)
    SF.move_rows(scene, [b, c], [], "Plans")
    scene.version += 1
    assert list(scene.saved_views) == [b, c, a]
    choose(win, 1)
    _run(win, ["tab:scene:up"])
    assert list(scene.saved_views) == [c, b, a]
    steps = len(win.viewport.history.undo_stack)
    _run(win, ["tab:scene:up"])
    assert len(win.viewport.history.undo_stack) == steps
    _run(win, ["tab:scene:down", "tab:scene:down"])
    assert list(scene.saved_views) == [b, c, a]
    assert len(win.viewport.history.undo_stack) == steps + 1
    assert [r[1] for r in rows(win) if r[4] == 1] == [c.name]
    assert SF.folder_of(scene, c) == "Plans"


def test_update_from_the_scene_chooser_is_one_undo_step(native_host):
    win = native_host.window
    win.run_command("scene_add")
    view, = win.viewport.scene.saved_views
    before = view.distance
    win.viewport.camera.distance = before + 25.0
    win.run_command("scene_update")
    assert view.distance == pytest.approx(before + 25.0)
    win.viewport.history.undo()
    assert view.distance == pytest.approx(before)


def test_a_scene_shown_from_the_menu_is_marked_in_the_chooser(native_host):
    from tests.scene_controls import rows
    win = native_host.window
    win.run_command("scene_add")
    win.run_command("scene_add")
    win.run_command("scene_previous")
    win.run_command("scene_show")
    assert [r[4] for r in rows(win)] == [1, 0]


def test_a_refresh_keeps_what_the_panel_has_picked(win):
    panel = _scenes(win, "A", "B", "C")
    first, _, last = panel.scene_rows()
    panel.tree.setCurrentItem(first)
    first.setSelected(True)
    last.setSelected(True)
    panel._shape = None
    panel.refresh()
    assert sorted(v.name for v in panel.selected_views()) == ["A", "C"]


def test_insert_scene_clamps_large_integer_positions():
    from core.history import AddSceneCommand
    from core.saved_views import SavedView
    from core.scene import Scene

    scene = Scene()
    centre, first, last = [SavedView(name) for name in ("Centre", "First", "Last")]
    scene.saved_views.append(centre)
    AddSceneCommand(first, -(2**100)).do(scene)
    AddSceneCommand(last, 2**100).do(scene)
    assert list(scene.saved_views) == [first, centre, last]
