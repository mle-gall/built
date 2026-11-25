# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The composer's Items list (issue #93, @pacaeiro): items can be renamed
and grouped by type. It keeps its own tab, the properties the next one —
Marco tried them sharing one tab, the list folded and floating, and kept
separate tabs (26-09). No sheet navigator: the sheet
strip at the bottom already switches sheets."""
from __future__ import annotations

from built_core import Input as Qt
def _close(comp, win):
    comp.close()
    win._saved_version = win.viewport.scene.version
    win.close()


def test_the_name_survives_the_igz(tmp_path):
    from built_core.composition import Composicion, MarcoVista
    c = Composicion()
    c.frames.append(MarcoVista(list_name="Alzado norte"))
    back = Composicion.from_dict(c.to_dict())
    assert back.frames[0].list_name == "Alzado norte"
