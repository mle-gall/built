"""Check launch profiles without using the developer's preferences."""
import os
from pathlib import Path
import plistlib
import subprocess
import sys
import textwrap

import pytest


def test_pytest_requires_isolation(monkeypatch):
    from app_shell import qt_host
    monkeypatch.delenv("BUILT_SETTINGS_DIR", raising=False)
    with pytest.raises(RuntimeError, match="isolated shell profile"):
        qt_host.start()


@pytest.mark.parametrize("mode", ["directory", "environment", "temporary"])
def test_isolated_profiles(tmp_path, mode):
    env = dict(os.environ, HOME=str(tmp_path), CFFIXED_USER_HOME=str(tmp_path))
    env.pop("BUILT_SETTINGS_DIR", None)
    if mode == "environment":
        env["BUILT_SETTINGS_DIR"] = str(tmp_path / "env")
    script = """
        import sys
        from pathlib import Path
        from PySide6.QtCore import QSettings, QStandardPaths
        from app_shell import qt_host
        mode, root = sys.argv[1], Path(sys.argv[2])
        folder = qt_host.start(str(root / "explicit") if mode == "directory" else None,
                               temporary=mode != "directory")
        assert QStandardPaths.isTestModeEnabled()
        assert QSettings.defaultFormat() == QSettings.IniFormat
        assert Path(QSettings().fileName()).is_relative_to(folder)
        if mode == "directory":
            assert Path(folder) == root / "explicit"
        elif mode == "environment":
            assert Path(folder) == root / "env"
    """
    env["TMPDIR"] = str(tmp_path)
    subprocess.run([sys.executable, "-c", textwrap.dedent(script), mode, str(tmp_path)],
                   env=env, check=True, timeout=45)


@pytest.mark.skipif(sys.platform != "darwin", reason="macOS native preference domain")
def test_native_and_classic_share_normal_profile(tmp_path):
    prefs = tmp_path / "Library/Preferences"
    prefs.mkdir(parents=True)
    plist = prefs / "com.ingetrazo.IngeTrazo.plist"
    plist.write_bytes(plistlib.dumps({"recent_files": ["seed.built"]}))
    # Verify interposition with a separate domain before either app starts.
    (prefs / "com.builtprofileprobe.Probe.plist").write_bytes(plistlib.dumps({"home": str(tmp_path)}))
    dylib = tmp_path / "preferences.dylib"
    source = Path(__file__).parent / "fixtures/shell/fake_preferences.c"
    subprocess.run(["clang", "-dynamiclib", source, "-framework", "CoreFoundation", "-o", dylib], check=True)
    env = dict(os.environ, HOME=str(tmp_path), CFFIXED_USER_HOME=str(tmp_path),
               DYLD_INSERT_LIBRARIES=str(dylib), QT_QPA_PLATFORM="offscreen")
    for key in ("BUILT_SETTINGS_DIR", "PYTEST_CURRENT_TEST", "XDG_DATA_HOME", "INGETRAZO_WGPU"):
        env.pop(key, None)
    script = """
        import sys
        from pathlib import Path
        from PySide6.QtCore import QSettings, QStandardPaths
        assert QSettings("BuiltProfileProbe", "Probe").value("home") == str(Path.home())
        def exchange():
            from core import recent, general_settings, renderer_settings
            st = QSettings()
            assert st.format() == QSettings.NativeFormat
            assert Path(st.fileName()) == Path.home() / "Library/Preferences/com.ingetrazo.IngeTrazo.plist"
            assert not QStandardPaths.isTestModeEnabled()
            for loc in (QStandardPaths.AppDataLocation, QStandardPaths.CacheLocation):
                path = Path(QStandardPaths.writableLocation(loc)).resolve()
                assert path.is_relative_to(Path.home().resolve() / "Library"), path
            assert recent.entries()[0]["path"] == sys.argv[2]
            assert renderer_settings.requested() == sys.argv[4]
            general_settings.set_value(renderer_settings.SETTING, sys.argv[5])
            st.setValue("recent_files", [sys.argv[3]])
            st.sync()
            assert st.status() == QSettings.NoError
        if sys.argv[1] == "native":
            from app_shell.host import Host
            host = Host(0, 1280, 800, 2)
            assert host.settings_dir is None
            exchange()
            host.close()
        else:
            # Stop classic startup immediately after it sets its application identity.
            import main
            set_org = main._App.setOrganizationName
            def profile_ready(self, name):
                set_org(name)
                exchange()
                raise SystemExit(0)
            main._App.setOrganizationName = profile_ready
            main.main()
            raise AssertionError("classic startup did not initialize the profile")
    """
    renderer = "opengl"
    for mode, old, new in (("native", "seed.built", "native.built"),
                           ("classic", "native.built", "classic.built"),
                           ("native", "classic.built", "last.built")):
        choice = "opengl" if renderer == "wgpu" else "wgpu"
        subprocess.run([sys.executable, "-c", textwrap.dedent(script), mode, old, new,
                        renderer, choice],
                       env=env, check=True, timeout=45)
        renderer = choice
        assert plistlib.loads(plist.read_bytes())["recent_files"] == [new]
