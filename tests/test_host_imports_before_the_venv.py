"""The embedded launcher must install site-packages before importing the document or renderer."""
import json
import os
from pathlib import Path
import site
import sys

import pytest

from scripts.isolated_launch import run


def test_launcher_processes_the_venv_before_importing_app_code(tmp_path):
    root = Path(__file__).resolve().parents[1]
    binary = Path(os.environ.get("BUILT_BINARY", root / "rust/built_app/target/release/built"))
    if not binary.exists():
        pytest.skip("requires the native launcher")
    venv = tmp_path / "venv"
    packages = venv / ("Lib/site-packages" if sys.platform == "win32" else
                       f"lib/python{sys.version_info.major}.{sys.version_info.minor}/site-packages")
    packages.mkdir(parents=True)
    snapshot = tmp_path / "early.json"
    (packages / "early_probe.py").write_text(
        "import json, sys\nfrom pathlib import Path\n"
        "early = sorted(name for name in sys.modules if name.split('.')[0] in "
        "('core', 'app_shell', 'built_core', 'built_render'))\n"
        f"Path({str(snapshot)!r}).write_text(json.dumps(early))\n")
    (packages / "first.pth").write_text("import early_probe\n" + "".join(
        f"import site; site.addsitedir({entry!r})\n" for entry in site.getsitepackages()))
    result = run([str(binary), "--root", str(root), "--venv", str(venv), "--check"],
                 cwd=tmp_path, capture_output=True, text=True, timeout=60)
    assert result.returncode == 0, result.stderr
    assert json.loads(snapshot.read_text()) == []
