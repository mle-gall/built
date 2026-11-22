"""Startup of the Qt app on a small document, measured the way
``built --measure`` measures the Rust shell: process start → first Metal
frame of the opened document, and peak RSS.

Usage: cd <checkout> && .venv/bin/python scripts/shell_vs_qt_startup.py <file.igz> [runs]

Each run is a fresh process (Qt app: this script with --one; shell:
rust/built_app/target/release/built --measure). Throwaway settings for
both; the window shows for about a second each run.
"""
import json
import os
import re
import resource
import subprocess
import sys
import tempfile
import time


def one(path):
    t0 = time.perf_counter()
    os.environ["INGETRAZO_WGPU"] = "1"
    sys.path.insert(0, os.getcwd())
    from PySide6.QtCore import QCoreApplication, QSettings, QTimer
    from PySide6.QtWidgets import QApplication
    app = QApplication([])
    settings = tempfile.mkdtemp(prefix="built-qt-measure-")
    os.environ["XDG_DATA_HOME"] = settings
    QSettings.setDefaultFormat(QSettings.IniFormat)
    for scope in (QSettings.UserScope, QSettings.SystemScope):
        QSettings.setPath(QSettings.IniFormat, scope, settings)
    app.setApplicationName("IngeTrazo")
    app.setOrganizationName("IngeTrazo")
    from pathlib import Path
    from views.main_window import MainWindow
    imported = time.perf_counter()
    win = MainWindow()
    win.resize(1280, 800)
    win.show()
    shown = time.perf_counter()
    out = {}

    def first_frame(*_):
        if "first_frame_ms" not in out:
            out["first_frame_ms"] = (time.perf_counter() - t0) * 1000
            QTimer.singleShot(0, app.quit)

    def open_doc():
        display = getattr(win._doc, "stand_in", None)
        if display is not None and hasattr(display, "frameReady"):
            display.frameReady.connect(first_frame)
        out["open_start_ms"] = (time.perf_counter() - t0) * 1000
        win.open_path(Path(path))
        out["opened_ms"] = (time.perf_counter() - t0) * 1000
        display = getattr(win._doc, "stand_in", None)
        if display is not None:
            display.frameReady.connect(first_frame)
            display.update()

    QTimer.singleShot(0, open_doc)
    QTimer.singleShot(20000, app.quit)
    app.exec()
    out.update(imports_ms=(imported - t0) * 1000, window_ms=(shown - t0) * 1000,
               metal=bool(getattr(win._doc, "_renderer_wgpu", False)),
               peak_rss_mb=resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 2**20)
    win._doc.viewport.scene.version  # keep alive until here
    print("QT " + json.dumps(out), flush=True)
    os._exit(0)


def run(cmd, tag):
    start = time.perf_counter()
    p = subprocess.run(["/usr/bin/time", "-l", *cmd], capture_output=True, text=True, timeout=120)
    wall = (time.perf_counter() - start) * 1000
    rss = re.search(r"(\d+)\s+maximum resident set size", p.stderr)
    line = next((l for l in p.stdout.splitlines() if l.startswith(tag)), "")
    data = json.loads(line[len(tag):].strip() or "{}") if line else {}
    data["process_wall_ms"] = wall
    data["peak_rss_mb_time_l"] = int(rss.group(1)) / 2**20 if rss else None
    return data


def main():
    if sys.argv[1] == "--one":
        one(sys.argv[2])
    path = sys.argv[1]
    runs = int(sys.argv[2]) if len(sys.argv) > 2 else 3
    settings = tempfile.mkdtemp(prefix="built-shell-measure-")
    results = {"qt": [], "shell": []}
    for _ in range(runs):
        results["qt"].append(run([sys.executable, __file__, "--one", path], "QT "))
        results["shell"].append(run(["rust/built_app/target/release/built", path, "--measure",
                                     "--settings", settings], "[built] measure "))
    print(json.dumps(results, indent=1))


if __name__ == "__main__":
    main()
