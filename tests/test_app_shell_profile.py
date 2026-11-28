"""Check launch profiles without using the developer's preferences."""
import os
from pathlib import Path
import plistlib
from scripts import isolated_launch
import sys
import textwrap

import pytest


def test_pytest_requires_isolation(monkeypatch):
    from built_render import start_profile
    monkeypatch.delenv("BUILT_SETTINGS_DIR", raising=False)
    with pytest.raises(RuntimeError, match="isolated shell profile"):
        start_profile()


def test_offscreen_document_fixture_keeps_a_cold_process_qt_free(tmp_path):
    script = """
        import os
        import sys
        flag = os.environ.get("PYSIDE6_OPTION_PYTHON_ENUM")
        from tests.shell_isolation import open_test_host
        with open_test_host(os.environ["BUILT_SETTINGS_DIR"]) as host:
            assert host.viewport is not None
        loaded = [name for name in sys.modules
                  if name.partition(".")[0] in {"PySide6", "shiboken6"}]
        assert not loaded, loaded
        assert os.environ.get("PYSIDE6_OPTION_PYTHON_ENUM") == flag
    """
    isolated_launch.run([sys.executable, "-c", textwrap.dedent(script)],
                        home=tmp_path / "home", settings=tmp_path / "settings",
                        check=True, timeout=45)


@pytest.mark.parametrize("mode", ["directory", "environment", "temporary"])
def test_isolated_profiles(tmp_path, mode):
    env = dict(os.environ, HOME=str(tmp_path), CFFIXED_USER_HOME=str(tmp_path))
    script = """
        import sys
        from pathlib import Path
        from built_render import start_profile
        mode, root = sys.argv[1], Path(sys.argv[2])
        if mode == "temporary":
            # Exercise the offscreen host's automatic temporary profile in isolation.
            import os
            os.environ.pop("BUILT_SETTINGS_DIR")
        folder = start_profile(str(root / "explicit") if mode == "directory" else None,
                               temporary=mode != "directory")
        from built_core.settings import settings as product_settings
        from built_core import NativeSettings
        assert isinstance(product_settings(), NativeSettings)
        product_settings().setValue("profile_probe", mode)
        assert NativeSettings(folder).value("profile_probe") == mode
        assert (Path(folder) / "Built/Built.ini").is_file()
        if mode == "directory":
            assert Path(folder) == root / "explicit"
        elif mode == "environment":
            assert Path(folder) == root / "env"
    """
    env["TMPDIR"] = str(tmp_path)
    isolated_launch.run([sys.executable, "-c", textwrap.dedent(script), mode, str(tmp_path)],
                   env=env, home=tmp_path, settings=tmp_path / "env", check=True, timeout=45)


def test_native_launches_share_an_isolated_profile(tmp_path):
    profile = tmp_path / "shared"
    script = """
        import sys
        from pathlib import Path
        def exchange():
            from built_core import settings
            from built_core import general_settings
            from built_core import NativeSettings
            st = settings.settings()
            user = settings.settings()
            assert isinstance(user, NativeSettings) == (sys.argv[1] == "native")
            assert st.value("recent_files", [], type=list) == ([sys.argv[2]] if sys.argv[2] else [])
            assert general_settings.value("test_profile", "initial") == (sys.argv[2] or "initial")
            general_settings.set_value("test_profile", sys.argv[3])
            user.setValue("recent_files", [sys.argv[3]])
            user.sync()
        from built_render import Host
        host = Host(0, 1280, 800, 2)
        assert Path(host.settings_dir) == Path(sys.argv[4])
        exchange()
        host.close()
    """
    env = dict(os.environ, QT_QPA_PLATFORM="offscreen")
    for mode, old, new in (("native", "", "native.built"), ("native", "native.built", "second.built"),
                           ("native", "second.built", "last.built")):
        isolated_launch.run([sys.executable, "-c", textwrap.dedent(script), mode, old, new,
                             str(profile)], env=env, home=tmp_path, settings=profile,
                            check=True, timeout=45)


@pytest.mark.skipif(sys.platform != "darwin", reason="macOS native preference domain")
def test_native_defaults_read_and_write_the_product_domain(tmp_path):
    # Test the production CFPreferences backend alone, without booting a launcher.
    prefs = tmp_path / "Library/Preferences"
    prefs.mkdir(parents=True)
    plist = prefs / "com.built.Built.plist"
    plist.write_bytes(plistlib.dumps({"isolation_marker": str(tmp_path), "recent_files": ["seed.built"], "General.autosave": "0", "shortcuts.x·y": "F9", "unknown": b"old widget state"}))
    (prefs / "com.builtprofileprobe.Probe.plist").write_bytes(plistlib.dumps({"home": str(tmp_path)}))
    dylib = tmp_path / "preferences.dylib"
    source = Path(__file__).parent / "fixtures/shell/fake_preferences.c"
    import subprocess
    subprocess.run(["clang", "-dynamiclib", source, "-framework", "CoreFoundation", "-o", dylib], check=True)
    script = """
        from pathlib import Path
        import os
        os.environ.pop("BUILT_SETTINGS_DIR")
        from built_core import NativeSettings
        store = NativeSettings()
        assert store.value("recent_files") == ["seed.built"]
        assert store.value("General/autosave", type=bool) is False
        store.beginGroup("shortcuts")
        assert store.childKeys() == ["x.y"]
        assert store.value("x.y") == "F9"
        store.endGroup()
        # The seed is unique to the interposed temporary preference domain.
        assert store.value("isolation_marker") == str(Path.home())
        store.setValue("recent_files", ["last.built"])
        store.setValue("new/foo.bar·baz", 9)
        store.sync()
        assert store.value("new/foo.bar·baz") == 9
    """
    isolated_launch.run([sys.executable, "-c", textwrap.dedent(script)], home=tmp_path,
                        env=dict(os.environ, DYLD_INSERT_LIBRARIES=str(dylib)), check=True, timeout=45)
    saved = plistlib.loads(plist.read_bytes())
    assert saved["recent_files"] == ["last.built"]
    assert saved["unknown"] == b"old widget state"


def test_native_profile_retains_unrelated_entries_and_group_identity(tmp_path):
    from built_core import NativeSettings
    path = tmp_path / "Built/Built.ini"
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
        from built_core.settings import settings, use_native
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
    from built_core import NativeSettings
    store = NativeSettings(tmp_path)
    for k in ("keyboard", "navigation"):
        for i in range(30):
            assert store.value(f"{k}/{i}", type=int) == i


def test_collected_tests_and_children_inherit_private_directories():
    import subprocess
    probe = """
import os
from pathlib import Path
from built_core import NativeSettings
home = Path.home()
assert home == Path(os.environ['CFFIXED_USER_HOME'])
assert home.parent.name.startswith('built-pytest-')
profile = Path(os.environ['BUILT_SETTINGS_DIR'])
assert profile.is_relative_to(home.parent)
assert Path(os.environ['BUILT_LOG_DIR']).is_relative_to(home.parent)
assert Path(os.environ['XDG_DATA_HOME']).is_relative_to(home.parent)
NativeSettings().setValue('isolation/child_process', str(home))
assert NativeSettings(profile).value('isolation/child_process') == str(home)
"""
    subprocess.run([sys.executable, "-c", probe], check=True, timeout=20)


@pytest.mark.skipif(sys.platform != "darwin", reason="macOS log location")
def test_log_guard_covers_new_log_names(tmp_path, monkeypatch):
    import pwd
    from types import SimpleNamespace
    from scripts.isolated_launch import log_mtimes

    monkeypatch.setattr(pwd, "getpwuid", lambda uid: SimpleNamespace(pw_dir=str(tmp_path)))
    before = log_mtimes()
    logs = tmp_path / "Library/Logs/Built"
    logs.mkdir(parents=True)
    (logs / "new-diagnostic.log").write_text("synthetic")
    assert log_mtimes() != before


@pytest.mark.parametrize(("getter", "saved", "message"), [
    ("preferences_mtime", "_real_preferences_before", "plist changed"),
    ("log_mtimes", "_real_logs_before", "logs changed"),
])
def test_session_guard_rejects_a_changed_file(tmp_path, monkeypatch, getter, saved, message):
    from types import SimpleNamespace
    from tests import conftest

    probe = tmp_path / "guarded-file"
    probe.write_bytes(b"seed")
    baseline = probe.stat().st_mtime_ns
    monkeypatch.setattr(conftest, saved, baseline)
    monkeypatch.setattr(conftest, getter, lambda: probe.stat().st_mtime_ns)
    session = SimpleNamespace(config=SimpleNamespace())
    conftest.pytest_sessionfinish(session, 0)
    os.utime(probe, ns=(baseline, baseline + 2_000_000_000))
    with pytest.raises(pytest.exit.Exception, match=message) as failure:
        conftest.pytest_sessionfinish(session, 0)
    assert failure.value.returncode == pytest.ExitCode.TESTS_FAILED


def test_batched_runner_propagates_a_session_failure_and_finishes_other_batches(tmp_path):
    import subprocess

    (tmp_path / "test_first.py").write_text("def test_ok(): pass\n")
    (tmp_path / "test_second.py").write_text("def test_ok(): pass\n")
    (tmp_path / "conftest.py").write_text(textwrap.dedent("""
        import pytest
        from pathlib import Path
        def pytest_sessionfinish(session, exitstatus):
            if any(str(arg).endswith("test_first.py") for arg in session.config.args):
                pytest.exit("synthetic profile guard", returncode=1)
            Path(__file__).with_name("second-finished").touch()
    """))
    runner = Path(__file__).resolve().parents[1] / "scripts" / "pytest_batches.py"
    result = subprocess.run([sys.executable, str(runner), "--batches", "2", str(tmp_path)],
                            cwd=tmp_path, capture_output=True, text=True)
    assert result.returncode == 1
    assert "synthetic profile guard" in result.stderr
    assert (tmp_path / "second-finished").is_file()
