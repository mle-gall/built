# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Keep the English and Spanish tool hints within their portable text budget."""
from __future__ import annotations

import pytest
from ingetrazo_core import Vec3 as QVector3D
from core.i18n import set_language, current_language
from tests.shell_isolation import native_window, restore_shell_patches  # noqa: F401
from views.status_hints import HINTS, hint_for

_MAX_CHARS = 112

#: Every state Alt can be in. The third one, «parallel / perpendicular», is
#: the longest and the first version of this test never measured it.
_MODES = ("all", "off", "parallel_perp")


#: What puts a tool past its idle phase, for the ones whose phase is not
#: the plain ``start_point`` that :func:`views.status_hints.phase_of`
#: checks first.
_STARTERS = {"fillet": ("sizing", True)}


def _start(key: str, tool) -> bool:
    """Put the tool mid-operation. False when it has no such phase."""
    attr, value = _STARTERS.get(key, ("start_point", QVector3D(1.0, 1.0, 0.0)))
    if not hasattr(tool, attr):
        return False
    try:
        setattr(tool, attr, value)
    except AttributeError:      # a read-only property: not ours to drive
        return False
    return True


@pytest.mark.parametrize("locale", ["en", "es"])
def test_tool_hints_fit_the_text_budget(native_window, locale):
    win = native_window
    previous = current_language()
    set_language(locale)
    vp = win.viewport
    bad = []
    try:
        for key in sorted(HINTS):
            try:
                win._activate_tool(key)
            except Exception:       # noqa: BLE001 — not every key is a tool
                continue
            tool = vp.active_tool
            if tool is None:
                continue
            for started in (False, True):
                # Only the tools that really can be mid-operation get
                # driven there, through whatever phase_of reads for them —
                # the fillet goes by ``sizing`` and its ``start_point`` is
                # read-only. Inventing the state on the others would
                # measure a line nobody can ever see.
                if started and not _start(key, tool):
                    continue
                for mode in _MODES:
                    text = hint_for(win._tool_key(tool), tool, None, mode)
                    if len(text) > _MAX_CHARS:
                        bad.append(f"{key} ({len(text)} chars > {_MAX_CHARS}): {text}")
    finally:
        set_language(previous)
    assert not bad, "\n".join(bad)
