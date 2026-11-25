# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Background opacity, the frame border that prints only when asked, and
the configurable sheet border."""
from __future__ import annotations

from built_core import Input as Qt

from core.composition import (Composicion, CotaItem, MarcoVista,
                              PAPER_SIZES_MM, TextoItem)


def _rgb(img, x, y):
    return img.pixel(x, y) & 0xFFFFFF


def test_legacy_fixed_scale_labels_load_as_bound_texts():
    """Documents saved before the movable scale label carried it as a frame
    flag the panel no longer exposes — the user had no way to switch it
    off. Loading converts each one into a text block bound to its frame."""
    from core.composition import PT_TO_MM, expand_fields, set_field_context
    c = Composicion()
    for i, pos in enumerate(("under-right", "under-left", "inside-br",
                             "inside-bl")):
        c.frames.append(MarcoVista(x_mm=20.0, y_mm=20.0 + i * 60.0, w_mm=80.0,
                                   h_mm=40.0, scale_n=40.0, show_scale=True,
                                   scale_pos=pos, scale_mm=3.0))
    c.frames.append(MarcoVista(x_mm=120.0, y_mm=20.0, w_mm=50.0, h_mm=30.0,
                               scale_n=100.0, show_scale=False))
    loaded = Composicion.from_dict(c.to_dict())
    assert all(f.show_scale is False for f in loaded.frames)
    assert len(loaded.texts) == 4                # the plain frame gets none
    by_pos = dict(zip(("under-right", "under-left", "inside-br", "inside-bl"),
                      loaded.texts))
    for f, (pos, t) in zip(loaded.frames, by_pos.items()):
        assert f.uid and t.frame_uid == f.uid and t.follow
        assert t.text == "ESC. {escala}" and t.bold
        assert abs(t.size_pt - 3.0 / PT_TO_MM) < 1e-6
        if pos.startswith("under"):
            assert t.y_mm > f.y_mm + f.h_mm and not t.bg_color
        else:
            assert f.y_mm < t.y_mm < f.y_mm + f.h_mm and t.bg_color
        assert t.align == ("left" if pos in ("under-left", "inside-bl") else "right")
    ur = by_pos["under-right"]
    assert ur.x_mm + ur.w_mm == loaded.frames[0].x_mm + loaded.frames[0].w_mm
    set_field_context(comp=loaded)
    assert expand_fields(ur.text, ur.frame_uid) == "ESC. 1:40"
    # Custom templates keep their wording; the number follows the frame.
    c2 = Composicion()
    c2.frames.append(MarcoVista(scale_n=75.0, show_scale=True,
                                scale_text="Escala {n}"))
    t2 = Composicion.from_dict(c2.to_dict()).texts[0]
    assert t2.text == "Escala {escala}"
    set_field_context(comp=Composicion.from_dict(c2.to_dict()))
    # Round trip: an already-migrated sheet loads unchanged.
    again = Composicion.from_dict(loaded.to_dict())
    assert len(again.texts) == 4


