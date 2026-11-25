# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Los cinco pedidos de láminas — Fase 11, la última de la 0.4.7 (Rafael).

Small ones, and two of them turned out to be bugs rather than requests:
a typed ``10:1`` silently became ``1:1``, and a new view came up wearing
a deleted one's render because the caches are keyed on ``id(frame)``.
"""
from __future__ import annotations

import ingetrazo_core
import pytest
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from ingetrazo_core import Vec3 as QVector3D

from core.composition import (Composicion, MarcoVista, format_scale,
                              parse_scale)


# ---- 1. enlargement scales, in the list and on the sheet ----------------

@pytest.mark.parametrize("n,shown", [
    (50, "1:50"), (100, "1:100"), (1, "1:1"),
    (0.5, "2:1"), (0.2, "5:1"), (0.1, "10:1"),
])
def test_an_enlargement_is_written_the_way_a_drawing_writes_it(n, shown):
    """«No me muestra la escala que yo le había metido, que era 10 a uno»
    (35:00). It used to print «ESC. 1:0.1»."""
    assert format_scale(n) == shown
    assert MarcoVista(scale_n=n).scale_label() == "ESC. " + shown


@pytest.mark.parametrize("text,n", [
    ("1:50", 50.0), ("50", 50.0), ("1:1", 1.0),
    ("10:1", 0.1), ("2:1", 0.5), ("1:0.1", 0.1),
    ("1:25,5", 25.5),                      # a comma is a decimal point here
])
def test_a_typed_scale_means_what_it_says(text, n):
    """Reading only what follows the colon turned a typed 10:1 into 1:1 —
    silently, on a technical drawing."""
    assert parse_scale(text) == pytest.approx(n)


def test_a_broken_scale_falls_back_instead_of_dividing_by_zero():
    for bad in ("", "x", "0:1", "1:0", "-2:1", None):
        assert parse_scale(bad, 100.0) == 100.0


def test_the_enlargements_are_in_the_list():
    from core.composition import COMMON_SCALES
    shown = [format_scale(n) for n in COMMON_SCALES]
    for s in ("10:1", "5:1", "2:1", "1:1", "1:50", "1:100"):
        assert s in shown


def test_the_title_block_field_says_it_too():
    from core.composition import expand_fields, set_field_context
    c = Composicion()
    c.frames.append(MarcoVista(scale_n=0.1))
    set_field_context(comp=c)
    try:
        assert expand_fields("{escala}") == "10:1"       # not «1:0.1»
        assert expand_fields("ESCALA {escala}") == "ESCALA 10:1"
    finally:
        set_field_context()


# ---- 2. sections are lettered, and can be relettered --------------------

def test_a_new_section_takes_a_LETTER_not_a_number():
    """«Las secciones se nombran así: no suele ser un 1, sino A-A, B-B»
    (50:45). The mark draws the letter at both ends, so «A … A»."""
    from ingetrazo_core.section import SectionPlane
    V = QVector3D
    planes = []
    for _ in range(4):
        planes.append(SectionPlane(V(0, 0, 0), V(0, 0, 1),
                                   symbol=ingetrazo_core.section_next_symbol(planes)))
    assert [p.symbol for p in planes] == ["A", "B", "C", "D"]
    # a gap is filled, and past Z it carries
    assert ingetrazo_core.section_next_symbol([planes[0], planes[2]]) == "B"
    assert ingetrazo_core.section_next_symbol([SectionPlane(V(0, 0, 0), V(0, 0, 1), symbol=chr(65 + i))
                        for i in range(26)]) == "AA"


def test_the_section_tool_letters_the_plane_it_places():
    from core.scene import Scene
    from tests.test_parity_tools import _Vp, _ctx
    from tools.section import SectionPlaneTool

    vp = _Vp(Scene())
    tool = SectionPlaneTool()
    for x in range(2):
        tool.on_activate(vp)
        tool.on_click(_ctx(vp, x, 0))
    assert [p.symbol for p in vp.scene.section_planes] == ["A", "B"]


def test_the_plane_can_be_renamed_after_it_is_placed(native_host):
    from core.history import PlaceSectionPlaneCommand
    from ingetrazo_core.section import SectionPlane
    from ingetrazo_render import context_menu_build, context_menu_run
    from ingetrazo_render import chrome_forms

    win = native_host.window
    vp = native_host.viewport
    plane = SectionPlane(QVector3D(0, 0, 0), QVector3D(0, 0, 1),
                         name="Original", symbol="A")
    vp.history.execute(PlaceSectionPlaneCommand(plane))
    vp.scene.select([plane])
    menu = context_menu_build(win, native_host.view)
    assert "secname" in menu.ids()
    assert context_menu_run(win, native_host.view, menu.kind, "secname", menu.sel, facts=menu.facts)
    form = chrome_forms.current(win)
    form.receive("name", "South elevation")
    form.receive("symbol", "S")
    form.press("ok")
    assert (plane.name, plane.symbol) == ("South elevation", "S")
    vp.history.undo()
    assert (plane.name, plane.symbol) == ("Original", "A")


# ---- 3. the N of the north arrow ----------------------------------------

def test_the_north_arrow_keeps_its_box():
    """The letter moved, nothing else: a sheet's north arrow stays put."""
    from core.composition import FlechaNorte
    a = FlechaNorte(size_mm=20.0)
    assert (a.w_mm, a.h_mm) == (20.0, 20.0)
    old = Composicion.from_dict({"nortes": [{"x_mm": 1.0, "size_mm": 20.0}]})
    assert (old.nortes[0].w_mm, old.nortes[0].h_mm) == (20.0, 20.0)


# ---- 4. an image magnetises to the drawing under it ---------------------


# ---- 5. a new view wearing a dead one's render --------------------------


