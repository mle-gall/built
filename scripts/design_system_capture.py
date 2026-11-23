"""Capture the Rust chrome from a Qt-owned Metal view using synthetic records."""
import argparse
import os
from pathlib import Path
import tempfile

from PySide6.QtCore import QSettings, Qt
from PySide6.QtGui import QImage
from PySide6.QtWidgets import QApplication, QWidget
from ingetrazo_render import Renderer


def capture(prefix, directory):
    QSettings.setDefaultFormat(QSettings.IniFormat)
    for scope in (QSettings.UserScope, QSettings.SystemScope):
        QSettings.setPath(QSettings.IniFormat, scope, os.environ["BUILT_SETTINGS_DIR"])
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

    if prefix == "gallery":
        shot("gallery-qt")
        return
    r.ui_set_start([(["sample", "Courtyard", "Model", "Studio", ""], [1, 0])],
                   [], [("blank", "Blank model", "Start a model")], [], "Built",
                   [("recent", "Recent files")], [])
    r.ui_show_start(True, False)
    shot(f"{prefix}-start")
    r.ui_show_start(False, False)
    r.ui_set_tree([(["studio", "Studio"], [0, 3, 0, 1, 0, 1, 0, 1, 1]),
                   (["walls", "Walls"], [1, 2]), (["roof", "Roof"], [1, 1])], 0)
    r.ui_set_project_tabs([("a", "Courtyard", True, False, False, "", ""),
                           ("b", "Studio", False, True, False, "", "")])
    r.ui_set_inspector("Studio", [("Properties", [("Name", "Studio")])])
    shot(f"{prefix}-dock-tree-tabs")
    r.ui_set_library([("materials", "Materials", 1, True)], "materials", 0, "name",
                     [], [], [], "", "", [(["brick", "Brick"], [2, 0, 0, 1])])
    r.ui_set_mode("Library")
    shot(f"{prefix}-library")
    r.ui_set_mode("Project")
    r.ui_set_jobs([("export", "Export drawings", "Pages", .45, True)])
    shot(f"{prefix}-activity")
    r.ui_set_sheet("Save changes?", "Courtyard has unsaved edits.", "question",
                   [("cancel", "Cancel", "Esc", 0), ("save", "Save", "↵", 1)])
    shot(f"{prefix}-sheet")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("prefix", choices=["before", "after", "gallery"])
    args = parser.parse_args()
    dest = Path(__file__).resolve().parents[1] / "build/design-system"
    dest.mkdir(parents=True, exist_ok=True)
    plist = Path.home() / "Library/Preferences/com.ingetrazo.IngeTrazo.plist"
    stamp = plist.stat().st_mtime_ns if plist.exists() else None
    with tempfile.TemporaryDirectory() as settings:
        os.environ.update(HOME=settings, CFFIXED_USER_HOME=settings, BUILT_SETTINGS_DIR=settings,
                          XDG_CONFIG_HOME=settings, BUILT_GALLERY=str(int(args.prefix == "gallery")))
        capture(args.prefix, dest)
    assert stamp == (plist.stat().st_mtime_ns if plist.exists() else None)
