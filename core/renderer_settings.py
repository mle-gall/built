# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Probe the GPU backend in a child before starting the window."""
import json
import platform
import subprocess
import sys
from dataclasses import dataclass
from functools import lru_cache
from PySide6.QtCore import qVersion


@dataclass(frozen=True)
class Capability:
    available: bool
    reason: str = ''
    python_version: str = platform.python_version()
    qt_version: str = qVersion()


def _probe_local():
    """Child only: exercise this crate's actual adapter/device requirements."""
    try:
        import ingetrazo_render
    except Exception as exc:
        reason = f'render crate unavailable: {exc}'
        return Capability(False, reason)
    try:
        from PySide6.QtWidgets import QApplication, QWidget
        from PySide6.QtCore import Qt
        app = QApplication.instance() or QApplication([])
        view = QWidget()
        view.setAttribute(Qt.WidgetAttribute.WA_NativeWindow)
        view.resize(16, 16)
        from views.wgpu_surface import window_handles
        window, handles = window_handles(view)
        renderer = ingetrazo_render.Renderer(window, 16, 16, 1.0, **handles)
        renderer.ui_enable(None)
        del renderer
        view.close()
        return Capability(True)
    except BaseException as exc:  # also catches PyO3's PanicException
        return Capability(False, f'GPU adapter/device unavailable: {exc}')


@lru_cache(maxsize=1)
def capability():
    """Probe once, before window construction, with a bounded crash boundary."""
    command = ([sys.executable, '--probe-gpu'] if getattr(sys, 'frozen', False)
               else [sys.executable, '-m', 'core.renderer_settings'])
    try:
        result = subprocess.run(command, capture_output=True, text=True,
                                timeout=30, check=False)
        if result.returncode:
            return Capability(False, f'GPU probe exited with code {result.returncode}')
        # Qt/drivers may write diagnostics before our final JSON line.
        data = json.loads(result.stdout.strip().splitlines()[-1])
        return Capability(**data)
    except subprocess.TimeoutExpired:
        return Capability(False, 'GPU probe timed out')
    except (OSError, ValueError, IndexError, TypeError) as exc:
        return Capability(False, f'GPU probe failed: {exc}')


def status_text():
    cap = capability()
    return 'wgpu' if cap.available else f'No supported GPU backend: {cap.reason}'


def probe_main():
    from dataclasses import asdict
    print(json.dumps(asdict(_probe_local())), flush=True)


if __name__ == '__main__':
    probe_main()
