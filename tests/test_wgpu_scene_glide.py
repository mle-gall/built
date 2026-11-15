# SPDX-License-Identifier: GPL-3.0-or-later
"""Metal: showing a scene glides, Play runs, and both stop like in GL
(scene transitions 81d868b8, #1342).

Under Metal the GL viewport is hidden and the Metal view draws its camera,
so ``show_view``'s «is the viewport on screen» check made every scene
jump. The player now counts the Metal view as showing the viewport,
repaints it each frame and stops on its clicks, wheel and Esc. The
document's «Scene Transitions» off still jumps. The window is not shown:
visibility is faked as in tests/test_scene_animation.py.
"""
import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import pytest
from PySide6.QtCore import QEvent, QPoint, QPointF, Qt
from PySide6.QtGui import QKeyEvent, QMouseEvent, QWheelEvent
from PySide6.QtWidgets import QApplication

from core.scene_animation import SceneAnimation

_app = QApplication.instance() or QApplication([])


class _Clock:
    def __init__(self):
        self.now = 100.0

    def __call__(self):
        return self.now


@pytest.fixture
def win():
    from views.main_window import MainWindow
    from views.wgpu_view import WgpuView
    w = MainWindow()
    display = WgpuView()
    display.source_viewport = w.viewport
    display.camera = w.viewport.camera
    display.updates = 0
    display.update = lambda: setattr(display, 'updates', display.updates + 1)
    display.isVisible = lambda: True
    w.viewport.isVisible = lambda: False     # hidden under Metal
    w._doc.stand_in = display
    w.scene_player._clock = _Clock()
    yield w, display
    w.scene_player.cancel()
    w._doc.stand_in = None
    display.close()
    for doc in list(w._documents):
        doc.saved_content = doc.viewport.scene.content_version
    w._saved_version = w.viewport.scene.version
    w.close()


def _scenes(win, *specs):
    from core.saved_views import SavedView
    cam = win.viewport.camera
    for name, view in specs:
        cam.set_view(view)
        win.viewport.scene.saved_views.append(
            SavedView.capture(name, win.viewport.scene, cam))
    win.tray.scenes.refresh()
    cam.set_view('iso')


def _esc_action(win):
    from PySide6.QtGui import QAction, QKeySequence
    return next(a for a in win.findChildren(QAction)
                if a.shortcut() == QKeySequence('Esc'))


def test_a_scene_glides_on_the_metal_view(win):
    w, display = win
    _scenes(w, ('Planta', 'top'), ('Alzado', 'front'))
    player = w.scene_player
    w._scene_tabs.tabs.tabBarClicked.emit(0)
    assert player.is_active()
    start = display.camera.pitch
    player._clock.now += 0.5
    player.tick()
    top = type(w.viewport.camera)()
    top.set_view('top')
    # Bit by bit, on the camera the Metal view draws, and repainted there.
    assert display.camera is w.viewport.camera
    assert start < display.camera.pitch < top.pitch
    assert display.updates >= 1
    player._clock.now += 5.0
    player.tick()
    assert not player.is_active()
    assert display.camera.pitch == pytest.approx(top.pitch)


def test_with_transitions_off_it_still_jumps(win):
    w, display = win
    _scenes(w, ('Planta', 'top'))
    w.viewport.scene.animation = SceneAnimation(enabled=False)
    w._scene_tabs.tabs.tabBarClicked.emit(0)
    assert not w.scene_player.is_active()
    top = type(w.viewport.camera)()
    top.set_view('top')
    assert display.camera.pitch == pytest.approx(top.pitch)


def test_hidden_metal_view_jumps_like_a_hidden_viewport(win):
    w, display = win
    display.isVisible = lambda: False
    _scenes(w, ('Planta', 'top'))
    w._scene_tabs.tabs.tabBarClicked.emit(0)
    assert not w.scene_player.is_active()


@pytest.mark.parametrize('kind', ['press', 'double', 'wheel', 'esc'])
def test_the_metal_view_s_input_stops_a_glide(win, kind):
    w, display = win
    _scenes(w, ('Planta', 'top'))
    w._scene_tabs.tabs.tabBarClicked.emit(0)
    assert w.scene_player.is_active()
    p = QPointF(5, 5)
    if kind == 'press':
        ev = QMouseEvent(QEvent.MouseButtonPress, p, p, Qt.LeftButton,
                         Qt.LeftButton, Qt.NoModifier)
    elif kind == 'double':
        ev = QMouseEvent(QEvent.MouseButtonDblClick, p, p, Qt.MiddleButton,
                         Qt.MiddleButton, Qt.NoModifier)
    elif kind == 'wheel':
        ev = QWheelEvent(p, p, QPoint(0, 0), QPoint(0, 120), Qt.NoButton,
                         Qt.NoModifier, Qt.NoScrollPhase, False)
    else:
        ev = QKeyEvent(QEvent.KeyPress, Qt.Key_Escape, Qt.NoModifier)
    # Through Qt's filter chain on the Metal view, not the GL viewport.
    eaten = w.scene_player.eventFilter(display, ev)
    assert not w.scene_player.is_active()
    assert eaten is (kind == 'esc')


def test_the_filter_sits_on_the_metal_view_only_while_moving(win):
    w, display = win
    _scenes(w, ('Planta', 'top'), ('Alzado', 'front'))
    player = w.scene_player
    w._scene_tabs.tabs.tabBarClicked.emit(0)
    assert player._display is display
    QApplication.sendEvent(display, QMouseEvent(
        QEvent.MouseButtonPress, QPointF(5, 5), QPointF(5, 5),
        Qt.LeftButton, Qt.LeftButton, Qt.NoModifier))
    assert not player.is_active()
    assert player._display is None


def test_the_window_s_esc_stops_a_glide_and_play(win):
    w, _display = win
    _scenes(w, ('A', 'top'), ('B', 'front'))
    w._scene_tabs.tabs.tabBarClicked.emit(1)
    assert w.scene_player.is_active()
    _esc_action(w).trigger()
    assert not w.scene_player.is_active()
    w.play_scenes(True)
    assert w.scene_player.is_playing()
    assert w.scene_player._display is _display
    _esc_action(w).trigger()
    assert not w.scene_player.is_active()
    assert not w._act_play_scenes.isChecked()
