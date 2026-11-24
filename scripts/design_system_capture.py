"""Capture the Rust chrome from a Qt-owned Metal view using synthetic records."""
import argparse
import json
import os
from pathlib import Path
from isolated_launch import isolate_script
import time

from PySide6.QtCore import QSettings, Qt
from PySide6.QtGui import QColor, QImage
from PySide6.QtWidgets import QApplication, QWidget
from ingetrazo_render import Renderer


def capture(prefix, directory):
    QSettings.setDefaultFormat(QSettings.IniFormat)
    for scope in (QSettings.UserScope, QSettings.SystemScope):
        QSettings.setPath(QSettings.IniFormat, scope, os.environ["BUILT_SETTINGS_DIR"])
    if prefix == "gallery":
        capture_gallery(directory)
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
        # Let fades and openings settle (MOTION_FADE is 200 ms).
        for _ in range(8):
            r.render(True)
            time.sleep(0.05)
        r.request_capture()
        r.render(True)
        pixels = bytes(r.take_capture())
        assert len(pixels) == 1280 * 800 * 4
        img = QImage(pixels, 1280, 800, QImage.Format_RGBA8888)
        if not img.save(str(directory / f"{name}.png")):
            raise OSError(name)
        return img.copy()

    r.ui_set_large_sheet(json.dumps(start_sheet()))
    shot(f"{prefix}-start")
    r.ui_set_large_sheet("")
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
    for name, rect in [("dock", (x, y, w, h)), ("tree", (x, y, w, min(h, 280))),
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
    r.ui_set_large_sheet(json.dumps(library_sheet()))
    shot(f"{prefix}-library")
    from types import SimpleNamespace
    from ingetrazo_render.settings_sheet import SettingsSheet
    settings = SettingsSheet("navigation").build(SimpleNamespace(win=None))
    r.ui_set_large_sheet(json.dumps(settings.wire()))
    shot(f"{prefix}-settings")
    r.ui_set_large_sheet("")
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


sys.path.insert(0, str(Path(__file__).resolve().parents[1]))


def start_sheet():
    """A start page with recent files, link states and a detail."""
    from ingetrazo_render.chrome_sheet import Sheet, item
    sh = Sheet("start", "Start")
    sh.verb("new", "New model", key="⌘N", primary=True)
    sh.verb("open", "Open…", key="⌘O")
    sh.search("Filter files")
    sh.side_caption("Files")
    sh.side_item("recent", "Recent", right="3", on=True)
    sh.side_item("pinned", "Pinned", right="1")
    sh.side_caption("Start from")
    sh.side_item("blank", "Blank model")
    c = sh.content("Recent", sub="3 files")
    c.tiles([item("courtyard", "Courtyard", sub="Edited today", hex="#3A3F45", dot="ok", dot_text="2 links"),
             item("studio", "Studio", sub="Yesterday", hex="#3A3F45", dot="warn", dot_text="1 changed"),
             item("ride", "Ride", sub="12 Nov", hex="#3A3F45", dot="error", dot_text="1 missing")], size=120)
    d = sh.detail(300)
    d.heading("Courtyard", sub="~/Models/Courtyard.igz")
    d.caption("Links")
    d.kv([("Ride.igz", "Found", "ok"), ("Survey.igz", "Missing", "error")])
    d.buttons([{"id": "open", "label": "Open", "key": "↵", "role": 1, "enabled": True, "tip": ""}])
    sh.focus("courtyard")
    return sh.wire()


def library_sheet():
    """The Library sheet with materials, a selection and its detail."""
    from ingetrazo_render.chrome_sheet import Sheet, item
    sh = Sheet("library", "Library")
    sh.tab("materials", "Materials", count=3, on=True)
    sh.tab("components", "Components", count=12)
    sh.tab("profiles", "Profiles", count="soon", enabled=False)
    sh.verb("new", "New material", primary=True)
    sh.search("Filter materials")
    sh.side_caption("Collections")
    sh.side_item("model", "In this model", right="3", on=True)
    sh.side_item("builtin", "Built in", right="48")
    c = sh.content("In this model")
    c.tiles([item("brick", "Brick", hex="#9A5B45", sub="Image"),
             item("oak", "Oak", hex="#B08A5E", sub="Image"),
             item("concrete", "Concrete", hex="#8E8E8A", sub="Colour")], size=96)
    d = sh.detail(300)
    d.heading("Brick", hex="#9A5B45", sub="Image · 2 faces")
    d.kv([("Base", "Image"), ("Size", "215 × 65 mm"), ("IFC material", "Masonry")])
    sh.focus("brick")
    return sh.wire()


def capture_gallery(directory):
    import subprocess
    import tempfile

    root = Path(__file__).resolve().parents[1]
    binary = os.environ.get("BUILT_BIN", root / "rust/built_app/target/release/built")
    with tempfile.TemporaryDirectory() as folder:
        steps = Path(folder) / "capture.txt"
        steps.write_text("wait 1500\ncapture gallery-built\nquit\n")
        subprocess.run([binary, "--gallery", "--script", steps, "--shots", directory,
                        "--root", root, "--size", "1280x800"], check=True,
                       env=dict(os.environ, BUILT_BACKGROUND="1"))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("prefix", choices=["before", "after", "gallery"])
    parser.add_argument("--dest", default="build/design-system",
                        help="output folder, relative to the checkout")
    args = parser.parse_args()
    dest = Path(__file__).resolve().parents[1] / args.dest
    dest.mkdir(parents=True, exist_ok=True)
    isolate_script()
    os.environ.pop("BUILT_GALLERY", None)
    capture(args.prefix, dest)
