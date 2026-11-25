# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Tool pointers by name, for document bindings whose host brings no
factory of its own.

The icons and pointers are the renderer's (``ingetrazo_render``:
``icon_art`` for toolbar, panel and tree icons, ``tool_cursor_name`` and
``tool_cursor_raster`` for pointers). The Classic window's Qt wrappers left
with it; the native window shows a pointer by its name.
"""
from __future__ import annotations


def tool_cursor(key: str | None, plus: bool = False) -> str | None:
    """The pointer for tool ``key``: ``"tool:<key>"`` (``+`` when the Tape
    or Protractor will leave a guide), ``"crosshair"``, or None to keep the
    arrow (Select, unknown keys, the arrow preference)."""
    from core import general_settings
    from ingetrazo_render import tool_cursor_name
    return tool_cursor_name(key, plus, str(general_settings.value("cursor_style", "tool")))


def solid_cursor(state: str) -> str:
    """The Solid Tools pointer: «×» over anything that is not a solid, «1»
    or «2» over a solid for the pick it will be."""
    return "tool:solid-" + state
