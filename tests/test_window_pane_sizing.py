"""Screen bounds, native dock insets, and coalesced Metal resizing."""
import sys
from unittest.mock import Mock

import pytest
from PySide6.QtCore import QRect, QSize, Qt
from PySide6.QtGui import QResizeEvent
from PySide6.QtTest import QTest
from PySide6.QtWidgets import QApplication, QDockWidget, QScrollArea, QWidget

from views.main_window import MainWindow


def _wait_for(cond, timeout_ms=3000):
    """Under a loaded suite an 80 ms timer can fire late: wait for it."""
    import time
    end = time.monotonic() + timeout_ms / 1000
    while not cond():
        if time.monotonic() > end:
            return False
        QTest.qWait(10)
    return True


@pytest.fixture
def win(monkeypatch):
    window = MainWindow()
    screen = Mock(availableGeometry=lambda: QRect(0, 0, 1600, 1000))
    monkeypatch.setattr(window, "screen", lambda: screen)
    window.resize(1400, 900)
    window.show()
    QApplication.processEvents()
    for _ in range(5):
        QApplication.processEvents()
    yield window
    window.hide()
    window.deleteLater()


def test_restored_oversize_geometry_is_clamped(win):
    avail = win.screen().availableGeometry()
    win.resize(avail.width() + 200, avail.height() + 300)
    win.move(avail.right() - 50, avail.bottom() - 50)
    win._clamp_to_screen()           # what restoreGeometry is followed by
    for _ in range(3):
        QApplication.processEvents()
    assert avail.contains(win.frameGeometry())


@pytest.mark.skipif(sys.platform != 'darwin', reason='macOS native safe areas')
def test_redocked_native_tab_does_not_add_safe_area_to_minimum(win):
    dock = win.layers_dock
    dock.setContentsMargins(1040, 1596, 0, 0)
    win._keep_docked(dock)
    QApplication.processEvents()
    assert dock.contentsMargins().isNull()
    assert not dock.testAttribute(Qt.WA_ContentsMarginsRespectsSafeArea)


def test_wide_extension_scrolls_without_changing_its_contents(win):
    dock = QDockWidget('Wide form', win)
    panel = QWidget()
    panel.setMinimumWidth(900)
    dock.setWidget(panel)
    win.addDockWidget(Qt.RightDockWidgetArea, dock)
    win.tabifyDockWidget(win.tray, dock)
    dock.show()
    win._fit_docks_to_screen()
    QApplication.processEvents()
    assert isinstance(dock.widget(), QScrollArea)
    assert dock.widget().widget() is panel
    dock.raise_()
    for _ in range(3):
        QApplication.processEvents()
    win.resizeDocks([dock], [300], Qt.Horizontal)
    QApplication.processEvents()
    assert dock.width() <= 320
    assert panel.width() >= 900


def test_metal_resize_burst_configures_only_final_size():
    from views.wgpu_view import WgpuView
    view = WgpuView()
    view.renderer = Mock()
    try:
        for width in range(300, 360):
            view.resize(width, 200)
            view.resizeEvent(QResizeEvent(QSize(width, 200), QSize(width - 1, 200)))
        view.renderer.resize.assert_not_called()
        assert view.draw_frame() is None
        assert _wait_for(lambda: view.renderer.resize.called)
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
    view.draw_frame = Mock()
    try:
        view.resize(400, 300)
        view.resizeEvent(QResizeEvent(QSize(400, 300), QSize(300, 200)))
        view._prepare_pick_frame()
        assert not view._resize_timer.isActive()
        view.renderer.resize.assert_called_once_with(400, 300, view.devicePixelRatioF())
        view.draw_frame.assert_called_once()
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
        assert _wait_for(lambda: not tree._resize_paint_timer.isActive())
        assert tree.updatesEnabled() == enabled
        assert not tree._resize_paint_timer.isActive()
    finally:
        tree.hide()
        tree.deleteLater()
        panel.deleteLater()
