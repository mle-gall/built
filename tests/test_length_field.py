# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Document-unit input in the Levels plugin, native 3D Text and the value box."""
from __future__ import annotations

import random

import pytest
from core import units  # noqa: E402
from core.scene import Scene  # noqa: E402
from examples.extensions.niveles import LengthSpinBox  # noqa: E402
from tests.controller_surface import ControllerSurface as Viewport

IN = 0.0254
FT = 0.3048


@pytest.fixture
def doc():
    scene = Scene()
    units.bind_scene(scene)

    def set_units(length, precision=2):
        scene.units = {"length": length, "precision": precision}
    yield set_units
    units.bind_scene(None)


def test_the_field_shows_the_documents_unit(doc):
    doc("mm", 0)
    box = LengthSpinBox(minimum=0.001, maximum=1000.0)
    box.setValue(0.25)
    assert box.text() == "250 mm"
    assert box.value() == pytest.approx(0.25)         # metres behind it
    doc("m", 2)
    box = LengthSpinBox(minimum=0.001, maximum=1000.0)
    box.setValue(0.25)
    assert box.text() == "0.250 m"                    # to the millimetre
    doc("ft-in-frac", 2)
    box = LengthSpinBox(minimum=0.0, maximum=1000.0)
    box.setValue(FT + 6.5 * IN)
    assert box.text() == "1'6 1/2\""


@pytest.mark.parametrize("typed, metres", [
    ("20", 0.020), ("2,5 cm", 0.025), ("1\"", IN), ("0.1m", 0.1),
    ("3/4\"", 0.75 * IN), ("1m 20", 1.2),
])
def test_the_field_reads_what_is_typed(doc, typed, metres):
    doc("mm", 0)
    box = LengthSpinBox(minimum=0.0, maximum=1000.0)
    assert not box.read(typed)[1]
    assert box.read(typed)[0] == pytest.approx(metres)


def test_the_field_refuses_what_is_not_a_length(doc):
    doc("mm", 0)
    box = LengthSpinBox(minimum=0.001, maximum=1.0)
    box.setValue(0.25)
    for bad in ("5 m²", "abc", "2 l"):
        assert box.read(bad)[0] is None
        assert box.value() == pytest.approx(0.25)
    assert box.read("5 m²")[1] == "Expected a length"
    assert box.read("2 m")[1].startswith("Out of range")
    assert box.read("300")[1] == ""


def test_arrows_step_one_shown_unit(doc):
    doc("mm", 0)
    box = LengthSpinBox(minimum=0.0, maximum=1000.0)
    box.setValue(0.25)
    box.stepBy(1)
    assert box.value() == pytest.approx(0.251)
    assert box.text() == "251 mm"


def test_values_survive_the_field_unrounded_to_the_micron(doc):
    """Generated values go in and come out of the field unchanged (Qt keeps
    9 decimals of metres) whatever the shown precision."""
    doc("cm", 0)
    box = LengthSpinBox(minimum=0.0, maximum=1000.0)
    rng = random.Random(287)
    for _ in range(300):
        v = round(rng.uniform(0, 900), 6)
        box.setValue(v)
        assert box.value() == pytest.approx(v, abs=1e-9)


def test_text3d_dialog_previews_the_font():
    from tests.test_text3d_units import exercise_picker
    draft, (out,) = exercise_picker("m", (None, None),
        {"text":"Abc", "font":"Georgia", "bold":True, "italic":False},
        changes={"font":"Helvetica", "bold":False, "italic":True})
    assert (draft["text"], draft["font"], draft["bold"], draft["italic"]) == ("Abc", "Georgia", True, False)
    assert (out["font"], out["bold"], out["italic"]) == ("Helvetica", False, True)


def test_text3d_dialog_in_millimetres():
    from tests.test_text3d_units import exercise_picker
    draft, (out,) = exercise_picker("mm", ("20", "3"), precision=0)
    assert draft["lengths"]["text"] == ["250 mm", "50 mm"]
    assert out["height"] == pytest.approx(.020) and out["thickness"] == pytest.approx(.003)


def test_text3d_dialog_in_metres_is_as_before():
    from tests.test_text3d_units import exercise_picker
    draft, (out,) = exercise_picker("m", (None, "20cm"))
    assert draft["lengths"]["text"] == ["0.250 m", "0.050 m"]
    assert out["height"] == pytest.approx(.25) and out["thickness"] == pytest.approx(.20)


def test_text3d_dialog_in_inches():
    from tests.test_text3d_units import exercise_picker
    draft, (out,) = exercise_picker("in-frac", ("1 1/2", None),
        {"text": "A", "height": 2 * IN, "thickness": .25 * IN})
    assert draft["lengths"]["text"] == ['2"', '1/4"']
    assert out["height"] == pytest.approx(1.5 * IN)


# ---- the value box ----------------------------------------------------------

def test_the_value_box_takes_feet_and_inches_with_spaces(doc):
    doc("m")
    parse = Viewport._parse_value_buffer
    assert parse('5\' 3 1/2"') == pytest.approx(5 * FT + 3.5 * IN)
    assert parse('5\' 3"') == pytest.approx(5 * FT + 3 * IN)
    assert parse('3\'-6 1/2"') == pytest.approx(3 * FT + 6.5 * IN)
    w, h = parse('5\';3"')                          # a 5 ft × 3 in rectangle
    assert w == pytest.approx(5 * FT) and h == pytest.approx(3 * IN)
    assert parse("1m20") == pytest.approx(1.2)
    assert parse("1 2") == (1.0, 2.0)               # still two plain fields
    assert parse("2m 3") == (2.0, 3.0)
    assert parse("5 m²") is None


# ---- #189: what a Mac keyboard types -------------------------------------------

class _Key:
    def __init__(self, text):
        self._t = text

    def text(self):
        return self._t

    def key(self):
        return 0

    def modifiers(self):
        from built_core import Input as Qt
        return Qt.NoModifier


def _typed(chars):
    """The value box's buffer after typing ``chars`` one key at a time."""
    vp = Viewport.__new__(Viewport)
    vp._value_buffer = ""
    vp.active_tool = object()
    vp.valueBufferChanged = type("S", (), {"emit": lambda self, t: None})()
    vp.update = lambda: None
    for ch in chars:
        vp._handle_value_key(_Key(ch))
    return vp._value_buffer


def test_fraction_characters_and_smart_quotes_are_typed(doc):
    """«1 ⅜", 1 3/16”» on a Mac in fractional inches: ⅜ and ” were dropped,
    and the comma read as a decimal joined both values (#189)."""
    doc("in-frac", 2)
    buf = _typed('1⅜”, 1 3/16”')
    assert buf == '1 3/8", 1 3/16"'
    w, h = Viewport._parse_value_buffer(buf, comma_lists=True)
    assert w == pytest.approx(1.375 * IN) and h == pytest.approx(1.1875 * IN)
    assert _typed("⅜”") == '3/8"'
    assert _typed("5’ 3½”") == "5' 3 1/2\""


@pytest.mark.parametrize("typed, inches", [
    ('1 ⅜", 1 3/16”', (1.375, 1.1875)),
    ("48, 96", (48, 96)),                     # comma and space: two values
    ('1 3/8"; 1 3/16"', (1.375, 1.1875)),
])
def test_rectangle_values_with_comma_and_space(doc, typed, inches):
    doc("in-frac", 2)
    for comma_lists in (False, True):
        got = Viewport._parse_value_buffer(typed, comma_lists=comma_lists)
        assert got == pytest.approx(tuple(v * IN for v in inches))


def test_a_decimal_comma_still_reads_as_one(doc):
    doc("m", 2)
    assert Viewport._parse_value_buffer("2,5") == pytest.approx(2.5)
    assert Viewport._parse_value_buffer("1,5;2,5") == pytest.approx((1.5, 2.5))
    assert units.parse("1⅜”", unit="in-frac").value == pytest.approx(1.375 * IN)
