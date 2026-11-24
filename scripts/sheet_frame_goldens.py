# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Write the frame goldens (tests/sheet_frame_cases.py) from the composer
of views/composer.py: what a frame offers the drawing tools, its cut and
looks, its annotations and its DXF, on the Sheets fixture. Run before the
composer is deleted; the goldens stay.

    HOME=$(mktemp -d) BUILT_SETTINGS_DIR=$(mktemp -d) \\
        .venv/bin/python scripts/sheet_frame_goldens.py tests/fixtures/sheet_frames/goldens.json
"""
from __future__ import annotations

import json
import os
import sys
import tempfile
from pathlib import Path

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(ROOT / "scripts"))


def main(path: str) -> None:
    from PySide6.QtWidgets import QApplication
    QApplication.instance() or QApplication([])
    from scripts.sheet_font import use_inter
    use_inter()
    import sheets_fixture
    from app_shell.host import Host
    from tests.sheet_frame_cases import CASES, canon, reference, set_up
    from tests.test_sheets_workspace import Recorder
    from views.composer import ComposerWindow
    from views.documents import DocumentHost
    ComposerWindow.render_frame = lambda self, f: None      # model renders need the Metal view
    out = {}
    for case in CASES:
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            doc = tmp / "sheet.igz"
            sheets_fixture.build(doc)
            host = Host(0, 1280, 800, 2.0, settings=str(tmp / "settings"))
            assert host.open(doc)
            host.view.renderer = Recorder()
            composer = ComposerWindow(host.window)
            composer._window = DocumentHost(host.window, host.window._doc)
            scene = composer._scene()
            plan = scene.compositions[0].frames[0]
            set_up(case, scene, plan, host.window)
            out[case] = canon(reference(case, composer, plan, tmp))
            composer.close()
            host.view.renderer = None
            host.close()
    Path(path).parent.mkdir(parents=True, exist_ok=True)
    Path(path).write_text(json.dumps(out, indent=0, sort_keys=True))
    print(len(out), "cases")


if __name__ == "__main__":
    main(sys.argv[1])
