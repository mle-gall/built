"""The shell imports ``app_shell.host`` before its bootstrap puts the venv on
``sys.path``: importing it must not import ``core`` or a native module, or
``core`` registers without them and later imports fail (core.topology)."""
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def test_app_shell_host_imports_without_the_venv():
    probe = (
        "import sys\n"
        f"sys.path.insert(0, {str(ROOT)!r})\n"
        "import app_shell.host\n"
        "early = sorted(m for m in sys.modules if m.split('.')[0] in "
        "('core', 'ingetrazo_core', 'ingetrazo_render', 'PySide6'))\n"
        "assert not early, early\n"
    )
    result = subprocess.run([sys.executable, "-S", "-c", probe], cwd="/",
                            capture_output=True, text=True, timeout=60)
    assert result.returncode == 0, result.stderr
