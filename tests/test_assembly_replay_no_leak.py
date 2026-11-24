"""Undoing a part deleted inside a shared assembly with the object-mesh
oracle once panicked in assembly replay (a slot still named a released
branch) and left the document transaction open, so every later step in
the process was refused. However the replay ends, nothing may stay open."""
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def test_assembly_undo_in_object_mode_never_leaves_a_transaction_open(tmp_path):
    env = dict(os.environ, INGETRAZO_STORE="0", QT_QPA_PLATFORM="offscreen",
               HOME=str(tmp_path), BUILT_SETTINGS_DIR=str(tmp_path / "settings"),
               PYTHONPATH=str(ROOT / "tests"))
    node = "tests/test_assembly_edit_shared.py::test_a_part_changed_two_levels_deep_undoes_in_place"
    out = subprocess.run([sys.executable, "-m", "pytest", "-q", "-s", "-p", "no:cacheprovider",
                          "-p", "leak_guard", node], cwd=ROOT, env=env,
                         capture_output=True, text=True, timeout=300)
    text = out.stdout + out.stderr
    assert "passed" in text or "failed" in text, text[-2000:]
    assert "OPEN-TRANSACTION" not in text, text[-2000:]
    assert "PanicException" not in text and "panicked at" not in text, text[-2000:]
