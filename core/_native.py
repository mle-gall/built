# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The Rust engine (``rust/ingetrazo_core``, built with maturin).

``native`` is the compiled module when the engine is on and loads, else
``None``. It is ON unless Preferences ▸ General ▸ «Rust engine» turns it
off (read once, so it applies at the next start; on by default since
0.5.10). Model maths always uses the compiled extension;
``INGETRAZO_RUST=1`` / ``=0`` overrides the setting, for tests and benches.
Every caller keeps its Python code and uses it whenever ``native`` is
``None``: the kernels make things faster, never different.

``native`` is resolved on first use, not at import: the setting lives in
QSettings, which needs the application's name set first."""
from __future__ import annotations

import logging
import os
import platform
import sys

log = logging.getLogger(__name__)

#: The General setting that turns the engine on ("1") or off ("0").
SETTING = "rust_engine"
#: Its value when the user never chose: on.
DEFAULT = "1"

_UNSET = object()
_module = _UNSET          # the compiled module, None, or not looked at yet


def available():
    """The compiled module if this build has it and it loads, else None —
    whatever the setting says (Preferences shows «not available»)."""
    global _module
    if _module is _UNSET:
        try:
            import ingetrazo_core as mod
        except Exception as exc:  # noqa: BLE001 — a missing module is the norm
            mod = None
            if not isinstance(exc, ImportError):
                log.warning("Rust engine failed to load, using Python: %s",
                            exc)
        _module = mod
    return _module


def wanted() -> bool:
    """Whether the engine is asked for: the environment first, then the
    saved preference (on by default)."""
    env = os.environ.get("INGETRAZO_RUST")
    if env is not None:
        return env != "0"
    try:
        from core import general_settings
        return str(general_settings.value(SETTING, DEFAULT)) == "1"
    except Exception:  # noqa: BLE001 — no Qt settings yet: stay on Python
        return False


def store_wanted() -> bool:
    """Use the engine's mesh store unless an oracle run explicitly opts out."""
    return os.environ.get("INGETRAZO_STORE") != "0"


def __getattr__(name):
    if name == "native":
        mod = available() if wanted() else None
        globals()["native"] = mod       # fixed for the whole session
        return mod
    raise AttributeError(name)


def engine() -> str:
    """«Rust» or «Python», for About and the logs."""
    import core._native as me
    return "Rust" if me.native is not None else "Python"


# Existing packed binary32 data retains its platform's rounding convention.
_LEGACY_HYPOT = {"win32": "msvc"}.get(sys.platform, "libstdc++")
if sys.platform == "darwin" and platform.machine() == "arm64":
    _LEGACY_HYPOT = "libc++-fma"


def qt_hypot():
    """Historical rounding convention for the engine's binary32 kernels."""
    return _LEGACY_HYPOT


#: How many starts in a row with the engine on are still under way: a
#: start adds one, the first drawn frame puts it back to 0. A start that
#: finds FAILED_STARTS of them never drawn turns the engine off
#: (start_guard).
STARTING = "rust_engine_starting"
#: One start that never drew is not the engine's doing often enough to act
#: on it: the window closed while it was loading, a second window opened
#: before the first had drawn, the machine went off. Two in a row are.
FAILED_STARTS = 2
#: The version whose start guard turned the engine off ("" when nobody or
#: the user did). Another version tries the engine again, once: what
#: stopped that start may be fixed, and on Python every edit of a large
#: model takes about twice as long — a price nobody chose.
OFF_BY_GUARD = "rust_engine_off_by_guard"
#: ``--python-engine``: this run on the normal engine, whatever is saved.
FLAG = "--python-engine"


def _count(raw) -> int:
    try:
        return max(0, int(str(raw)))
    except ValueError:
        return 0


def start_guard(argv) -> bool:
    """Before anything reads ``native``: honour ``--python-engine``, and if
    the last FAILED_STARTS starts with the engine on never reached their
    first frame, turn the engine off (the user turns it on again in
    Preferences, and the next version tries it again by itself).
    Returns True when it was turned off that way — the window says so."""
    if FLAG in argv:
        os.environ["INGETRAZO_RUST"] = "0"
        return False
    if os.environ.get("INGETRAZO_RUST") is not None:
        return False                    # tests and benches choose for us
    try:
        from PySide6.QtCore import QSettings
        from core import general_settings
        from core.version import __version__
        st = QSettings()
        on = str(general_settings.value(SETTING, DEFAULT, st)) == "1"
        off_by = str(general_settings.value(OFF_BY_GUARD, "", st))
        if not on and off_by and off_by != __version__:
            # Turned off by the guard of another version, not by the user:
            # this version gets its own try.
            general_settings.set_value(SETTING, "1", st)
            general_settings.set_value(OFF_BY_GUARD, "", st)
            general_settings.set_value(STARTING, "0", st)
            on = True
            log.info("The Rust engine was turned off by the start guard of "
                     "%s: trying it again.", off_by)
        pending = _count(general_settings.value(STARTING, "0", st))
        if on and pending >= FAILED_STARTS:
            general_settings.set_value(SETTING, "0", st)
            general_settings.set_value(STARTING, "0", st)
            general_settings.set_value(OFF_BY_GUARD, __version__, st)
            st.sync()
            log.warning("The last %d starts with the Rust engine did not "
                        "open the window: back on the Python engine.",
                        pending)
            return True
        if on:
            general_settings.set_value(STARTING, str(pending + 1), st)
            st.sync()                   # on disk before anything can crash
    except Exception as exc:  # noqa: BLE001 — never stop a start over this
        log.warning("Rust engine start guard: %s", exc)
    return False


def chosen_by_user(settings=None) -> None:
    """The user set the engine in Preferences: from now on the choice is
    theirs, and no later version turns it back on for them."""
    try:
        from core import general_settings
        general_settings.set_value(OFF_BY_GUARD, "", settings)
        general_settings.set_value(STARTING, "0", settings)
    except Exception as exc:  # noqa: BLE001
        log.warning("Rust engine start guard: %s", exc)


def start_ok() -> None:
    """The window is up and has drawn: this start with the engine worked."""
    try:
        from PySide6.QtCore import QSettings
        from core import general_settings
        st = QSettings()
        if _count(general_settings.value(STARTING, "0", st)):
            general_settings.set_value(STARTING, "0", st)
            st.sync()
    except Exception as exc:  # noqa: BLE001
        log.warning("Rust engine start guard: %s", exc)
