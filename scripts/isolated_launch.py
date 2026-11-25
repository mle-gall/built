"""Launch a test process with private settings and verify macOS preferences."""
from __future__ import annotations

import os
from pathlib import Path
import subprocess
import sys
import tempfile


def preferences_mtime():
    if sys.platform != "darwin":
        return None
    import pwd
    real_home = Path(pwd.getpwuid(os.getuid()).pw_dir)
    prefs = real_home / "Library/Preferences"
    stamps = {}
    for name in ("com.ingetrazo.IngeTrazo.plist", "com.built.Built.plist"):
        try:
            stamps[name] = (prefs / name).stat().st_mtime_ns
        except FileNotFoundError:
            stamps[name] = None
    return stamps


def run(args, *, env=None, settings=None, home=None, **options):
    with tempfile.TemporaryDirectory(prefix="built-launch-") as folder:
        home = str(home or folder)
        settings = str(settings or Path(folder) / "settings")
        Path(settings).mkdir(parents=True, exist_ok=True)
        child = dict(os.environ if env is None else env)
        child.update(HOME=home, CFFIXED_USER_HOME=home, BUILT_SETTINGS_DIR=settings, BUILT_BACKGROUND="1",
                     XDG_CONFIG_HOME=settings, XDG_DATA_HOME=str(Path(settings) / "data"))
        before = preferences_mtime()
        try:
            return subprocess.run(args, env=child, **options)
        finally:
            if preferences_mtime() != before:
                raise RuntimeError("The macOS Built preferences plist changed during the launch")


def isolate_script():
    """Run even an os._exit-based harness under a parent that checks the plist."""
    script = str(Path(sys.argv[0]).resolve())
    if os.environ.pop("BUILT_ISOLATED_SCRIPT", None) != script:
        result = run([sys.executable, *sys.argv], env=dict(os.environ, BUILT_ISOLATED_SCRIPT=script))
        raise SystemExit(result.returncode)


if __name__ == "__main__":
    raise SystemExit(run(sys.argv[1:]).returncode)
