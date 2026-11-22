"""The Sheets workspace in the native ``built`` shell, in a real window
(scripts/sheets_shell_smoke.sh): open a document with a sheet, Sheets from
the workspace ▾, select and drag on the page, zoom, export a PDF, switch
back and forth, ⌃Tab to the model. It opens a window, so it runs only with
``BUILT_WINDOW_TESTS=1`` and a built binary
(``cargo build --release --manifest-path rust/built_app/Cargo.toml``)."""
import os
import re
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
BIN = ROOT / "rust" / "built_app" / "target" / "release" / "built"

pytestmark = pytest.mark.skipif(
    os.environ.get("BUILT_WINDOW_TESTS") != "1" or not BIN.exists(),
    reason="opens a window: BUILT_WINDOW_TESTS=1 and the built binary")


def test_sheets_in_the_built_shell(tmp_path):
    shots = tmp_path / "shots"
    out = subprocess.run(["sh", str(ROOT / "scripts" / "sheets_shell_smoke.sh")],
                         env={**os.environ, "SHOTS": str(shots)},
                         capture_output=True, text=True, timeout=300).stdout
    assert re.search(r"pdf: +\d+ bytes", out), out
    assert "plist untouched" in out, out
    for name in ("02-sheets", "04-selected", "05-zoomed", "07-sheets-again", "08-model"):
        assert (shots / f"{name}.png").stat().st_size > 10_000, name
    # The one error is AppKit offering the --script file as a document.
    assert out.count("[built] error in") <= 1, out
