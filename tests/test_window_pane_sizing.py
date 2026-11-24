"""Coalesced Metal resizing."""
import sys
from unittest.mock import Mock

import pytest
from PySide6.QtCore import QSize
from PySide6.QtGui import QResizeEvent
from PySide6.QtWidgets import QApplication, QWidget


def test_metal_resize_burst_configures_only_final_size():
    from views.wgpu_view import WgpuView
    view = WgpuView()
    view.renderer = Mock()
    try:
        for width in range(300, 360):
            view.resize(width, 200)
            view.resizeEvent(QResizeEvent(QSize(width, 200), QSize(width - 1, 200)))
        view.renderer.resize.assert_not_called()
        # A repaint during the drag keeps the last frame: nothing drawn.
        view.draw_frame = Mock(wraps=view.draw_frame)
        view.paintEvent(None)
        view.draw_frame.assert_not_called()
        del view.draw_frame
        # One debounce armed for the whole burst; what it runs when it fires
        # configures the final size once. (Fired here rather than waited
        # for: on a loaded machine under pytest-xdist the 80 ms timer came
        # later than any wall-clock wait.)
        assert view._resize_timer.isActive() and view._resize_timer.isSingleShot()
        view._resize_timer.stop()
        view._resize_timer.timeout.emit()
        view.renderer.resize.assert_called_once_with(359, 200, view.devicePixelRatioF())
        assert view._pick_frame_dirty
        view._finish_resize()
        assert view.renderer.resize.call_count == 1
    finally:
        view.renderer = None
        view.deleteLater()


def test_picking_flushes_pending_resize_before_reading_ids():
    from views.wgpu_view import WgpuView
    view = WgpuView()
    view.renderer = Mock()
    view.sync_edit_context = Mock()
    view._present_frame = Mock()
    try:
        view.resize(400, 300)
        view.resizeEvent(QResizeEvent(QSize(400, 300), QSize(300, 200)))
        view._prepare_pick_frame()
        assert not view._resize_timer.isActive()
        view.renderer.resize.assert_called_once_with(400, 300, view.devicePixelRatioF())
        view._present_frame.assert_called_once_with(pick_only=True)
    finally:
        view.renderer = None
        view.deleteLater()


@pytest.mark.parametrize('enabled', [True, False])
def test_mac_tree_repaints_once_resize_settles(monkeypatch, enabled):
    from views.side_tree import SideTree
    panel = QWidget()
    tree = SideTree(panel)
    tree.show()
    QApplication.processEvents()
    monkeypatch.setattr(QApplication, 'platformName', lambda: 'cocoa')
    tree.setUpdatesEnabled(enabled)
    try:
        for width in range(300, 360):
            tree.resizeEvent(QResizeEvent(QSize(width, 200), QSize(width - 1, 200)))
        assert not tree.updatesEnabled()
        # The settle timer, fired rather than waited for (a wall-clock wait
        # through the event loop took 127 s once under pytest-xdist).
        assert tree._resize_paint_timer.isActive() and tree._resize_paint_timer.isSingleShot()
        tree._resize_paint_timer.stop()
        tree._resize_paint_timer.timeout.emit()
        assert tree.updatesEnabled() == enabled
        assert not tree._resize_paint_timer.isActive()
    finally:
        tree.hide()
        tree.deleteLater()
        panel.deleteLater()


def test_an_explicit_draw_applies_the_pending_resize_first():
    """A pick, the view grid or an export draws now, at the new size;
    only the repaints of a live drag wait for the resize to settle."""
    from views.wgpu_view import WgpuView
    view = WgpuView()
    view.renderer = Mock()
    view._present_frame = Mock(return_value="frame")
    try:
        view.resize(420, 240)
        view.resizeEvent(QResizeEvent(QSize(420, 240), QSize(400, 240)))
        assert view._resize_timer.isActive()
        assert view.draw_frame() == "frame"
        assert not view._resize_timer.isActive()
        view.renderer.resize.assert_called_once_with(420, 240, view.devicePixelRatioF())
    finally:
        view.renderer = None
        view.deleteLater()
