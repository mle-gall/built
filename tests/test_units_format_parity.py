# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The unit formatters moved to Rust (``rust/built_core/src/units.rs``)
and must read exactly as the Python ones did.

``tests/fixtures/units_format_golden.json.gz`` holds what the retired Python
implementation (``core/units.py`` at ``6f3a2576``) answered for every case
of :func:`corpus`, in order. Regenerate it only on purpose, with
``python tests/test_units_format_parity.py --write`` against the reference
implementation."""
from __future__ import annotations

import gzip
import json
import math
import random
import sys
from pathlib import Path

GOLDEN = Path(__file__).parent / "fixtures" / "units_format_golden.json.gz"

LENGTHS = ("m", "cm", "mm", "in", "ft", "ft-in", "in-frac", "ft-in-frac",
           "km", "yd", "mi", "us-ft", "", None, "parsec")
CODES = ("mm2", "cm2", "m2", "ha", "in2", "ft2", "yd2", "acre", "cm3", "l",
         "m3", "in3", "ft3", "yd3", "gal", "deg", "rad", "grad", "m", "cm",
         "mm", "km", "us-ft", "in", "ft", "ft-in", "in-frac", "ft-in-frac")


def _values():
    rng = random.Random(20261124)
    fixed = [0.0, -0.0, 1e-9, -1e-9, 0.004, 0.005, 0.0049999, 0.015, 0.025,
             0.0254, 0.3048, 0.30479999, 0.3047, 0.5, 1.0, 1.2345, -1.2345,
             1.005, 2.675, 3.65, 12 * 0.0254, 11.99 * 0.0254,
             11.999 * 0.0254, 0.3048 - 1e-7, 0.3048 + 1e-7, 1.5 * 0.0254,
             1 / 128 * 0.0254, 1 / 32 * 0.0254, 3 / 64 * 0.0254, 100.0,
             1234.5678, -987.654321, 1e6, 9.87654321e8, 123456789012.5,
             math.nan, math.inf, -math.inf, 0.1 + 0.2, 2.5, 0.125, 0.375]
    rand = [rng.uniform(-50, 50) for _ in range(40)]
    rand += [rng.uniform(-0.05, 0.05) for _ in range(20)]
    rand += [round(rng.uniform(0, 10), 3) for _ in range(20)]
    rand += [rng.randint(0, 400) * 0.0254 / 64 for _ in range(30)]
    return fixed + rand


SETTINGS = (
    {}, {"length": "m", "precision": 2}, {"length": "mm", "precision": 0},
    {"length": "cm", "precision": 1, "decimal": ","},
    {"length": "ft-in-frac", "precision": 2},
    {"length": "in", "precision": 3, "area": "ft2", "volume": "gal",
     "angle": "grad", "angle_precision": 3},
    {"length": "ft", "precision": 1, "area_precision": 4,
     "volume_precision": 0, "angle": "rad"},
    {"length": "in-frac", "precision": 4, "decimal": ","},
    {"length": "mm", "precision": 2, "area": "ha", "volume": "l"},
    {"length": "ft-in", "precision": 0},
)

#: Raw ``units`` payloads :func:`core.units.model_units_of` cleans.
RAW_UNITS = (
    None, {}, {"length": "m"}, {"length": "parsec"}, {"length": "mm", "precision": 9},
    {"length": "cm", "precision": "3"}, {"length": "cm", "precision": None},
    {"length": "in", "precision": True}, {"length": "ft", "precision": 2.7},
    {"length": "m", "area": "m3"}, {"length": "m", "area": "ha"},
    {"length": "m", "area": "auto", "area_precision": 3},
    {"length": "m", "area_precision": True}, {"length": "m", "area_precision": 7},
    {"length": "m", "volume": "l", "volume_precision": 3},
    {"length": "m", "volume_precision": 0}, {"length": "m", "angle": "rad"},
    {"length": "m", "angle": "deg", "angle_precision": 1},
    {"length": "m", "angle": "turn"}, {"length": "m", "decimal": ","},
    {"length": "m", "decimal": ";"}, {"length": "mm", "precision": 0,
     "area": "cm2", "area_precision": 1, "volume": "cm3",
     "volume_precision": 2, "angle": "grad", "angle_precision": 4,
     "decimal": ","},
    "units", [1, 2],
)


def corpus():
    """``(name, args)`` in a fixed order."""
    vals = _values()
    out = []
    for v in vals:
        for u in LENGTHS:
            for d in (-1, 0, 1, 2, 3, 4, 6, 7):
                out.append(("format_length", (v, u, d)))
            out.append(("format_length", (v, u, 2, False)))
            out.append(("format_signed_level", (v, u, 2)))
            out.append(("format_signed_level", (v, u, 0)))
            out.append(("format_signed_level", (v, u, 9)))
        for c in CODES:
            out.append(("format_value", (v, c, 2)))
            out.append(("format_value", (v, c, 0, False)))
        for s in SETTINGS:
            for fn in ("fmt_len_with", "fmt_area_with", "fmt_volume_with",
                       "fmt_angle_with"):
                out.append((fn, (v, s)))
            out.append(("fmt_angle_with", (v, s, True)))
    for s in SETTINGS:
        out.append(("preview_text", (s,)))
        for u in LENGTHS:
            out.append(("model_fmt", (s, u)))
    for raw in RAW_UNITS:
        out.append(("model_units_of", (raw,)))
        out.append(("model_units_of_dict", (raw,)))
    for raw in RAW_UNITS + tuple(SETTINGS):
        if isinstance(raw, dict):
            out.append(("profile_of", (raw,)))
            for name in ("metric", "centimetric", "millimetric", "imperial"):
                out.append(("with_profile", (raw, name)))
    for u in LENGTHS:
        for d in (None, 0, 1, 2, 3, 4, 5):
            out.append(("display_step", (u, d)))
    for text in ("", "500", " 2.5 ", "1,2", "2;3", "-1.5", "5m", "5'3", "abc",
                 "1.2.3", "+4"):
        for u in ("m", "mm", "ft-in-frac", "in"):
            out.append(("typed_value_text", (text, u)))
    return out


class _Bound:
    def __init__(self, units):
        self.units = units


def _model_fmt(units, s, u):
    """The model-bound readouts with the document's units ``s`` (its length
    replaced by ``u`` when ``u`` is a length form)."""
    scene = _Bound(dict(s, **({"length": u} if u in units._BARE_SCALE else {})))
    before = units._SCENE
    units.bind_scene(scene)
    try:
        vals = (0.0, 1.2345, -0.004, 0.3048 * 5.5, 12.0, 0.00125)
        return [units.fmt_len(v) for v in vals] + [
            units.fmt_num(v) for v in vals] + [
            units.fmt_pair(a, b) for a, b in zip(vals, vals[1:])] + [
            units.fmt_len_fine(v) for v in vals] + [
            units.fmt_triple(*vals[:3]), units.fmt_triple(*vals[3:], fine=True),
            units.fmt_area(2.5), units.fmt_volume(0.75), units.fmt_angle(0.5),
            units.fmt_angle(-0.25, signed=True), units.fmt_angle_deg(30, True),
            units.fine_precision(), units.model_unit(), units.model_precision()]
    finally:
        units.bind_scene(before)


def answer(units, name, args):
    """What ``units`` answers for one case, as JSON-safe data."""
    try:
        if name == "model_fmt":
            return _model_fmt(units, *args)
        if name == "model_units_of":
            return units.model_units_of(_Bound(args[0]))
        if name == "model_units_of_dict":
            return units.model_units_of({"units": args[0]})
        if name in ("typed_value_text",):
            text, u = args
            before = units._SCENE
            units.bind_scene(_Bound({"length": u, "precision": 2}))
            try:
                return units.typed_value_text(text)
            finally:
                units.bind_scene(before)
        if name == "display_step":
            u, d = args
            before = units._SCENE
            units.bind_scene(_Bound({"length": "mm", "precision": 1}))
            try:
                return units.display_step(u, d)
            finally:
                units.bind_scene(before)
        got = getattr(units, name)(*args)
    except Exception as exc:  # noqa: BLE001 — the error is part of the answer
        return ["error", type(exc).__name__]
    return got


def _jsonable(x):
    if isinstance(x, float) and not math.isfinite(x):
        return repr(x)
    if isinstance(x, (list, tuple)):
        return [_jsonable(i) for i in x]
    if isinstance(x, dict):
        return {k: _jsonable(v) for k, v in x.items()}
    return x


def _key(name, args):
    return f"{name}{_jsonable(list(args))!r}"


def test_formatters_read_as_the_python_ones_did():
    from core import units
    want = json.loads(gzip.decompress(GOLDEN.read_bytes()))
    cases = corpus()
    assert len(cases) == len(want)
    wrong = []
    for (name, args), expected in zip(cases, want):
        got = _jsonable(answer(units, name, args))
        if got != expected:
            wrong.append(f"{_key(name, args)}: {got!r} != {expected!r}")
    assert not wrong, f"{len(wrong)} of {len(cases)} differ:\n" + "\n".join(wrong[:40])


if __name__ == "__main__" and "--write" in sys.argv:
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    from core import units as _units
    rows = [_jsonable(answer(_units, n, a)) for n, a in corpus()]
    GOLDEN.write_bytes(gzip.compress(json.dumps(
        rows, ensure_ascii=False, separators=(",", ":")).encode(), mtime=0))
    print(len(rows), "cases ->", GOLDEN)
