"""Capture the Rust chrome from a Qt-owned Metal view using synthetic records."""
import argparse
import os
import sys
from pathlib import Path
import tempfile
import time

from PySide6.QtCore import QSettings, QTimer, Qt
from PySide6.QtGui import QColor, QImage
from PySide6.QtWidgets import QApplication, QWidget
from ingetrazo_render import Renderer


def capture(prefix, directory):
    QSettings.setDefaultFormat(QSettings.IniFormat)
    for scope in (QSettings.UserScope, QSettings.SystemScope):
        QSettings.setPath(QSettings.IniFormat, scope, os.environ["BUILT_SETTINGS_DIR"])
    if prefix == "gallery":
        capture_qt_gallery(directory)
        return
    app = QApplication([])
    window = QWidget()
    window.resize(1280, 800)
    window.setAttribute(Qt.WA_NativeWindow)
    window.show()
    app.processEvents()
    r = Renderer(int(window.winId()), 1280, 800, 1.)
    import numpy as np
    blank = lambda n: np.zeros(n, dtype=np.uint8)
    r.set_scene((blank(28), blank(4), blank(32), blank(40)), blank(80))
    r.ui_enable(str(Path(__file__).resolve().parents[1] / "resources/fonts/inter"))

    def shot(name):
        for _ in range(3):
            r.render(True)
        r.request_capture()
        r.render(True)
        pixels = bytes(r.take_capture())
        assert len(pixels) == 1280 * 800 * 4
        img = QImage(pixels, 1280, 800, QImage.Format_RGBA8888)
        if not img.save(str(directory / f"{name}.png")):
            raise OSError(name)
        return img.copy()

    r.ui_set_start([(["sample", "Courtyard", "Model", "Studio", ""], [1, 0])],
                   [], [("blank", "Blank model", "Start a model")], [], "Built",
                   [("recent", "Recent files")], [])
    r.ui_show_start(True, False)
    shot(f"{prefix}-start")
    r.ui_show_start(False, False)
    # [depth, count, problems, active, row, eye, lock, expandable, expanded, selected, mark]
    r.ui_set_tree([(["n:ride", "Ride.igz", "", "", "file:found"], [0, 48, 0, 0, 0, 1, 0, 1, 0, 0, 0]),
                   (["n:studio", "Studio"], [0, 12, 0, 1, 0, 1, 0, 1, 1, 1, 0]),
                   (["n:walls", "Walls"], [1, 6, 0, 0, 0, 1, 0, 0, 0, 6, 1]),
                   (["n:roof", "Roof"], [1, 4, 0, 0, 0, 1, 1, 0, 0, 0, 0]),
                   (["n:furniture", "Furniture"], [1, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0]),
                   (["n:survey", "Survey.igz", "", "via Ride", "file:missing"],
                    [0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0])], 0)
    r.ui_set_project_tabs([("a", "Courtyard", True, False, False, "", ""),
                           ("b", "Studio", False, True, False, "", "")])
    r.ui_set_inspector("Walls", [("Node", [("Name", "Walls"), ("Kind", "Wall"), ("Items", "6")]),
                                 ("Actions", [("measure", "Measure quantities")]),
                                 ("Export details", [("IFC class", "IfcWall"),
                                                     ("Predefined type", "STANDARD"),
                                                     ("Tag", "W-01"),
                                                     ("Uniclass", "EF_25_10 Walls")])])
    image = shot(f"{prefix}-dock-tree-tabs")
    x, y, w, h = map(round, r.ui_area_rect("column"))
    tabs = [rect for rect in r.ui_project_tab_rects() if rect]
    tx, ty, _, th = tabs[0]
    tab_end = max(left + width for left, _, width, _ in tabs)
    for name, rect in [("dock", (x, y, w, h)), ("tree", (x, y, w, min(h, 260))),
                       ("tabs", (round(tx), round(ty), round(tab_end - tx), round(th)))]:
        if not image.copy(*rect).save(str(directory / f"{prefix}-{name}.png")):
            raise OSError(name)
    # Open "Export details" with a real click: it is the lowest text line in the dock.
    lowest = None
    for row in range(y + h - 8, y + 40, -1):
        if any(QColor(image.pixel(col, row)).lightness() > 75 for col in range(x + 8, x + 140)):
            lowest = row
            break
    if lowest is not None:
        for kind in ("move", "press", "release"):
            r.ui_pointer(kind, x + 40, lowest - 5, 0, 0)
            r.render(True)
        r.ui_pointer("leave", 0, 0, 0, 0)
        for _ in range(6):  # let the disclosure finish opening
            time.sleep(0.05)
            r.render(True)
    shot(f"{prefix}-inspector-full")
    full = QImage(str(directory / f"{prefix}-inspector-full.png"))
    full.copy(x, y, w, h).save(str(directory / f"{prefix}-inspector.png"))
    r.ui_set_library([("materials", "Materials", 1, True)], "materials", 0, "name",
                     [], [], [], "", "", [(["brick", "Brick"], [2, 0, 0, 1])])
    r.ui_set_mode("Library")
    shot(f"{prefix}-library")
    r.ui_set_mode("")
    r.ui_set_jobs([("export", "Export drawings", "Pages", .45, True)])
    shot(f"{prefix}-activity")
    r.ui_set_jobs([])
    r.ui_set_activity([("done", "Drawings exported", "12 pages to Deliveries/Handover", "", "Show", 1.0)], "")
    shot(f"{prefix}-toast")
    r.ui_set_activity([], "")
    r.ui_set_jobs([("export", "Export drawings", "Pages", .45, True)])
    r.ui_set_sheet("Save changes?", "Courtyard has unsaved edits.", "question",
                   [("cancel", "Cancel", "Esc", 0), ("save", "Save", "↵", 1)])
    shot(f"{prefix}-sheet")


def capture_qt_gallery(directory):
    # Exercise the actual Qt entry point with the same flag as an interactive launch.
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    import main
    from views.wgpu_view import WgpuView

    sys.argv[:] = [sys.argv[0], "--gallery"]
    def capture_after_layout(app):
        def readback():
            view = next((v for v in app.allWidgets() if isinstance(v, WgpuView)
                         and v.isVisible() and v.renderer is not None), None)
            if view is None:
                return
            for _ in range(3):
                view.draw_frame()
            view.renderer.request_capture()
            view.draw_frame()
            data = view.renderer.take_capture()
            if data is None:
                return
            scale = view.devicePixelRatioF()
            w, h = int(view.width() * scale), int(view.height() * scale)
            img = QImage(bytes(data), w, h, QImage.Format_RGBA8888)
            app.exit(0 if img.save(str(directory / "gallery-qt.png")) else 1)
        for top in app.topLevelWidgets():
            if top.isWindow() and hasattr(top, "_doc"):
                top.resize(1400, 930)
        timer = QTimer()
        timer.timeout.connect(readback)
        timer.start(600)
        QTimer.singleShot(20000, lambda: app.exit(1))
        return QApplication.exec()

    main._App.exec = capture_after_layout
    if main.main():
        raise RuntimeError("Qt gallery capture failed")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("prefix", choices=["before", "after", "gallery"])
    parser.add_argument("--dest", default="build/design-system",
                        help="output folder, relative to the checkout")
    args = parser.parse_args()
    dest = Path(__file__).resolve().parents[1] / args.dest
    dest.mkdir(parents=True, exist_ok=True)
    plist = Path.home() / "Library/Preferences/com.ingetrazo.IngeTrazo.plist"
    stamp = plist.stat().st_mtime_ns if plist.exists() else None
    with tempfile.TemporaryDirectory() as settings:
        os.environ.update(HOME=settings, CFFIXED_USER_HOME=settings, BUILT_SETTINGS_DIR=settings,
                          XDG_CONFIG_HOME=settings)
        os.environ.pop("BUILT_GALLERY", None)
        capture(args.prefix, dest)
    assert stamp == (plist.stat().st_mtime_ns if plist.exists() else None)
