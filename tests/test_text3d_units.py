# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The 3D Text dialog reads lengths in the model's unit (#287): someone
designing 3D-printed parts in millimetres was asked for the text height
and extrusion in metres. The parameters it returns stay in metres."""
from __future__ import annotations

import json
from types import SimpleNamespace
from unittest.mock import Mock

import pytest

from core import units
from core.scene import Scene
from ingetrazo_render import font_length
from views import font_picker


def exercise_picker(unit, typed, params=None, precision=2, changes=None, cancel=False):
    scene = Scene()
    scene.units = {"length": unit, "precision": precision}
    view = SimpleNamespace(_ui_attached=True, renderer=Mock(), update=lambda: None)
    win = SimpleNamespace(viewport=SimpleNamespace(scene=scene), _doc=SimpleNamespace(stand_in=view))
    applied = []
    previous = units._SCENE
    try:
        units.bind_scene(scene)
        font_picker.text3d(win, params, applied.append)
        draft = json.loads(view.renderer.ui_font_picker.call_args.args[0])
        choice = dict(draft, **(changes or {}))
        for key, low, text in zip(("height", "thickness"), (draft["minimum"], 0.), typed):
            if text is not None:
                choice[key] = font_length(text, unit, low, draft["maximum"])
        font_picker.answer(win, "" if cancel else json.dumps(choice))
        return draft, applied
    finally:
        units.bind_scene(previous)


def test_a_millimetre_model_asks_for_millimetres():
    draft, (out,) = exercise_picker("mm", ("120", "8"), precision=0)
    assert draft["lengths"]["text"] == ["250 mm", "50 mm"]
    assert out["height"] == pytest.approx(.12) and out["thickness"] == pytest.approx(.008)


def test_editing_shows_the_text_in_the_model_unit():
    draft, (out,) = exercise_picker("cm", ("30", "2"), {"text":"A", "height":.5, "thickness":.1})
    assert draft["height"] == 50 and draft["thickness"] == 10 and draft["unit"].strip() == "cm"
    assert out["height"] == pytest.approx(.3)


def test_a_metre_model_is_as_before():
    draft, (out,) = exercise_picker("m", (".4", ".02"))
    assert draft["lengths"]["text"] == ["0.250 m", "0.050 m"]
    assert out["height"] == pytest.approx(.4)


def test_cancel_does_not_start_placement():
    draft, applied = exercise_picker("m", (None, None), cancel=True)
    assert draft["editable"] and not applied


@pytest.mark.parametrize("typed", ["2 m²", "-2", "very tall", "1001 m"])
def test_invalid_lengths_cannot_apply(typed):
    with pytest.raises(ValueError):
        font_length(typed, "m", .001, 1000.)
