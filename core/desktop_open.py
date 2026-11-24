# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Open files and URLs with the desktop's registered applications."""
from __future__ import annotations

import os
import shutil
import subprocess
import sys
from pathlib import Path


def _launch_ourselves(environ=None) -> bool:
    env = os.environ if environ is None else environ
    if not sys.platform.startswith("linux") or not (getattr(sys, "frozen", False) or env.get("APPIMAGE")):
        return False
    return not env.get("FLATPAK_ID") and not env.get("SNAP")


def host_env(environ=None) -> dict:
    """The environment a host program expects: the Blender child's
    (``core.render_blender.clean_env``) without the AppImage's GIO
    redirect either."""
    from core.render_blender import clean_env
    env = clean_env(environ)
    gio, appdir = env.get("GIO_MODULE_DIR", ""), env.get("APPDIR", "")
    if gio and appdir and gio.startswith(appdir):
        env.pop("GIO_MODULE_DIR")
    return env


def open_url(target) -> bool:
    """Open a path or URL with the operating system's registered handler."""
    if hasattr(target, "isLocalFile"):
        target = target.toLocalFile() if target.isLocalFile() else target.toString()
    arg = str(target)
    try:
        if sys.platform == "win32":
            os.startfile(arg)
        else:
            opener = "/usr/bin/open" if sys.platform == "darwin" else shutil.which("xdg-open")
            if not opener:
                return False
            subprocess.Popen([opener, arg], env=host_env() if _launch_ourselves() else None,
                             stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                             stderr=subprocess.DEVNULL, start_new_session=True)
        return True
    except OSError:
        return False


def reveal(path) -> bool:
    """Show ``path`` selected in the desktop's file manager (Finder's
    «Show in Finder»); elsewhere open its folder."""
    p = Path(path)
    try:
        if sys.platform == "darwin" and p.exists():
            subprocess.Popen(["open", "-R", str(p)], stdin=subprocess.DEVNULL,
                             stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            return True
        if sys.platform.startswith("win") and p.exists():
            subprocess.Popen(["explorer", "/select,", str(p)])
            return True
    except OSError:
        pass
    folder = p.parent
    return open_url(folder) if folder.exists() else False
