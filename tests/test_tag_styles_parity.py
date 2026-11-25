# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""A tag's styles moved to Rust (``rust/ingetrazo_core/src/model/tag_styles.rs``)
and must answer as ``core/layers.py``'s Python did at ``9cf53080``: colours,
hatch grounds and inks, the paint and look a tag gives its objects, the
line styles a sheet and the model ink with, the legend's rows, the hatched
cut's patterns, grounds, inks and forms, the line types' colours and the
tags left out of the sheets.

``tests/fixtures/tag_styles_golden.json.gz`` holds those answers on seeded
documents. Regenerate only on purpose, against the reference
implementation: ``python tests/test_tag_styles_parity.py --write``."""
from __future__ import annotations

import gzip

import ingetrazo_core
import json
import math
import random
import sys
from pathlib import Path

GOLDEN = Path(__file__).parent / "fixtures" / "tag_styles_golden.json.gz"

HATCHES = ("brick", "concrete", "insulation", "brick_flemish", "tiles", "parquet",
           "pat:mine", "pat:gone", "block", "solid", None)


def _scene(rng):
    from core.group import Group
    from core.layers import Layer
    from core.mesh import Mesh
    from core.scene import Scene
    from core.style import Style
    s = Scene()
    s.hatch_patterns = {"mine": {"label": "Mine", "families": [[45.0, 0, 0, 0, 2.0, []]]}}
    if rng.random() < 0.5:
        s.bricks = {"brick": (240.0, 115.0, 52.0, 10.0)}
    if rng.random() < 0.5:
        s.line_types = {"mine": {"label": "Mine", "pattern": [5.0, -2.0],
                                 "color": [0.1, 0.2, 0.3], "text": "GAS", "text_size": 2.0,
                                 "text_at": 1},
                        "plain": {"label": "Plain", "pattern": [3.0, -1.0, 0.0, -1.0]}}
    if rng.random() < 0.4:
        s.linetype_colors = {"dashed": (1.0, 0.0, 0.0)}
    for k in range(rng.randint(2, 8)):
        kw = {"visible": rng.random() < 0.85, "locked": rng.random() < 0.2,
              "printable": rng.random() < 0.85}
        if rng.random() < 0.5:
            kw["color"] = tuple(round(rng.random(), 3) for _ in range(3))
        if rng.random() < 0.5:
            kw["linetype"] = rng.choice(("dashed", "dot", "lin:mine", "lin:plain", "lin:gone",
                                         "continuous"))
        if rng.random() < 0.4:
            kw["lineweight"] = rng.choice((0.25, 0.5))
        if rng.random() < 0.3:
            kw["cut_weight"] = 0.7
        if rng.random() < 0.3:
            kw["opacity"] = 0.4
        h = rng.choice(HATCHES)
        if h is not None:
            kw["hatch"] = h
            kw["hatch_angle"] = rng.choice((0.0, 30.0))
            kw["hatch_scale"] = rng.choice((1.0, 2.5))
            kw["hatch_ground"] = rng.choice((None, "layer", (0.9, 0.9, 0.9)))
            kw["hatch_color"] = rng.choice((None, "layer", (0.0, 0.0, 0.0)))
            kw["hatch_on_paper"] = rng.choice((None, True, False))
            kw["brick"] = rng.choice((None, (250.0, 120.0, 55.0, 10.0)))
            kw["hatch_cross"] = rng.random() < 0.3
            kw["joints"] = rng.choice((None, "single", "double"))
            kw["mortar"] = rng.random() < 0.3
            kw["hatch_offset"] = rng.choice((None, (10.0, 0.0)))
        name = rng.choice(("Walls", "Steel", "Doors", "Glass", "Roof", "Notes")) + f" {k}"
        if rng.random() < 0.2:
            name += " / Untagged"
        s.layers.append(Layer(name, **kw))
    for ly in list(s.layers)[1:]:
        if rng.random() < 0.3:
            ly.folder = "F"
    if rng.random() < 0.3:
        s.layer_folders = ["F"]
        s.folder_visibility = {"F": rng.random() < 0.5}
    st = Style()
    st.color_by_layer = rng.random() < 0.5
    s.display_style = st
    g = Group(Mesh(), name="g")
    g.layer = rng.choice([ly.key for ly in s.layers] + [None])
    return s, g


def _j(x):
    if isinstance(x, dict):
        return {str(k): _j(v) for k, v in x.items()}
    if isinstance(x, (list, tuple)):
        return [_j(v) for v in x]
    if isinstance(x, float) and not math.isfinite(x):
        return repr(x)
    return x


def answers(L):
    rng = random.Random(20261125)
    out = []
    for _ in range(70):
        s, g = _scene(rng)
        a = {}
        per = []
        for ly in s.layers:
            per.append([ingetrazo_core.tag_styles.hatch_ground_rgb(ly), ingetrazo_core.tag_styles.hatch_ink_rgb(ly), ingetrazo_core.tag_styles.light_layer_color(ly),
                        ingetrazo_core.tag_styles.layer_color(ly), ingetrazo_core.tag_styles.is_link_untagged(ly.name), ingetrazo_core.tag_styles.layer_brick(s, ly),
                        ingetrazo_core.tag_styles.layer_hatch_families(s, ly), ingetrazo_core.tag_styles.hatch_label(s, ly.hatch),
                        ingetrazo_core.tag_styles.hatch_grows(ly), ingetrazo_core.tag_styles.hatch_families(s, ly.hatch),
                        ingetrazo_core.tag_styles.linetype_pattern(ly.linetype, s), ingetrazo_core.tag_styles.linetype_text(ly.linetype, s),
                        ingetrazo_core.tag_styles.linetype_color(ly.linetype, s)])
        a["per"] = per
        a["none_color"] = ingetrazo_core.tag_styles.layer_color(None)
        a["object"] = [ingetrazo_core.tag_styles.object_layer(g), ingetrazo_core.tag_styles.object_layer(None)]
        a["paint"] = ingetrazo_core.tag_styles.layer_paint(s, g)
        a["fx"] = ingetrazo_core.tag_styles.layer_fx(s)
        a["look"] = ingetrazo_core.tag_styles.layer_look(s, g, {"color": (1, 0, 0)})
        a["wanted"] = [ingetrazo_core.tag_styles.layer_lines_wanted(s), ingetrazo_core.tag_styles.layer_lines_wanted(s, s.display_style)]
        a["styles"] = [ingetrazo_core.tag_styles.line_styles(s, True), ingetrazo_core.tag_styles.line_styles(s, False)]
        a["model_styles"] = ingetrazo_core.tag_styles.model_line_styles(s)
        a["legend"] = L.legend_rows(s)
        a["cut"] = [ingetrazo_core.tag_styles.cut_hatches(s), ingetrazo_core.tag_styles.cut_hatch_grounds(s), ingetrazo_core.tag_styles.cut_hatch_inks(s),
                    ingetrazo_core.tag_styles.cut_fill_colours(s), ingetrazo_core.tag_styles.cut_hatch_forms(s), ingetrazo_core.tag_styles.cut_hatch_forms(s, 50.0),
                    ingetrazo_core.tag_styles.cut_hatch_forms(s, 0)]
        a["unprinted"] = [ly.key for ly in L.unprinted(s)]
        a["offsets"] = [ingetrazo_core.tag_styles.clean_offset(v) for v in
                        (None, (1, 2), (0, 0), (2e5, -3e5), ("a", 1), (1, 2, 3), (math.nan, 1), [5, 0])]
        a["colours"] = [ingetrazo_core.tag_styles.clean_line_colour(v) for v in
                        (None, (2, -1, 0.5), (1, 2), ("x", 0, 0), (math.nan, 0, 0), [0.1, 0.2, 0.3])]
        changed = []
        for key, rgb in (("dashed", (0, 1, 0)), ("dashed", None), ("lin:mine", (1, 1, 0)),
                         ("lin:mine", None), ("lin:gone", (1, 0, 0)), ("nope", (1, 0, 0)),
                         ("dot", (0.5, 0.5, 0.5)), ("dot", (0.5, 0.5, 0.5))):
            changed.append([ingetrazo_core.tag_styles.set_linetype_color(s, key, rgb), dict(s.linetype_colors),
                            dict(s.line_types)])
        a["set_colour"] = changed
        ingetrazo_core.tag_styles.load_linetype_colors(s, {"dashed": [0.2, 0.2, 0.2], "nope": [1, 1, 1], "dot": "x"})
        ingetrazo_core.tag_styles.load_linetype_colors(s, None)
        ingetrazo_core.tag_styles.load_bricks(s, {"brick": (230, 110, 50, 10), "block": "x", "nope": (1, 2, 3, 4)})
        ingetrazo_core.tag_styles.load_bricks(s, [1])
        a["loaded"] = [dict(s.linetype_colors), dict(s.bricks)]
        ingetrazo_core.tag_styles.hide_unprinted(s)
        a["hidden"] = [ly.visible for ly in s.layers]
        out.append(_j(a))
    return out


def test_the_tag_styles_answer_as_the_python_did():
    from core import layers
    want = json.loads(gzip.decompress(GOLDEN.read_bytes()))
    got = json.loads(json.dumps(answers(layers)))
    assert len(got) == len(want)
    wrong = [f"case {i} {k}" for i, (g, w) in enumerate(zip(got, want)) for k in w if g[k] != w[k]]
    assert not wrong, f"{len(wrong)} answers differ: {wrong[:20]}"


if __name__ == "__main__" and "--write" in sys.argv:
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    from core import layers
    data = answers(layers)
    GOLDEN.parent.mkdir(exist_ok=True)
    GOLDEN.write_bytes(gzip.compress(json.dumps(data).encode(), mtime=0))
    print(f"wrote {GOLDEN} ({len(data)} cases)")
