"""Select storage and the language before opening a native window."""
import os
from pathlib import Path
import sys
import tempfile


def start(directory=None, *, temporary=False):
    from core import i18n, settings

    profile = directory or os.environ.get("BUILT_SETTINGS_DIR")
    if not profile and temporary:
        profile = tempfile.mkdtemp(prefix="built-shell-settings-")
    if not profile and ("pytest" in sys.modules or "PYTEST_CURRENT_TEST" in os.environ):
        raise RuntimeError("pytest must supply an isolated shell profile")
    if profile:
        data = Path(profile) / "data"
        data.mkdir(parents=True, exist_ok=True)
        os.environ["XDG_DATA_HOME"] = str(data)
    settings.use_native(profile)
    i18n.set_language(i18n.resolve_language(settings.settings().value("language")))
    return profile
