"""Public units operations sampled before retiring the Python facade."""
import json
from dataclasses import astuple
from pathlib import Path
from types import SimpleNamespace

from core import units


def observations():
    result = {"convert": {}, "document": {}}
    for src in units.UNITS.values():
        result["convert"][src.code] = {
            dst.code: units.convert(-1.2345, src.code, dst.code).hex()
            for dst in units.UNITS.values() if dst.kind == src.kind
        }
    previous = units._SCENE
    try:
        scene = SimpleNamespace(units={}, dimension_style={})
        units.bind_scene(scene)
        for code in units.UNIT_CHOICES:
            units.apply_units(scene, {"length": code, "decimal": ",", "precision": 3})
            with units.unitless_numbers():
                with units.unitless_numbers():
                    raw = units.parse("1.25").value
                outer = units.bare_number_scale()
            result["document"][code] = {
                "contexts": [list(units.typed_context(kind))
                             for kind in ("length", "area", "volume", "angle")],
                "parsed": [list(astuple(units.parse(text))) for text in
                           ("1.25", "5-3 1/2", "=2*45cm", "3 unknown", "5 m²")],
                "spinbox": [units.length_number(1.2345, code).hex(),
                            units.metres_from_number(1.2345, code).hex(),
                            units.length_suffix(code)],
                "labels": [units.fmt_triple(1.25, 2.51, 0.005, fine=True),
                           units.fmt_angle_deg(-18.25, signed=True),
                           units.typed_value_text("1;2"), units.localized("1.2")],
                "scopes": [raw, outer, units.bare_number_scale()],
                "style": dict(scene.dimension_style),
            }
    finally:
        units.bind_scene(previous)
    return result


def test_units_facade_parity():
    fixture = Path(__file__).parent / "fixtures" / "units_facade_golden.json"
    assert observations() == json.loads(fixture.read_text())
