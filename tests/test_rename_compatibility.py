"""One-release namespace and preference migration contracts."""
import importlib
import os
from pathlib import Path
import plistlib
import subprocess
import sys

import pytest


def test_old_environment_and_explicit_precedence():
    code = "import os; assert os.environ['BUILT_REFUSALS'] == os.environ['EXPECTED']"
    env = dict(os.environ, INGETRAZO_REFUSALS="old", EXPECTED="old")
    env.pop("BUILT_REFUSALS", None)
    subprocess.run([sys.executable, "-c", code], env=env, check=True)
    env.update(BUILT_REFUSALS="", EXPECTED="")
    subprocess.run([sys.executable, "-c", code], env=env, check=True)


def test_removed_extension_names_in_a_fresh_interpreter():
    subprocess.run([sys.executable, "-c", """
from importlib import import_module
from built_core.scene import Scene
from built_core.mesh import Mesh
for name in ("ingetrazo_core", "ingetrazo_render", "ingetrazo_rfa", "core.mesh", "core.scene"):
    try:
        import_module(name)
    except ModuleNotFoundError as error:
        assert error.name == name, error
    else:
        raise AssertionError(name)
"""], check=True)


@pytest.mark.skipif(sys.platform != "darwin", reason="CFPreferences domain")
def test_once_only_migration_in_temporary_domains(tmp_path):
    from scripts import isolated_launch
    prefs = tmp_path / "Library/Preferences"
    prefs.mkdir(parents=True)
    old = prefs / "com.ingetrazo.IngeTrazo.plist"
    new = prefs / "com.built.Built.plist"
    old.write_bytes(plistlib.dumps({"legacy": "original", "opaque": b"abc", "existing": "old"}))
    new.write_bytes(plistlib.dumps({"existing": "new"}))
    lib = tmp_path / "prefs.dylib"
    subprocess.run(["clang", "-dynamiclib", Path(__file__).parent / "fixtures/shell/fake_preferences.c",
                    "-framework", "CoreFoundation", "-o", lib], check=True)
    code = """import os
os.environ.pop('BUILT_SETTINGS_DIR')
from built_core import NativeSettings
s = NativeSettings()
assert s.value('existing') == 'new'
assert s.value('legacy') == 'original'
assert s.value('opaque') == b'abc'
assert s.value('later') is None
"""
    for run in range(2):
        if run:
            old.write_bytes(plistlib.dumps({"later": "must never be copied"}))
        before = old.read_bytes(), old.stat().st_mtime_ns
        isolated_launch.run([sys.executable, "-c", code], home=tmp_path,
                            env=dict(os.environ, DYLD_INSERT_LIBRARIES=str(lib)), check=True)
        assert (old.read_bytes(), old.stat().st_mtime_ns) == before
