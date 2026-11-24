# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Resolve the active document viewport for plugin panels."""


def live_viewport(viewport):
    """A callable that answers the viewport of the project in front of
    ``viewport``'s window — what a panel built with the viewport of its day
    keeps, so it acts on whichever project the user is looking at. It keeps
    the WINDOW, never that viewport: a panel holding a viewport would hold
    its whole closed project in memory. A viewport outside a tabbed window
    (tests, a lone widget) is answered as it is."""
    try:
        win = viewport.window()
    except (AttributeError, RuntimeError):
        win = None
    if win is not None and hasattr(win, "documents"):
        return lambda: win.viewport
    return lambda: viewport
