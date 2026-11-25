# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Re-export of :mod:`core.history` for older imports: the command API and
its helpers live there now (docs/commands-port.md)."""
from core import history as _history
from core.history import *  # noqa: F401,F403 -- the public command API


def __getattr__(name):
    # The private helpers (``_faces_holding``, ``_TagState`` ...) that the
    # object kernels and other lanes still read through this module.
    return getattr(_history, name)
