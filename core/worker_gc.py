# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Only the main thread frees cyclic garbage.

CPython runs an automatic collection in whichever thread crosses the
allocation threshold. A worker that walks the model in Python (the snap
index build reads every edge through store views, `views/wgpu_view.py`)
allocates enough to start one, and that pass frees whatever cyclic
garbage the whole process holds: windows, timers and widgets the main
thread let go of. A QObject destroyed on the worker ("QObject::killTimer:
Timers cannot be stopped from another thread") crashed the wgpu test
batch with a segmentation fault or a bus error, one run in three or four.

So a collection that starts on any other thread first freezes the heap
(``gc.freeze``: every tracked object moves to the permanent generation,
an O(1) list splice) and finds nothing to free; the next collection on
the main thread unfreezes it into the oldest generation, where the main
thread's own passes free it. Callbacks run before a pass picks its
generations, so the frozen objects are not examined.
"""
from __future__ import annotations

import gc
import threading

_main = threading.main_thread().ident


def _on_collect(phase, _info) -> None:
    if phase != "start":
        return
    if threading.get_ident() == _main:
        if gc.get_freeze_count():
            gc.unfreeze()
    else:
        gc.freeze()


def install() -> None:
    """Idempotent: once per process, from the main thread."""
    if _on_collect not in gc.callbacks:
        gc.callbacks.append(_on_collect)
