# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Whether this machine has a GPU the window can draw with
(``ingetrazo_render.gpu_adapter``); the shell's own answer replaces this
once it runs (``ingetrazo_render.shell_files.install``)."""
import platform
from dataclasses import dataclass
from functools import lru_cache


@dataclass(frozen=True)
class Capability:
    available: bool
    reason: str = ''
    python_version: str = platform.python_version()
    adapter: str = ""


@lru_cache(maxsize=1)
def capability():
    """Ask once for an adapter that can draw Built's scene."""
    try:
        import ingetrazo_render
    except Exception as exc:  # noqa: BLE001 — a missing or broken crate is the answer
        return Capability(False, f'render crate unavailable: {exc}')
    try:
        return Capability(True, adapter=ingetrazo_render.gpu_adapter())
    except BaseException as exc:  # also catches PyO3's PanicException
        return Capability(False, f'GPU adapter/device unavailable: {exc}')


def status_text():
    cap = capability()
    return 'wgpu' if cap.available else f'No supported GPU backend: {cap.reason}'
