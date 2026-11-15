# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Per-layer cut weight, hatch and line type (#926) follow the tag's KEY.

Geometry carries ``Layer.key``; ``Layer.name`` is only a label, which a
rename changes and two tags in different folders may share. Every map the
sheets, the DXF and the 3D view read a tag's look from is keyed by the key
the segments and rings carry — keyed by label, a renamed tag lost its look
and two «Corte» tags got one look between them."""
from types import SimpleNamespace

import numpy as np
import pytest

from core import layers as L
from core.layers import Layer
from core.scene import Scene


@pytest.fixture
def scene():
    sc = Scene()
    # A tag renamed after it was made: its key stays «Muros».
    walls = Layer("Muros", hatch="brick", cut_weight=0.7, linetype="dashed",
                  color=(0.8, 0.2, 0.2))
    sc.layers.append(walls)
    assert L.rename_layer(sc, "Muros", "Paredes")
    # Two tags with one label, in two folders.
    north = L.add_folder(sc, "Norte")
    south = L.add_folder(sc, "Sur")
    a = Layer("Corte", key="corte-norte", folder=north, hatch="concrete",
              cut_weight=0.35, linetype="dotted", color=(0.1, 0.6, 0.2))
    b = Layer("Corte", key="corte-sur", folder=south, hatch="insulation",
              cut_weight=0.18, linetype="center", hatch_angle=45.0,
              hatch_ground=(1.0, 1.0, 1.0), hatch_color=(0.0, 0.0, 1.0),
              color=(0.2, 0.2, 0.9))
    sc.layers.extend([a, b])
    return sc


KEYS = ("Muros", "corte-norte", "corte-sur")


def test_the_setup_is_what_it_says(scene):
    walls = scene.layer_by_key("Muros")
    assert walls.name == "Paredes" and walls.key == "Muros"
    a, b = scene.layer_by_key("corte-norte"), scene.layer_by_key("corte-sur")
    assert a.name == b.name == "Corte" and a.folder != b.folder


def test_cut_hatches_and_their_looks_are_keyed_by_key(scene):
    assert L.cut_hatches(scene) == {"Muros": "brick",
                                    "corte-norte": "concrete",
                                    "corte-sur": "insulation"}
    forms = L.cut_hatch_forms(scene, 50.0)
    assert set(forms) == set(KEYS)
    assert forms["corte-sur"]["angle"] == 45.0
    assert forms["corte-norte"]["angle"] == 0.0
    assert set(L.cut_hatch_grounds(scene)) == {"corte-sur"}
    assert L.cut_hatch_inks(scene)["corte-sur"] == (0.0, 0.0, 1.0)
    fills = L.cut_fill_colours(scene)
    assert {"Muros", "corte-norte", "corte-sur"} <= set(fills)
    assert fills["corte-norte"] != fills["corte-sur"]
    for name in ("Paredes", "Corte"):            # no label leaks in as a key
        for table in (L.cut_hatches(scene), forms, fills):
            assert name not in table


def test_line_styles_carry_each_tags_own_pen_and_line(scene):
    styles = L.line_styles(scene, True)
    assert styles["Muros"][1] == "dashed" and styles["Muros"][3] == 0.7
    assert styles["corte-norte"][1] == "dotted"
    assert styles["corte-norte"][3] == 0.35
    assert styles["corte-sur"][1] == "center"
    assert styles["corte-sur"][3] == 0.18
    assert "Paredes" not in styles and "Corte" not in styles
    model = L.model_line_styles(scene)
    assert set(model) == set(KEYS)
    assert model["corte-norte"][1] != model["corte-sur"][1]


def test_the_view_inks_each_cut_with_its_tags_weight(scene):
    from views.viewport import Viewport
    from core.style import Style
    keep = np.array([True, True, True, True])
    tags = np.array(["Muros", "corte-norte", "corte-sur", ""], dtype=object)
    vp = SimpleNamespace(scene=scene, _frame_style=Style(),
                         _section_cut_cache=(None, None, tags),
                         CUT_PEN_MM=Viewport.CUT_PEN_MM)
    plane = SimpleNamespace(fill={"pattern": "layers"})
    got = Viewport._cut_edge_weights(vp, plane, keep)
    assert got.tolist() == pytest.approx([1.4, 0.7, 0.4, 1.0])


def test_the_legend_and_the_dxf_names_tell_shared_labels_apart(scene):
    rows, swatches = L.legend_rows(scene)
    assert "Paredes" in rows
    north = L.path_label(scene, "corte-norte")
    south = L.path_label(scene, "corte-sur")
    assert north != south and north in rows and south in rows
    assert swatches[north]["hatch"] == "concrete"
    assert swatches[south]["hatch"] == "insulation"
    assert swatches[south]["hatch_angle"] == 45.0
    assert swatches[north]["linetype"] == "dotted"
    assert swatches[south]["linetype"] == "center"
    labels = L.export_labels(scene)
    assert labels["corte-norte"] != labels["corte-sur"]
    assert labels["Muros"] == "Paredes"


def test_the_dxf_groups_go_out_per_tag_under_its_label(scene):
    from core.hlr import KIND_EDGE
    from views.composer import dxf_groups_by_layer
    segs = np.array([[0, 0, 1, 0], [0, 1, 1, 1], [0, 2, 1, 2]], float)
    drawing = SimpleNamespace(
        segs=segs, kinds=np.array([KIND_EDGE] * 3),
        layers=np.array(list(KEYS), dtype=object),
        styles=L.line_styles(scene, True), labels=L.export_labels(scene))
    groups = dxf_groups_by_layer(drawing, "VISTA", hidden_lines=False)
    by_name = {g[0]: g for g in groups if len(g[1])}
    north = L.path_label(scene, "corte-norte")
    south = L.path_label(scene, "corte-sur")
    assert {"Paredes", north, south} <= set(by_name)
    assert by_name[north][2] != by_name[south][2]       # their own line types
