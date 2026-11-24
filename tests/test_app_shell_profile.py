"""Check launch profiles without using the developer's preferences."""
import os
from pathlib import Path
import plistlib
from scripts import isolated_launch
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
    script = """
        import sys
        from pathlib import Path
        from PySide6.QtCore import QSettings, QStandardPaths
        from app_shell import qt_host
        mode, root = sys.argv[1], Path(sys.argv[2])
        if mode == "temporary":
            # Exercise the offscreen host's automatic temporary profile in isolation.
            import os
            os.environ.pop("BUILT_SETTINGS_DIR")
        folder = qt_host.start(str(root / "explicit") if mode == "directory" else None,
                               temporary=mode != "directory")
        from core.settings import settings as product_settings
        from ingetrazo_core import NativeSettings
        assert isinstance(product_settings(), NativeSettings)
        assert QStandardPaths.isTestModeEnabled()
        assert QSettings.defaultFormat() == QSettings.IniFormat
        assert Path(QSettings().fileName()).is_relative_to(folder)
        if mode == "directory":
            assert Path(folder) == root / "explicit"
        elif mode == "environment":
            assert Path(folder) == root / "env"
    """
    env["TMPDIR"] = str(tmp_path)
    isolated_launch.run([sys.executable, "-c", textwrap.dedent(script), mode, str(tmp_path)],
                   env=env, home=tmp_path, settings=tmp_path / "env", check=True, timeout=45)


def test_qt_launcher_respects_isolated_profile(tmp_path):
    profile = tmp_path / "settings"
    env = dict(os.environ, HOME=str(tmp_path), CFFIXED_USER_HOME=str(tmp_path),
               BUILT_SETTINGS_DIR=str(profile))
    script = """
        import sys
        from pathlib import Path
        from PySide6.QtCore import QSettings
        import main
        def check():
            assert QSettings.defaultFormat() == QSettings.IniFormat
            assert Path(QSettings().fileName()).is_relative_to(sys.argv[2])
            return 0
        main._self_check = check
        sys.exit(main.main())
    """
    isolated_launch.run([sys.executable, "-c", textwrap.dedent(script), "--check", str(profile)],
                   env=env, home=tmp_path, settings=profile, check=True, timeout=45)


def test_launchers_share_an_isolated_profile(tmp_path):
    profile = tmp_path / "shared"
    script = """
        import sys
        from pathlib import Path
        from PySide6.QtCore import QSettings
        def exchange():
            from core import general_settings, settings
            from ingetrazo_core import NativeSettings
            st = QSettings()
            user = settings.settings()
            assert isinstance(user, NativeSettings) == (sys.argv[1] == "native")
            assert st.format() == QSettings.IniFormat
            assert Path(st.fileName()).is_relative_to(sys.argv[4])
            assert st.value("recent_files", [], type=list) == ([sys.argv[2]] if sys.argv[2] else [])
            assert general_settings.value("test_profile", "initial") == (sys.argv[2] or "initial")
            general_settings.set_value("test_profile", sys.argv[3])
            user.setValue("recent_files", [sys.argv[3]])
            user.sync()
            assert st.status() == QSettings.NoError
        if sys.argv[1] == "native":
            from app_shell.host import Host
            host = Host(0, 1280, 800, 2)
            assert Path(host.settings_dir) == Path(sys.argv[4])
            exchange()
            host.close()
        else:
            import main
            set_org = main._App.setOrganizationName
            def ready(self, name):
                set_org(name)
                exchange()
                raise SystemExit(0)
            main._App.setOrganizationName = ready
            main.main()
            raise AssertionError("Qt startup did not initialize its profile")
    """
    env = dict(os.environ, QT_QPA_PLATFORM="offscreen")
    for mode, old, new in (("native", "", "native.built"), ("qt", "native.built", "qt.built"),
                           ("native", "qt.built", "last.built")):
        isolated_launch.run([sys.executable, "-c", textwrap.dedent(script), mode, old, new,
                             str(profile)], env=env, home=tmp_path, settings=profile,
                            check=True, timeout=45)


@pytest.mark.skipif(sys.platform != "darwin", reason="macOS native preference domain")
def test_native_defaults_read_and_write_the_product_domain(tmp_path):
    # Test the production CFPreferences backend alone, without booting a launcher.
    prefs = tmp_path / "Library/Preferences"
    prefs.mkdir(parents=True)
    plist = prefs / "com.ingetrazo.IngeTrazo.plist"
    plist.write_bytes(plistlib.dumps({"recent_files": ["seed.built"], "General.autosave": "0", "shortcuts.x·y": "F9", "unknown": b"old widget state"}))
    (prefs / "com.builtprofileprobe.Probe.plist").write_bytes(plistlib.dumps({"home": str(tmp_path)}))
    dylib = tmp_path / "preferences.dylib"
    source = Path(__file__).parent / "fixtures/shell/fake_preferences.c"
    import subprocess
    subprocess.run(["clang", "-dynamiclib", source, "-framework", "CoreFoundation", "-o", dylib], check=True)
    script = """
        from pathlib import Path
        from PySide6.QtCore import QCoreApplication, QSettings, QStandardPaths
        QCoreApplication.setOrganizationName("BuiltProfileProbe")
        QCoreApplication.setApplicationName("Probe")
        assert QSettings().value("home") == str(Path.home())
        QCoreApplication.setOrganizationName("IngeTrazo")
        QCoreApplication.setApplicationName("IngeTrazo")
        st = QSettings()
        assert st.format() == QSettings.NativeFormat
        assert Path(st.fileName()) == Path.home() / "Library/Preferences/com.ingetrazo.IngeTrazo.plist"
        assert not QStandardPaths.isTestModeEnabled()
        for loc in (QStandardPaths.AppDataLocation, QStandardPaths.CacheLocation):
            assert Path(QStandardPaths.writableLocation(loc)).resolve().is_relative_to(Path.home() / "Library")
        assert st.value("recent_files") == ["seed.built"]
        import os
        # The successful probe above established that CFPreferences is
        # interposed into this test's container before selecting the domain.
        os.environ.pop("BUILT_SETTINGS_DIR")
        from ingetrazo_core import NativeSettings
        store = NativeSettings()
        assert store.value("recent_files") == ["seed.built"]
        assert store.value("General/autosave", type=bool) is False
        store.beginGroup("shortcuts")
        assert store.childKeys() == ["x.y"]
        assert store.value("x.y") == "F9"
        store.endGroup()
        store.setValue("recent_files", ["last.built"])
        store.setValue("new/foo.bar·baz", 9)
        store.sync()
        st.sync()
        assert st.value("new/foo.bar·baz") == 9
        assert st.status() == QSettings.NoError
    """
    isolated_launch.run([sys.executable, "-c", textwrap.dedent(script)], home=tmp_path,
                        env=dict(os.environ, DYLD_INSERT_LIBRARIES=str(dylib)), check=True, timeout=45)
    saved = plistlib.loads(plist.read_bytes())
    assert saved["recent_files"] == ["last.built"]
    assert saved["unknown"] == b"old widget state"


def test_native_profile_retains_unrelated_entries_and_group_identity(tmp_path):
    from ingetrazo_core import NativeSettings
    path = tmp_path / "IngeTrazo/IngeTrazo.ini"
    path.parent.mkdir()
    opaque = "; extension-owned\ngeometry=@Variant(opaque)\n"
    path.write_text("[General]\n" + opaque + "[%General]\nundo=42\n")
    st = NativeSettings(tmp_path)
    assert st.value("General/undo", type=int) == 42
    st.beginGroup("custom")
    st.setValue("nested/é:·🌲", ['@abc', ' x ', 'comma, quote"', 'x\\x'])
    st.setValue("piece", 2.5)
    st.setValue("piece", 3.5)
    assert st.childKeys() == ["piece"]
    assert st.allKeys() == ["nested/é:·🌲", "piece"]
    assert NativeSettings(tmp_path).value("custom/nested/é:·🌲") == ['@abc', ' x ', 'comma, quote"', 'x\\x']
    assert path.read_text().count("[custom]") == 1
    st.remove("nested")
    st.endGroup()
    assert st.value("custom/piece", type=float) == 3.5
    assert opaque in path.read_text()


def test_native_processes_merge_independent_keys_without_qt(tmp_path):
    import subprocess
    script = textwrap.dedent("""
        import sys
        from core.settings import settings, use_native
        use_native(None)
        st = settings()
        for i in range(30):
            st.setValue(f"{sys.argv[1]}/{i}", i)
        assert not any(s.startswith("PySide6") for s in sys.modules)
    """)
    env = dict(os.environ, BUILT_SETTINGS_DIR=str(tmp_path))
    writers = [subprocess.Popen([sys.executable, "-c", script, k], env=env)
               for k in ("keyboard", "navigation")]
    assert [proc.wait(timeout=30) for proc in writers] == [0, 0]
    from ingetrazo_core import NativeSettings
    store = NativeSettings(tmp_path)
    for k in ("keyboard", "navigation"):
        for i in range(30):
            assert store.value(f"{k}/{i}", type=int) == i
