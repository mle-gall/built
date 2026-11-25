# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The section cut by layer on the sheets (#926): every cut chord and every
poché ring knows the layer of the solid it cuts, so a wall's cut can take a
heavier pen than the door beside it (the layer's cut weight) and each solid
can be filled with its layer's colour."""
from __future__ import annotations

import ingetrazo_core
import json
import math
import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import numpy as np
import pytest
from ingetrazo_core import Vec3 as QVector3D


from core import layers as L
from core.camera import OrbitCamera
from core.composition import MarcoVista, apply_frame_camera
from core.group import Group
from core.hlr import KIND_CUT, KIND_EDGE, hlr_drawing
from core.layers import DEFAULT_LAYER, Layer
from core.mesh import Mesh
from core.scene import Scene
from ingetrazo_core.section import SectionPlane

V = QVector3D


def _box(m, x0, y0, z0, x1, y1, z1):
    """A closed axis-aligned box of six faces."""
    m.add_face([V(x0, y0, z0), V(x1, y0, z0), V(x1, y1, z0), V(x0, y1, z0)])
    m.add_face([V(x0, y0, z1), V(x1, y0, z1), V(x1, y1, z1), V(x0, y1, z1)])
    m.add_face([V(x0, y0, z0), V(x1, y0, z0), V(x1, y0, z1), V(x0, y0, z1)])
    m.add_face([V(x0, y1, z0), V(x1, y1, z0), V(x1, y1, z1), V(x0, y1, z1)])
    m.add_face([V(x0, y0, z0), V(x0, y1, z0), V(x0, y1, z1), V(x0, y0, z1)])
    m.add_face([V(x1, y0, z0), V(x1, y1, z0), V(x1, y1, z1), V(x1, y0, z1)])


def _wall_and_door(scene):
    """A wall (layer «Muros», heavy cut) and a door leaf beside it
    («Carpinterías»), cut by a plan plane at 1 m, plus a loose box on the
    default layer."""
    scene.layers += [Layer("Muros", color=(0.8, 0.1, 0.1), cut_weight=0.7),
                     Layer("Carpinterías", color=(0.1, 0.1, 0.8))]
    wall = Group(Mesh(), name="muro")
    _box(wall.mesh, 0, 0, 0, 4, 0.3, 3)
    wall.layer = "Muros"
    door = Group(Mesh(), name="puerta")
    _box(door.mesh, 5, 0, 0, 6, 0.1, 2)
    door.layer = "Carpinterías"
    scene.groups += [wall, door]
    _box(scene.mesh, 8, 0, 0, 9, 1, 2)
    sp = SectionPlane(V(0, 0, 1), V(0, 0, 1))
    scene.section_planes.append(sp)
    scene.set_active_section(sp)
    return scene


def _top_camera(scene):
    cam = OrbitCamera()
    f = MarcoVista(view_key="std:top", scale_n=100.0, w_mm=200.0, h_mm=100.0)
    apply_frame_camera(cam, f, saved_view=None, scene=scene)
    return cam


# ---- The layer record ----------------------------------------------------

def test_the_cut_weight_round_trips_and_makes_the_layer_worth_saving():
    ly = Layer("Muros", cut_weight=0.7)
    raw = json.loads(json.dumps(ly.to_dict()))
    assert raw["cut_weight"] == 0.7
    assert Layer.from_dict(raw).cut_weight == 0.7
    assert ly.has_style()
    assert "cut_weight" not in Layer("Plain").to_dict()


@pytest.mark.parametrize("raw, expected", [
    ({"name": "X", "cut_weight": "thick"}, None),
    ({"name": "X", "cut_weight": 99}, 5.0),
    ({"name": "X", "cut_weight": 0}, 0.05),
    ({"name": "X"}, None),
])
def test_a_malformed_cut_weight_loads_kept_in_range(raw, expected):
    assert Layer.from_dict(raw).cut_weight == expected


def test_a_template_carries_the_cut_weight():
    src = Scene()
    src.layers.append(Layer("Muros", cut_weight=0.5))
    data = json.loads(json.dumps(ingetrazo_core.tag_edits.template_of(src)))
    dst = Scene()
    dst.layers.append(Layer("Muros"))
    ingetrazo_core.tag_edits.apply_template(dst, data)
    assert dst.layer("Muros").cut_weight == 0.5


def test_a_cut_weight_alone_asks_for_lines_by_layer():
    scene = Scene()
    scene.layers.append(Layer("Muros"))
    assert not ingetrazo_core.tag_styles.layer_lines_wanted(scene)
    scene.layer("Muros").cut_weight = 0.5
    assert ingetrazo_core.tag_styles.layer_lines_wanted(scene)
    assert ingetrazo_core.tag_styles.line_styles(scene, colored=False)["Muros"] == (
        None, "continuous", None, 0.5, (), None)


def test_the_fill_colours_leave_the_default_layer_to_the_frame():
    scene = Scene()
    scene.layers += [Layer("Muros", color=(0.8, 0.1, 0.1)), Layer("Auto")]
    table = ingetrazo_core.tag_styles.cut_fill_colours(scene)
    assert DEFAULT_LAYER not in table
    assert table["Muros"] == pytest.approx((0.8, 0.1, 0.1))
    assert table["Auto"] == ingetrazo_core.tag_styles.layer_color(scene.layer("Auto"))


# ---- The hidden-line pass ------------------------------------------------

def test_every_cut_chord_and_ring_carries_its_solid_layer():
    scene = _wall_and_door(Scene())
    d = hlr_drawing(scene, _top_camera(scene), profiles=False, layers=True)
    cut_tags = {t for k, t in zip(d.kinds, d.layers) if k == KIND_CUT}
    assert cut_tags == {"Muros", "Carpinterías", DEFAULT_LAYER}
    assert len(d.loop_layers) == len(d.loops) == 3
    assert sorted(d.loop_layers) == sorted(
        ["Muros", "Carpinterías", DEFAULT_LAYER])
    # each ring lies over its own solid: the wall's is the 4 m one
    widths = {tag: float(np.ptp(ring[:, 0]))
              for ring, tag in zip(d.loops, d.loop_layers)}
    assert widths["Muros"] == pytest.approx(4.0)
    assert widths["Carpinterías"] == pytest.approx(1.0)


def test_without_layers_the_drawing_is_the_same_and_untagged():
    scene = _wall_and_door(Scene())
    cam = _top_camera(scene)
    plain = hlr_drawing(scene, cam, profiles=False)
    tagged = hlr_drawing(scene, cam, profiles=False, layers=True)
    assert plain.layers is None and plain.loop_layers is None
    assert len(plain.segs) == len(tagged.segs)
    assert len(plain.loops) == len(tagged.loops)
    assert (int((plain.kinds == KIND_CUT).sum())
            == int((tagged.kinds == KIND_CUT).sum()))


def test_no_fills_asked_means_no_ring_layers_either():
    scene = _wall_and_door(Scene())
    d = hlr_drawing(scene, _top_camera(scene), profiles=False, fills=False,
                    layers=True)
    assert d.loops == [] and d.loop_layers == []


def test_a_layer_set_on_a_face_inside_the_object_wins():
    scene = _wall_and_door(Scene())
    scene.layers.append(Layer("Revestimiento"))
    wall = scene.groups[0]
    for f in wall.mesh.faces:
        L.assign_layer(f, "Revestimiento")
    d = hlr_drawing(scene, _top_camera(scene), profiles=False, layers=True)
    assert "Revestimiento" in d.loop_layers
    assert "Muros" not in d.loop_layers


# ---- Pens ------------------------------------------------------------------


# ---- Fills -----------------------------------------------------------------


def test_the_frame_keeps_its_fill_by_layer_switch():
    from core.composition import Composicion
    comp = Composicion()
    comp.frames.append(MarcoVista(cut_fill_by_layer=True))
    back = Composicion.from_dict(json.loads(json.dumps(comp.to_dict())))
    assert back.frames[0].cut_fill_by_layer is True
    assert MarcoVista().cut_fill_by_layer is False


# ---- On the sheet ----------------------------------------------------------


# ---- The Layers panel ------------------------------------------------------

def test_drawing_sets_the_cut_weight():
    from tests.test_project_drawing_filters import Project, drawing_edit
    p = Project(Layer("A"))
    drawing_edit(p, "drawingpatch", [["A"], {"cut_weight": "0.50"}])
    assert p.scene.layer("A").cut_weight == 0.5


# ---- Hatches by layer --------------------------------------------------------

def test_every_hatch_has_a_name_and_marks_inside_any_box():
    from core.hatches import HATCH_LABELS, HATCH_ORDER, hatch_marks
    assert set(HATCH_ORDER) == set(HATCH_LABELS)
    for key in HATCH_ORDER:
        marks = hatch_marks(key, (0.0, 0.0, 20.0, 10.0))
        n = sum(len(v) for v in marks.values())
        assert (n == 0) == (key == "solid"), key
    assert hatch_marks("nonsense", (0, 0, 10, 10))["lines"] == []


def test_a_hatch_is_locked_to_the_paper_not_to_the_box():
    """Two pieces of one wall side by side: the pattern runs on unbroken —
    the courses of a small box are courses of a bigger one."""
    from core.hatches import hatch_marks
    small = hatch_marks("brick", (0.0, 0.0, 6.0, 6.0))["lines"]
    big = hatch_marks("brick", (-10.0, -10.0, 20.0, 20.0))["lines"]
    rows_small = {round(s[1], 6) for s in small if abs(s[1] - s[3]) < 1e-9}
    rows_big = {round(s[1], 6) for s in big if abs(s[1] - s[3]) < 1e-9}
    assert rows_small and rows_small <= rows_big
    assert 1.5 in rows_small


def test_the_hatch_scales_with_the_frame_spacing():
    from core.hatches import hatch_marks
    box = (0.0, 0.0, 30.0, 30.0)
    assert (len(hatch_marks("diagonal", box, 2.0)["lines"])
            < len(hatch_marks("diagonal", box, 1.0)["lines"]))


def test_the_hatch_round_trips_and_an_unknown_one_is_dropped():
    ly = Layer("Muros", hatch="brick")
    raw = json.loads(json.dumps(ly.to_dict()))
    assert raw["hatch"] == "brick" and ly.has_style()
    assert Layer.from_dict(raw).hatch == "brick"
    assert Layer.from_dict({"name": "X", "hatch": "zigzag"}).hatch is None
    assert Layer.from_dict({"name": "X", "hatch": 3}).hatch is None
    assert "hatch" not in Layer("Plain").to_dict()


def test_a_template_and_the_legend_carry_the_hatch():
    src = Scene()
    src.layers.append(Layer("Aislante", hatch="insulation"))
    dst = Scene()
    dst.layers.append(Layer("Aislante"))
    ingetrazo_core.tag_edits.apply_template(dst, json.loads(json.dumps(ingetrazo_core.tag_edits.template_of(src))))
    assert dst.layer("Aislante").hatch == "insulation"
    _rows, swatches = L.legend_rows(src)
    assert swatches["Aislante"]["hatch"] == "insulation"
    assert "hatch" not in swatches[DEFAULT_LAYER]


# ---- A solid ground under the hatch ---------------------------------------

def test_the_hatch_ground_round_trips_as_a_colour_or_the_layer():
    grey = Layer("Muros", hatch="brick", hatch_ground=(0.7, 0.7, 0.7))
    raw = json.loads(json.dumps(grey.to_dict()))
    assert raw["hatch_ground"] == [0.7, 0.7, 0.7]
    assert Layer.from_dict(raw).hatch_ground == (0.7, 0.7, 0.7)
    own = Layer("Aislante", color=(1.0, 0.8, 0.0), hatch_ground="layer")
    assert Layer.from_dict(own.to_dict()).hatch_ground == "layer"
    assert own.has_style()
    for bad in ("purple", [1, 2], 7, None):
        assert Layer.from_dict({"name": "X", "hatch_ground": bad}
                               ).hatch_ground is None


def test_the_layer_ground_is_its_colour_made_light():
    ly = Layer("A", color=(0.8, 0.0, 0.0), hatch="brick",
               hatch_ground="layer")
    r, g, b = ingetrazo_core.tag_styles.hatch_ground_rgb(ly)
    assert r > g and g == pytest.approx(0.75) and r > 0.9
    assert ingetrazo_core.tag_styles.hatch_ground_rgb(Layer("B")) is None


def test_only_hatched_layers_give_their_ground_to_the_sheet():
    scene = Scene()
    scene.layers += [Layer("A", hatch="brick", hatch_ground=(1, 1, 1)),
                     Layer("B", hatch_ground=(0.5, 0.5, 0.5))]
    assert ingetrazo_core.tag_styles.cut_hatch_grounds(scene) == {"A": (1.0, 1.0, 1.0)}
    _rows, swatches = L.legend_rows(scene)
    assert swatches["A"]["hatch_ground"] == [1.0, 1.0, 1.0]
    assert "hatch_ground" not in swatches["B"]


def test_a_template_carries_the_ground():
    src = Scene()
    src.layers.append(Layer("Muros", hatch="brick", hatch_ground="layer"))
    dst = Scene()
    dst.layers.append(Layer("Muros"))
    ingetrazo_core.tag_edits.apply_template(dst, json.loads(json.dumps(ingetrazo_core.tag_edits.template_of(src))))
    assert dst.layer("Muros").hatch_ground == "layer"


# ---- The ink of the hatch: as the frame says, by layer, or fixed -----------

def test_the_hatch_ink_round_trips_by_layer_or_fixed():
    fixed = Layer("Muros", hatch="brick", hatch_color=(0.0, 0.0, 0.0))
    raw = json.loads(json.dumps(fixed.to_dict()))
    assert raw["hatch_color"] == [0.0, 0.0, 0.0]
    assert Layer.from_dict(raw).hatch_color == (0.0, 0.0, 0.0)
    by_layer = Layer("A", hatch="brick", hatch_color="layer")
    assert Layer.from_dict(by_layer.to_dict()).hatch_color == "layer"
    assert Layer("Plain", hatch_color="nope").hatch_color is None
    assert "hatch_color" not in Layer("Plain").to_dict()


def test_by_layer_is_the_layer_colour_a_shade_darker():
    ly = Layer("A", color=(1.0, 0.5, 0.0), hatch="brick", hatch_color="layer")
    assert ingetrazo_core.tag_styles.hatch_ink_rgb(ly) == pytest.approx((0.7, 0.35, 0.0))
    ly.color = (0.0, 0.0, 1.0)                     # follows the layer
    assert ingetrazo_core.tag_styles.hatch_ink_rgb(ly) == pytest.approx((0.0, 0.0, 0.7))
    assert ingetrazo_core.tag_styles.hatch_ink_rgb(Layer("B")) is None


def test_only_hatched_layers_give_their_ink_and_the_legend_shows_it():
    scene = Scene()
    scene.layers += [Layer("A", hatch="brick", hatch_color=(0, 0, 0)),
                     Layer("B", hatch_color=(0, 0, 0))]
    assert ingetrazo_core.tag_styles.cut_hatch_inks(scene) == {"A": (0.0, 0.0, 0.0)}
    _rows, swatches = L.legend_rows(scene)
    assert swatches["A"]["hatch_color"] == [0.0, 0.0, 0.0]


# ---- The catalogue -----------------------------------------------------------

def test_every_pattern_sits_in_exactly_one_group():
    from core.hatches import HATCH_GROUPS, HATCH_LABELS, HATCH_ORDER
    listed = [k for _t, keys in HATCH_GROUPS for k in keys]
    assert sorted(listed) == sorted(HATCH_LABELS)
    assert len(listed) == len(set(listed)) == len(HATCH_ORDER)


def test_the_first_patterns_keep_their_names():
    """Documents saved with the first set still load their hatches."""
    from core.hatches import known
    for key in ("solid", "diagonal", "crosshatch", "brick", "block",
                "concrete", "reinforced", "insulation", "earth", "gravel",
                "sand", "stone", "wood", "steel"):
        assert known(key)
        assert Layer.from_dict({"name": "X", "hatch": key}).hatch == key


def test_every_pattern_marks_inside_its_box_and_tells_apart():
    """Each pattern leaves marks near the box (the clip trims the rest),
    and no two patterns draw the very same marks."""
    from core.hatches import HATCH_ORDER, SAME_FACE, hatch_marks
    box = (0.0, 0.0, 12.0, 8.0)
    seen = {}
    for key in HATCH_ORDER:
        if key == "solid" or key in SAME_FACE:   # these part in a cut
            continue
        marks = hatch_marks(key, box)
        pts = [p for seg in marks["lines"] for p in (seg[:2], seg[2:])]
        pts += marks["dots"] + [c[:2] for c in marks["circles"]]
        pts += [p for poly in marks["polys"] + marks["closed"] for p in poly]
        assert pts, key
        inside = [p for p in pts if -1 <= p[0] <= 13 and -1 <= p[1] <= 9]
        assert inside, key
        sig = repr(sorted((round(a, 3), round(b, 3)) for a, b in pts)[:60])
        assert sig not in seen, (key, seen.get(sig))
        seen[sig] = key


# ---- Angle, scale, size on paper or with the drawing, imported patterns -----

_PAT = """;%UNITS=MM
*TEST-HEX, hexagons
0, 0,0, 4.7625,2.74963, 3.175,-6.35
120, 0,0, 4.7625,2.74963, 3.175,-6.35
60, 3.175,0, 4.7625,2.74963, 3.175,-6.35
*BROKEN, only bad lines
1, 2
45, 0,0, 0,0
*TOO-FINE
0, 0,0, 0,0.00001
"""


def test_a_pattern_file_is_read_and_bad_lines_are_dropped():
    from core.hatches import parse_pat
    found = parse_pat(_PAT)
    assert set(found) == {"TEST-HEX"}
    assert found["TEST-HEX"]["label"] == "hexagons"
    assert len(found["TEST-HEX"]["families"]) == 3
    inch = parse_pat(";%UNITS=INCH\n*A\n45, 0,0, 0,0.125\n")
    assert inch["A"]["families"][0][4] == pytest.approx(3.175)


def test_turning_a_pattern_turns_its_lines():
    from core.hatches import hatch_marks
    flat = hatch_marks("grid", (0, 0, 10, 10))["lines"]
    turned = hatch_marks("grid", (0, 0, 10, 10), angle=45.0)["lines"]
    def slopes(lines):
        import math
        return {round(math.degrees(math.atan2(y1 - y0, x1 - x0)) % 180)
                for x0, y0, x1, y1 in lines if (x1 - x0) or (y1 - y0)}
    assert slopes(flat) == {0, 90}
    assert slopes(turned) == {45, 135}


def test_the_angle_scale_and_size_round_trip_on_the_layer():
    ly = Layer("Baldosas", hatch="grid", hatch_angle=30, hatch_scale=2.5,
               hatch_on_paper=False)
    back = Layer.from_dict(json.loads(json.dumps(ly.to_dict())))
    assert (back.hatch_angle, back.hatch_scale, back.hatch_on_paper) == (
        30.0, 2.5, False)
    assert ly.has_style()
    plain = Layer("X").to_dict()
    assert not {"hatch_angle", "hatch_scale", "hatch_on_paper"} & set(plain)
    odd = Layer.from_dict({"name": "X", "hatch_angle": "nan",
                           "hatch_scale": 999})
    assert odd.hatch_angle == 0.0 and odd.hatch_scale == 20.0


def test_a_hatch_that_grows_with_the_drawing_follows_the_frame_scale():
    scene = Scene()
    scene.layers += [Layer("Papel", hatch="grid", hatch_scale=2.0),
                     Layer("Dibujo", hatch="grid", hatch_on_paper=False)]
    at_50 = ingetrazo_core.tag_styles.cut_hatch_forms(scene, 50.0)
    at_200 = ingetrazo_core.tag_styles.cut_hatch_forms(scene, 200.0)
    assert at_50["Papel"]["scale"] == at_200["Papel"]["scale"] == 2.0
    assert at_50["Dibujo"]["scale"] == pytest.approx(2.0)   # twice at 1:50
    assert at_200["Dibujo"]["scale"] == pytest.approx(0.5)  # half at 1:200


def test_a_material_is_drawn_at_its_real_size_by_itself():
    """A brick's course is 6.5 cm in reality: 6.5 mm on paper at 1:10,
    1.3 mm at 1:50 — whatever the sheet, with no box to tick; a stipple
    keeps its size on paper; the layer can force either."""
    from core.hatches import REAL_SIZE, real_scale
    unit, metres = REAL_SIZE["brick"]
    for n in (10, 20, 50, 100):
        assert unit * real_scale("brick", n) == pytest.approx(
            metres * 1000.0 / n)
    scene = Scene()
    scene.layers += [Layer("Muro", hatch="brick"),
                     Layer("Losa", hatch="concrete"),
                     Layer("Muro fijo", hatch="brick", hatch_on_paper=True)]
    at_10 = ingetrazo_core.tag_styles.cut_hatch_forms(scene, 10.0)
    at_50 = ingetrazo_core.tag_styles.cut_hatch_forms(scene, 50.0)
    assert at_10["Muro"]["scale"] == pytest.approx(5 * at_50["Muro"]["scale"])
    assert at_10["Losa"]["scale"] == at_50["Losa"]["scale"] == 1.0
    assert at_10["Muro fijo"]["scale"] == 1.0
    assert ingetrazo_core.tag_styles.hatch_grows(scene.layer("Muro"))
    assert not ingetrazo_core.tag_styles.hatch_grows(scene.layer("Losa"))
    # far too small a scale: not a grey blur
    assert real_scale("brick", 2000) == pytest.approx(0.25)
    assert real_scale("concrete", 10) is None


def test_the_size_rule_is_saved_only_when_the_layer_chose_it():
    assert "hatch_on_paper" not in Layer("A", hatch="brick").to_dict()
    for chosen in (True, False):
        ly = Layer("A", hatch="brick", hatch_on_paper=chosen)
        assert Layer.from_dict(json.loads(json.dumps(ly.to_dict()))
                               ).hatch_on_paper is chosen
    assert Layer.from_dict({"name": "A", "hatch_on_paper": "x"}
                           ).hatch_on_paper is None


def test_imported_patterns_live_in_the_document(tmp_path):
    from core.hatches import parse_pat
    from formats import igz
    scene = Scene()
    scene.hatch_patterns = parse_pat(_PAT)
    scene.layers.append(Layer("Muro", hatch="pat:TEST-HEX"))
    assert ingetrazo_core.tag_styles.hatch_families(scene, "pat:TEST-HEX")
    assert ingetrazo_core.tag_styles.hatch_families(scene, "brick") is None
    path = tmp_path / "hex.igz"
    igz.save_scene(scene, path)
    back = Scene()
    igz.load_into(back, path)
    assert set(back.hatch_patterns) == {"TEST-HEX"}
    assert back.layer("Muro").hatch == "pat:TEST-HEX"
    assert len(back.hatch_patterns["TEST-HEX"]["families"]) == 3
    fresh = Scene()
    igz.load_into(back, tmp_path / "hex.igz")      # no leak on reopen
    igz.save_scene(fresh, tmp_path / "empty.igz")
    igz.load_into(back, tmp_path / "empty.igz")
    assert back.hatch_patterns == {}


# ---- The catalogue's entries, the pattern file out, the frame's hatch pen ---

def test_every_pattern_has_its_catalogue_entry():
    from core.hatches import HATCH_INFO, HATCH_LABELS
    assert set(HATCH_INFO) == set(HATCH_LABELS)
    aliases = [info[0] for info in HATCH_INFO.values()]
    assert len(aliases) == len(set(aliases))          # no two codes alike
    for key, (alias, density, scale, angle, use) in HATCH_INFO.items():
        assert alias.replace("-", "").isalnum() and alias.isupper(), key
        assert density in ("low", "medium", "high"), key
        assert scale > 0 and 0 <= angle < 360 and use, key


def _inked(key, families=None, size=(120, 90), scale=10.0, shift=(0.37, 0.29)):
    """The pattern's marks inside a 12 × 9 mm box, inked one pixel wide on
    a ``size`` grid at ``scale`` px/mm, shifted off the pixel grid (a line
    lying exactly between two pixels lands on either for a millionth of a
    millimetre): the test's own drawing, the comparison is of the marks."""
    from core.hatches import hatch_marks
    w, h = size
    box = (0.0, 0.0, w / scale, h / scale)
    marks = hatch_marks(key, box, 1.0, 0.0, families, None)
    ink = np.zeros((h, w), bool)

    def seg(a, b):
        ax, ay = a[0] * scale + shift[0], a[1] * scale + shift[1]
        bx, by = b[0] * scale + shift[0], b[1] * scale + shift[1]
        n = int(max(abs(bx - ax), abs(by - ay)) * 2) + 2
        t = np.linspace(0.0, 1.0, n)
        x = np.floor(ax + (bx - ax) * t).astype(int)
        y = np.floor(ay + (by - ay) * t).astype(int)
        keep = (x >= 0) & (x < w) & (y >= 0) & (y < h)
        ink[y[keep], x[keep]] = True

    for x0, y0, x1, y1 in marks["lines"]:
        seg((x0, y0), (x1, y1))
    for pts in marks["polys"]:
        for a, b in zip(pts, pts[1:]):
            seg(a, b)
    for pts in marks["closed"]:
        for a, b in zip(pts, pts[1:] + pts[:1]):
            seg(a, b)
    for dot in marks.get("dots", ()):
        seg(dot[:2], dot[:2])
    for x, y, r in marks["circles"]:
        ring = [(x + r * math.cos(t), y + r * math.sin(t))
                for t in np.linspace(0.0, 2 * math.pi, 33)]
        for a, b in zip(ring, ring[1:]):
            seg(a, b)
    return ink


def test_a_line_pattern_goes_to_a_pattern_file_and_back_unchanged():
    """Out to .pat and read again, every line pattern draws the same — the
    herringbone's built-in turn included (compared as drawn: six decimals
    in the file may move a line that the clip throws away anyway)."""
    from core.hatches import HATCH_ORDER, line_families, parse_pat, to_pat
    keys = [k for k in HATCH_ORDER if line_families(k)]
    assert {"brick", "brick_flemish", "hexagon", "parquet"} <= set(keys)
    assert line_families("concrete") is None          # a stipple: no .pat
    back = parse_pat(to_pat([(k, k, line_families(k)) for k in keys]))
    for key in keys:
        a, b = _inked(key), _inked("pat:" + key, back[key]["families"])
        assert a.any(), key
        differ = int((a != b).sum())
        assert differ < 120 * 90 * 0.005, (key, differ)


def test_export_includes_standard_line_patterns_and_imported_patterns(tmp_path):
    from core.history import History
    from core.hatches import line_families, HATCH_ORDER, parse_pat
    from ingetrazo_core import import_hatch_file, export_style_file
    scene = Scene()
    pat = tmp_path / "mine.pat"
    pat.write_text(_PAT, encoding="utf-8")
    assert import_hatch_file(History(scene), pat) == 1
    out = tmp_path / "out.pat"
    n = export_style_file(scene, out, "pat")
    lines = [k for k in HATCH_ORDER if line_families(k)]
    assert n == len(lines) + 1
    back = parse_pat(out.read_text(encoding="utf-8"))
    assert "IT-BRICK" in back and "TEST-HEX" in back
    assert "IT-CONC" not in back                      # stipples stay out


def test_the_frame_keeps_its_hatch_pen():
    from core.composition import Composicion
    comp = Composicion()
    comp.frames.append(MarcoVista(pen_hatch_mm=0.13))
    back = Composicion.from_dict(json.loads(json.dumps(comp.to_dict())))
    assert back.frames[0].pen_hatch_mm == 0.13
    assert MarcoVista().pen_hatch_mm == 0.0


# ---- The soft insulation symbol, fitted to each piece ------------------------

def _band(cx, cy, length, thick, angle):
    a = math.radians(angle)
    u, v = (math.cos(a), math.sin(a)), (-math.sin(a), math.cos(a))
    return [(cx + u[0] * x + v[0] * y, cy + u[1] * x + v[1] * y)
            for x, y in ((-length / 2, -thick / 2), (length / 2, -thick / 2),
                         (length / 2, thick / 2), (-length / 2, thick / 2))]


@pytest.mark.parametrize("angle", [0, 30, 90, 135])
def test_a_piece_is_measured_along_its_length_and_across_its_thickness(angle):
    from core.hatches import fit_band
    origin, along, across, length, thick = fit_band(_band(10, 20, 50, 6,
                                                          angle))
    assert length == pytest.approx(50) and thick == pytest.approx(6)
    a = math.radians(angle)
    assert abs(along[0] * math.cos(a) + along[1] * math.sin(a)) \
        == pytest.approx(1.0)


def test_the_insulation_reads_as_teardrops_up_and_down():
    """Half circles touching against each face, and from the ends of each
    one straight legs to where two half circles meet on the other face —
    teardrops pointing up and down, side by side, from face to face."""
    from core.hatches import batt_marks, _BATT_RADIUS
    polys = batt_marks((0.0, 0.0), (1.0, 0.0), (0.0, 1.0), 40.0, 8.0)
    ys = [y for pts in polys for _x, y in pts]
    assert min(ys) == pytest.approx(0.0, abs=1e-6)
    assert max(ys) == pytest.approx(8.0, abs=1e-6)
    r = 8.0 * _BATT_RADIUS
    legs = [pts for pts in polys if len(pts) == 3]
    for (x0, y0), (x1, y1), (x2, y2) in legs:
        # from the two ends of an arch to the meeting point of two cups
        assert y0 == y2 == pytest.approx(r) and y1 == pytest.approx(8.0 - r)
        assert x1 - x0 == pytest.approx(r) and x2 - x1 == pytest.approx(r)
    arches = [pts for pts in polys if len(pts) > 3 and pts[len(pts) // 2][1]
              < 1e-6]
    cups = [pts for pts in polys if len(pts) > 3 and pts[len(pts) // 2][1]
            > 8.0 - 1e-6]
    assert arches and cups
    centres = sorted((pts[0][0] + pts[-1][0]) / 2 for pts in arches)
    assert centres[1] - centres[0] == pytest.approx(2 * r)   # they touch
    thick = batt_marks((0.0, 0.0), (1.0, 0.0), (0.0, 1.0), 40.0, 16.0)
    assert len(thick) < len(polys)               # bigger, so fewer


# ---- Masonry as it is seen: courses in section, bond along the wall in plan --


# ---- Line types of one's own: made, imported, exported -----------------------

_LIN = """;; a test file
*BORDE_2,Double border __ __ .
A,6,-1.5,6,-1.5,0,-1.5
*GAS,Gas ---- GAS ----
A,.5,-.2,["GAS",STANDARD,S=.1,R=0.0,X=-0.1,Y=-.05],-.25
*BOXES,With a shape
A,.5,-.2,[BOX,ltypeshp.shx,x=-.1,s=.1],-.1
*FENCE,Imperial fence
A,.25,-.1,0,-.1
*BROKEN,no gap
A,5,5
"""


def test_a_line_type_file_is_read_and_its_complex_types_left_out():
    found, skipped = L.parse_lin(_LIN)
    assert set(found) == {"BORDE_2", "FENCE", "GAS"}
    assert found["BORDE_2"]["pattern"] == [6.0, -1.5, 6.0, -1.5, 0.0, -1.5]
    assert found["FENCE"]["pattern"][0] == pytest.approx(6.35)   # inches
    assert skipped == ["BOXES"]                    # a shape: left out
    # The text one: its two gaps one room, the words in it, in mm.
    gas = found["GAS"]
    assert gas["pattern"] == pytest.approx([12.7, -11.43])
    assert gas["text"] == "GAS" and gas["text_at"] == 1
    assert gas["text_size"] == pytest.approx(2.54)
    back, _ = L.parse_lin(L.to_lin(
        [(n, e["label"], e["pattern"]) for n, e in found.items()]))
    assert back["BORDE_2"]["pattern"] == found["BORDE_2"]["pattern"]


def test_a_pattern_must_draw_and_leave_blank():
    assert L.clean_pattern([3, -1]) == [3.0, -1.0]
    for bad in ([], [3, 3], [-1, -2], [float("nan"), -1], "x", [5000, -1]):
        assert L.clean_pattern(bad) is None


def test_the_pen_draws_dash_gap_in_turn():
    assert L.drawn_dashes((6, -1.5, 0, -1.5)) == (6, 1.5, 0, 1.5)
    assert L.drawn_dashes((-1, 4, -1)) == (4, 2)        # leading gap moved
    assert L.drawn_dashes((3, 3, -1)) == (6, 1)         # two dashes joined
    assert L.file_dashes((6, 1.5, 0, 1.5)) == (6, -1.5, 0, -1.5)


def test_a_layer_takes_a_line_type_of_the_document(tmp_path):
    from formats import igz
    scene = Scene()
    ingetrazo_core.tag_edits.load_line_types(scene, {"Borde": {"label": "Borde",
                                        "pattern": [6, -1.5, 0, -1.5]}})
    scene.layers.append(Layer("Lindero", linetype="lin:Borde"))
    assert Layer("X", linetype="zigzag").linetype == "continuous"
    st = ingetrazo_core.tag_styles.line_styles(scene, False)["Lindero"]
    assert st[1] == "lin:Borde" and st[4] == (6.0, 1.5, 0.0, 1.5)
    path = tmp_path / "lin.igz"
    igz.save_scene(scene, path)
    back = Scene()
    igz.load_into(back, path)
    assert back.layer("Lindero").linetype == "lin:Borde"
    assert back.line_types["Borde"]["pattern"] == [6.0, -1.5, 0.0, -1.5]
    igz.save_scene(Scene(), tmp_path / "empty.igz")
    igz.load_into(back, tmp_path / "empty.igz")
    assert back.line_types == {}                  # no leak to the next


def test_a_template_and_the_legend_carry_the_own_line_type():
    src = Scene()
    ingetrazo_core.tag_edits.load_line_types(src, {"Borde": {"label": "Borde",
                                      "pattern": [6, -1.5, 0, -1.5]},
                            "Unused": {"label": "U", "pattern": [1, -1]}})
    src.layers.append(Layer("Lindero", linetype="lin:Borde"))
    data = json.loads(json.dumps(ingetrazo_core.tag_edits.template_of(src)))
    assert set(data["line_types"]) == {"Borde"}  # only the used ones
    dst = Scene()
    ingetrazo_core.tag_edits.apply_template(dst, data)
    assert dst.layer("Lindero").linetype == "lin:Borde"
    assert "Borde" in dst.line_types
    _rows, swatches = L.legend_rows(src)
    assert swatches["Lindero"]["pattern"] == [6.0, 1.5, 0.0, 1.5]


def edit_line(project, op, **fields):
    from views import wgpu_ui
    project.action("editline", op + wgpu_ui.SEP + json.dumps(fields))


def test_drawing_creates_a_valid_custom_line_and_rejects_a_pattern_without_gaps():
    from tests.test_project_drawing_filters import Project
    p = Project(Layer("A"))
    edit_line(p, "save", name="Borde", pieces=[6, -1.5, 0, -1.5], tags=["A"])
    assert p.scene.layer("A").linetype == "lin:Borde"
    edit_line(p, "save", name="Mal", pieces=[5, 5], tags=["A"])
    assert "Mal" not in p.scene.line_types and p.scene.layer("A").linetype == "lin:Borde"


def test_drawing_imports_and_exports_line_type_files(tmp_path, monkeypatch):
    from tests.test_project_drawing_filters import Project
    from ingetrazo_render.shell_files import ShellFileDialog
    p = Project()
    lin = tmp_path / "mine.lin"
    lin.write_text(_LIN, encoding="utf-8")
    monkeypatch.setattr(ShellFileDialog, "getOpenFileName", lambda *_: (str(lin), ""))
    p.action("importlin", "")
    assert "3" in p.win.viewport.status[-1] and "BOXES" in p.win.viewport.status[-1]
    lin.unlink()
    p.action("importlin", "")
    assert set(p.scene.line_types) == {"BORDE_2", "FENCE", "GAS"}
    out = tmp_path / "out.lin"
    monkeypatch.setattr(ShellFileDialog, "getSaveFileName", lambda *_: (str(out), ""))
    p.action("exportlin", "")
    found, _ = L.parse_lin(out.read_text(encoding="utf-8"))
    assert set(found) == {"BORDE_2", "FENCE", "GAS"} and found["GAS"]["text"] == "GAS"


# ---- Bonds: brick by brick, by the wall's thickness and the brick's size -----

def test_english_bond_alternates_headers_and_stretchers():
    from core.hatches import bond_course
    depth, height, rects = bond_course("brick_bond", 0, 215.0, None, 0, 1000)
    assert {(b0, b1) for _a0, _a1, b0, b1, _k in rects} == {(0.0, 225.0)}
    assert rects[1][1] - rects[1][0] == pytest.approx(112.5)   # headers
    _d, _h, rects = bond_course("brick_bond", 1, 215.0, None, 0, 1000)
    assert {(b0, b1) for _a0, _a1, b0, b1, _k in rects} == {
        (0.0, 112.5), (112.5, 225.0)}                         # two wythes
    assert height == pytest.approx(75.0) and depth == pytest.approx(225.0)


def test_one_and_a_half_swaps_its_faces_course_by_course():
    from core.hatches import bond_course
    _d, _h, a = bond_course("brick_one_half", 0, 330.0, None, 0, 500)
    _d, _h, b = bond_course("brick_one_half", 1, 330.0, None, 0, 500)
    assert min(r[3] - r[2] for r in a if r[2] == 0.0) == pytest.approx(112.5)
    assert min(r[3] - r[2] for r in b if r[2] == 0.0) == pytest.approx(225.0)


def test_the_bond_by_thickness_follows_each_wall():
    from core.hatches import bond_course
    for thick, depth in ((60.0, 75.0),        # brick on edge
                         (102.5, 112.5),      # half a brick, stretchers
                         (215.0, 225.0),      # one brick, English
                         (330.0, 337.5)):     # one and a half
        assert bond_course("brick_auto", 0, thick)[0] == pytest.approx(depth)


def test_paved_areas_and_walls_cut_along_show_the_face():
    """No wall: a paved area or a wall cut along shows the bond's face —
    courses the whole width, staggered head joints."""
    from core.hatches import bond_marks
    wall = ((0.0, 0.0), (1.0, 0.0), (0.0, 1.0), 60.0, 2.4)
    assert bond_marks("brick", wall, "plan", 0.5, 100.0)["lines"]
    yard = ((0.0, 0.0), (1.0, 0.0), (0.0, 1.0), 60.0, 30.0)
    face = bond_marks("brick", yard, "plan", 0.5, 100.0)["lines"]
    beds = [ln for ln in face if abs(ln[1] - ln[3]) < 1e-9
            and abs(ln[2] - ln[0]) >= 60.0 - 1e-6]
    assert len(beds) >= 30 * 2 / 1.5 * 0.9 / 2      # a bed every course
    heads = [ln for ln in face if abs(ln[0] - ln[2]) < 1e-9]
    assert heads and all(abs(ln[3] - ln[1]) == pytest.approx(0.75)
                         for ln in heads)          # one course high
    across = bond_marks("brick", ((0.0, 50.0), (0.0, -1.0), (1.0, 0.0),
                                  50.0, 40.0), "vertical", 0.5, 400.0)
    assert across["lines"]                         # a wall cut along
    assert bond_marks("concrete", wall, "plan") is None


def test_the_joints_follow_the_scale():
    from core.hatches import joints_mode
    assert joints_mode(None, 100) == joints_mode(None, 75) == "hatch"
    assert joints_mode(None, 50) == joints_mode(None, 20) == "single"
    assert joints_mode(None, 15) == joints_mode(None, 5) == "double"
    for mode in ("single", "double", "none"):
        assert joints_mode(mode, 200) == mode
    assert joints_mode("zigzag", 50) == "single"


def test_double_joints_part_every_brick_by_its_real_joint():
    from core.hatches import bond_marks, fit_band
    wall = [(0.0, 0.0), (60.0, 0.0), (60.0, 21.5), (0.0, 21.5)]   # 1:10
    marks = bond_marks("brick_bond", fit_band(wall), "plan", 5.0, 10.0,
                       None, "double")
    assert not marks["lines"] and marks["bricks"]
    xs = sorted({round(min(x for x, _y in b), 6) for b in marks["bricks"]})
    gaps = [b - a for a, b in zip(xs, xs[1:])]
    # Headers 112.5 mm on centres, at 1:10 and × 5 the course: 11.25 mm.
    assert gaps[1] == pytest.approx(11.25, abs=0.01)
    one = marks["bricks"][2]
    w = max(x for x, _y in one) - min(x for x, _y in one)
    assert w == pytest.approx(10.25, abs=0.01)     # the brick, the joint off
    sec = bond_marks("brick_bond", ((0.0, 40.0), (0.0, -1.0), (1.0, 0.0),
                                    40.0, 21.5), "vertical", 5.0, 10.0,
                     None, "double")
    hs = sorted({round(min(-y for _x, y in b), 4) for b in sec["bricks"]})
    assert hs[1] - hs[0] == pytest.approx(7.5, abs=0.01)   # course 75 mm


def test_a_layer_keeps_its_joints_and_mortar(tmp_path):
    ly = Layer("Muro", hatch="brick", joints="double", mortar=True)
    back = Layer.from_dict(ly.to_dict())
    assert back.joints == "double" and back.mortar is True
    assert Layer("X", joints="wavy").joints is None
    assert "joints" not in Layer("X", hatch="brick").to_dict()
    scene = Scene()
    scene.layers.append(ly)
    form = ingetrazo_core.tag_styles.cut_hatch_forms(scene, 10.0)["Muro"]
    assert form["joints"] == "double" and form["mortar"] is True


def test_a_gap_of_the_honeycomb_bond_is_crossed():
    from core.hatches import bond_course
    _d, _h, rects = bond_course("brick_honeycomb", 0, 102.5, None, 0, 1000)
    gaps = [r for r in rects if r[4] == "gap"]
    bricks = [r for r in rects if r[4] == "brick"]
    assert gaps and bricks
    assert gaps[0][1] - gaps[0][0] == pytest.approx(112.5)    # half a brick
    assert bricks[0][1] - bricks[0][0] == pytest.approx(225.0)


def test_a_layer_keeps_its_own_brick(tmp_path):
    from core.hatches import clean_brick, real_scale
    from formats import igz
    assert clean_brick((290, 140, 71, 10)) == (290.0, 140.0, 71.0, 10.0)
    for bad in ((100, 200, 50, 10), (290, 140, 71), ("x", 1, 1, 1),
                (290, 140, 71, 99), (float("nan"), 1, 1, 1)):
        assert clean_brick(bad) is None
    ly = Layer("Muros", hatch="brick", brick=(290, 140, 71, 10))
    assert Layer.from_dict(ly.to_dict()).brick == (290.0, 140.0, 71.0, 10.0)
    assert Layer("X", brick=(1, 1, 1, 1)).brick is None
    scene = Scene()
    scene.layers.append(ly)
    path = tmp_path / "brick.igz"
    igz.save_scene(scene, path)
    back = Scene()
    igz.load_into(back, path)
    assert back.layer("Muros").brick == (290.0, 140.0, 71.0, 10.0)
    # A taller course: the pattern grows to it, real at any scale.
    assert real_scale("brick", 10, (290, 140, 71, 10)) == pytest.approx(
        81.0 / 10.0 / 1.5)
    forms = ingetrazo_core.tag_styles.cut_hatch_forms(scene, 10.0)
    assert forms["Muros"]["brick"] == (290.0, 140.0, 71.0, 10.0)
    assert forms["Muros"]["families"]                  # drawn with it
    assert forms["Muros"]["scale_n"] == 10.0


def test_the_brick_families_follow_the_brick():
    from core.hatches import brick_families
    usual = brick_families("brick")
    chile = brick_families("brick", (290, 140, 71, 10))
    assert usual[0][4] == chile[0][4] == 1.5          # the course
    # A stretcher: (length + joint) / (height + joint) courses long.
    assert chile[1][4] * 2 == pytest.approx(1.5 * 300.0 / 81.0)
    assert usual[1][4] * 2 == pytest.approx(1.5 * 225.0 / 75.0)


def test_the_zigzag_insulation_runs_face_to_face():
    from core.hatches import FITTED, fitted_marks
    assert "insulation_zigzag" in FITTED
    (pts,) = fitted_marks("insulation_zigzag", (0.0, 0.0), (1.0, 0.0),
                          (0.0, 1.0), 12.0, 3.0)
    ys = {round(y, 6) for _x, y in pts}
    assert ys == {0.0, 3.0}                      # every corner on a face
    xs = [x for x, _y in pts]
    assert xs[1] - xs[0] == pytest.approx(1.5)   # a V every thickness
    assert min(xs) <= 0.0 and max(xs) >= 12.0
    assert fitted_marks("insulation_zigzag", (0, 0), (1, 0), (0, 1),
                        12.0, 0.0) == []


def test_timber_cut_across_is_crossed_corner_to_corner():
    from core.hatches import FITTED, fit_band, fitted_marks
    assert "wood_cross" in FITTED
    stud = [(10.0, 0.0), (15.0, 0.0), (15.0, 10.0), (10.0, 10.0)]
    a, b = fitted_marks("wood_cross", *fit_band(stud))
    corners = {(10.0, 0.0), (15.0, 0.0), (15.0, 10.0), (10.0, 10.0)}
    ends = {tuple(round(v, 6) for v in q) for q in a + b}
    assert ends == corners                       # the two diagonals
    assert fitted_marks("wood_cross", (0, 0), (1, 0), (0, 1), 5.0, 0.0) == []


def test_a_piece_is_cut_across_when_short_and_stocky():
    from core.hatches import WOOD, cut_across, fit_band
    assert "wood_grain" in WOOD and "wood_cross" not in WOOD
    stud = [(0, 0), (1.5, 0), (1.5, 0.5), (0, 0.5)]        # 15 × 5 cm, 1:10
    board = [(0, 0), (40, 0), (40, 0.5), (0, 0.5)]         # 4 m × 5 cm
    joist = [(0, 0), (0.5, 0), (0.5, 2.5), (0, 2.5)]       # 5 × 25 cm
    assert cut_across(fit_band(stud), 10.0)
    assert cut_across(fit_band(joist), 10.0)
    assert not cut_across(fit_band(board), 10.0)
    assert not cut_across(fit_band(stud), 1000.0)          # 15 m: no stud
    assert not cut_across(None)


# ---- One wall in pieces, and the hatches in the DXF ---------------------------

def test_the_pieces_of_one_wall_share_their_joints():
    """A wall split by a door: the head joints of both pieces fall on one
    grid, whichever way round each piece was traced."""
    from core.hatches import bond_marks, fit_band
    left = [(0.0, 0.0), (30.0, 0.0), (30.0, 2.15), (0.0, 2.15)]
    right = [(70.0, 2.15), (40.0, 2.15), (40.0, 0.0), (70.0, 0.0)]  # reversed
    xs = set()
    for ring in (left, right):
        marks = bond_marks("brick", fit_band(ring), "plan", 5.0, 10.0, None,
                           "single")
        heads = [ln for ln in marks["lines"] if abs(ln[0] - ln[2]) < 1e-9]
        xs |= {round(ln[0] % 22.5, 6) % 22.5 for ln in heads
               if 0.0 <= ln[0] <= 70.0}
    # Stretchers 225 mm at 1:10 = 22.5 mm, in two wythes half a brick apart:
    # every head joint on one of two phases, the same in both pieces.
    assert len(xs) <= 2


def test_the_courses_of_two_pieces_line_up_in_a_section():
    from core.hatches import bond_marks, vertical_band
    a = [(0.0, 0.0), (2.15, 0.0), (2.15, 40.0), (0.0, 40.0)]
    b = [(30.0, 13.0), (32.15, 13.0), (32.15, 40.0), (30.0, 40.0)]
    beds = []
    for ring in (a, b):
        marks = bond_marks("brick", vertical_band(ring), "vertical", 5.0,
                           10.0, None, "single")
        beds.append({round(ln[1] % 7.5, 6) % 7.5 for ln in marks["lines"]
                     if abs(ln[1] - ln[3]) < 1e-9})
    assert beds[0] == beds[1] and len(beds[0]) == 1


def test_clip_keeps_what_is_inside_even_odd():
    from core.hatches import clip_segments
    outer = [(0, 0), (10, 0), (10, 10), (0, 10)]
    hole = [(4, 4), (6, 4), (6, 6), (4, 6)]
    parts = clip_segments([(-5, 5, 15, 5)], [outer, hole])
    spans = sorted((round(min(x0, x1), 6), round(max(x0, x1), 6))
                   for x0, _y0, x1, _y1 in parts)
    assert spans == [(0.0, 4.0), (6.0, 10.0)]
    assert clip_segments([(20, 20, 30, 30)], [outer]) == []


def test_hatch_lines_draw_as_the_sheet_does():
    from core.hatches import hatch_lines
    wall = [(0.0, 0.0), (60.0, 0.0), (60.0, 2.15), (0.0, 2.15)]
    plan = {"scale": 5.0, "cut": "plan", "scale_n": 10.0}
    lines = hatch_lines("brick", [wall], plan)
    assert lines and all(-1e-6 <= min(x0, x1) and max(x0, x1) <= 60 + 1e-6
                         and -1e-6 <= min(y0, y1) and max(y0, y1) <= 2.15
                         + 1e-6 for x0, y0, x1, y1 in lines)
    small = hatch_lines("brick", [wall], dict(plan, scale_n=100.0))
    assert small and all(abs((x1 - x0) - (y1 - y0)) < 1e-6
                         or abs((x1 - x0) + (y1 - y0)) < 1e-6
                         for x0, y0, x1, y1 in small)   # the 45° hatch
    assert hatch_lines("brick", [wall], dict(plan, joints="none")) == []
    assert hatch_lines("solid", [wall], plan) == []
    assert hatch_lines("insulation", [wall], plan)      # fitted, piece-wise
    assert hatch_lines("concrete", [wall], plan)         # its stones


# ---- Timber: the grain from the 3D piece ----------------------------------------

def _timber(scene):
    """A stud standing through a plan cut at 1 m, a beam lying along it
    (cut along its length), and a short block lying flat across it — short
    and stocky in the cut, but cut along its grain too."""
    scene.layers.append(Layer("Madera", hatch="wood_grain",
                              hatch_cross=True))
    for name, box in (("pie", (0.0, 0.0, 0.0, 0.05, 0.15, 2.4)),
                      ("viga", (0.0, 1.0, 0.9, 4.0, 1.1, 1.1)),
                      ("taco", (1.0, 2.0, 0.95, 1.3, 2.1, 1.05))):
        g = Group(Mesh(), name=name)
        _box(g.mesh, *box)
        g.layer = "Madera"
        scene.groups.append(g)
    sp = SectionPlane(V(0, 0, 1), V(0, 0, 1))
    scene.section_planes.append(sp)
    scene.set_active_section(sp)
    return scene


def test_a_solid_runs_through_the_plane_as_deep_as_it_is():
    from core.hlr import hlr_drawing
    scene = _timber(Scene())
    d = hlr_drawing(scene, _top_camera(scene), profiles=False, fills=True,
                    layers=True, depth_layers=["Madera"])
    sizes = []
    for ring, depth in zip(d.loops, d.loop_depths):
        w = float(ring[:, 0].max() - ring[:, 0].min())
        sizes.append((round(w, 3), None if depth is None
                      else round(depth, 3)))
    assert sorted(sizes) == [(0.05, 2.4), (0.3, 0.1), (4.0, 0.2)]
    plain = hlr_drawing(scene, _top_camera(scene), profiles=False,
                        fills=True, layers=True)
    assert plain.loop_depths is None                # not asked: not measured
    assert ingetrazo_core.hlr_solid_depth([(9, 9, 1), (9.1, 9, 1), (9.1, 9.1, 1)],
                       [[(0, 0, 0), (1, 0, 0), (0, 1, 0)]], (0, 0, 1)) is None


def test_the_grain_settles_the_cross_when_the_piece_is_known():
    from core.hatches import cut_across, fit_band
    beam = [(0, 0), (60, 0), (60, 2.5), (0, 2.5)]       # 6 m cut along: 1:10
    stocky = [(0, 0), (1.5, 0), (1.5, 0.5), (0, 0.5)]
    assert not cut_across(fit_band(beam), 10.0)
    assert cut_across(fit_band(beam), 10.0, depth=70.0)   # runs on further
    assert cut_across(fit_band(stocky), 10.0)              # by its shape
    assert not cut_across(fit_band(stocky), 10.0, depth=0.5)


def test_only_the_timber_cut_across_its_grain_is_crossed(monkeypatch):
    """The stud gets the cross; the beam and the block — cut along — their
    grain, though the block is short and stocky in the cut and its shape
    alone would have crossed it."""
    import core.hatches as H
    from core.hlr import hlr_drawing
    scene = _timber(Scene())
    d = hlr_drawing(scene, _top_camera(scene), profiles=False, fills=True,
                    layers=True, depth_layers=["Madera"])
    k = 10.0                                       # paper mm per metre
    rings = [[(x * k, -y * k) for x, y in r] for r in d.loops]
    depths = [None if v is None else v * k for v in d.loop_depths]
    form = {"scale": 1.0, "cut": "plan", "scale_n": 100.0, "cross": True}
    crossed = []
    real = H.fitted_marks

    def spy(key, origin, along, across, length, thick, *a, **kw):
        crossed.append(round(min(length, thick) / k, 3))
        return real(key, origin, along, across, length, thick, *a, **kw)
    monkeypatch.setattr(H, "fitted_marks", spy)
    H.hatch_lines("wood_grain", rings, form, 1.0, depths)
    assert crossed == [0.05]                      # the stud alone
    crossed.clear()
    H.hatch_lines("wood_grain", rings, form, 1.0)  # by shape: the block too
    assert sorted(crossed) == [0.05, 0.1]


# ---- Solid fills and both DXF exports --------------------------------------------

def test_a_region_cuts_into_strips_even_odd():
    from core.hatches import fill_quads
    outer = [(0, 0), (10, 0), (10, 10), (0, 10)]
    hole = [(4, 4), (6, 4), (6, 6), (4, 6)]
    quads = fill_quads([outer, hole])

    def area(q):
        (ax, ay), (bx, by), (cx, cy), (dx, dy) = q
        return ((bx - ax) + (dx - cx)) / 2.0 * (cy - ay)
    assert sum(area(q) for q in quads) == pytest.approx(100.0 - 4.0)
    tri = fill_quads([[(0, 0), (4, 0), (0, 4)]])
    assert sum(area(q) for q in tri) == pytest.approx(8.0)
    assert fill_quads([]) == []


def test_the_dxf_writes_solids_under_the_lines(tmp_path):
    from formats.dxf_out import save_dxf_layers
    path = tmp_path / "solid.dxf"
    save_dxf_layers(path, [("LINEAS", [(0, 0, 1, 1)])],
                    [("RELLENO", [((0, 0), (1, 0), (0, 1), (1, 1))],
                      (0.9, 0.9, 0.9))])
    text = path.read_text(encoding="ascii")
    assert "0\nSOLID\n8\nRELLENO\n" in text
    assert text.index("SOLID\n8") < text.index("LINE\n8")   # under them
    assert "2\nRELLENO\n70\n0\n62\n" in text             # declared


# ---- Line types that write words: GAS, AGUA… ----------------------------------

def test_a_line_type_takes_its_words_in_a_gap_of_their_own():
    pattern, spec = L.with_text([6.0, -1.5], "GAS", 2.5)
    room = L.text_width("GAS", 2.5) + 2.0
    assert pattern == pytest.approx([6.0, -1.5 - room])   # the gap widened
    assert spec == {"text": "GAS", "text_size": 2.5, "text_at": 1}
    # Its middle in the middle of that gap.
    assert L.text_offset(pattern, 1) == pytest.approx(6.0 + (1.5 + room) / 2)
    assert L.with_text([6.0, -1.5], "  ") == ([6.0, -1.5], None)
    assert L.clean_line_text({"text": "X", "text_at": 0}, [6.0, -1.5]) is None
    huge = L.clean_line_text({"text": "AGUA" * 20, "text_size": 99,
                              "text_at": 1}, [6.0, -1.5])
    assert len(huge["text"]) == 24 and huge["text_size"] == 20.0


def test_the_words_fall_where_the_gaps_are_and_read_upright():
    spec = ("GAS", 2.5, 10.0, 20.0)
    marks = L.text_marks((0.0, 0.0), (65.0, 0.0), spec)
    assert [round(x, 6) for x, _y, _a in marks] == [10.0, 30.0, 50.0]
    back = L.text_marks((65.0, 0.0), (0.0, 0.0), spec)
    assert {a for _x, _y, a in back} == {0.0}           # not upside down
    assert L.text_marks((0, 0), (3, 0), spec) == []     # no room for one


def test_a_document_keeps_and_draws_a_line_type_with_words(tmp_path):
    from formats import igz
    scene = Scene()
    pattern, spec = L.with_text([8.0, -2.0], "GAS", 2.0)
    ingetrazo_core.tag_edits.load_line_types(scene, {"Gas": dict({"label": "Gas",
                                           "pattern": pattern}, **spec)})
    scene.layers.append(Layer("Instalaciones", linetype="lin:Gas"))
    path = tmp_path / "gas.igz"
    igz.save_scene(scene, path)
    back = Scene()
    igz.load_into(back, path)
    assert back.line_types["Gas"]["text"] == "GAS"
    words = ingetrazo_core.tag_styles.line_styles(back, False)["Instalaciones"][5]
    assert words[0] == "GAS" and words[1] == 2.0
    assert ingetrazo_core.tag_styles.linetype_text("dashed") is None


# ---- Favourite hatches, and the line type editor -------------------------------

def test_drawing_edits_and_deletes_its_line_types():
    from tests.test_project_drawing_filters import Project
    p = Project(Layer("Gas"))
    edit_line(p, "save", name="Gas", pieces=[8,-2], tags=["Gas"], text="GAS", text_size=2.0)
    entry = p.scene.line_types["Gas"]
    assert entry["pieces"] == [8., -2.] and entry["text"] == "GAS" and entry["pattern"][1] < -2.
    assert ingetrazo_core.tag_styles.linetype_text("lin:Gas", p.scene)[0] == "GAS"
    edit_line(p, "save", original="Gas", name="Gas natural", pieces=[10,-2], text="GN", text_size=2.)
    assert "Gas" not in p.scene.line_types and p.scene.layer("Gas").linetype == "lin:Gas natural"
    edit_line(p, "delete", original="Gas natural")
    assert not p.scene.line_types and p.scene.layer("Gas").linetype == "continuous"


# ---- The document's brick ----------------------------------------------------

def test_the_document_pieces_draw_every_layer_without_its_own(tmp_path):
    from formats import igz
    chile = (290.0, 140.0, 71.0, 10.0)
    block = (390.0, 190.0, 190.0, 10.0)
    scene = Scene()
    scene.layers += [Layer("Muros", hatch="brick"),
                     Layer("Tabiques", hatch="brick_on_edge",
                           brick=(240, 115, 50, 10)),
                     Layer("Bloques", hatch="block")]
    scene.bricks = {"brick": chile, "block": block}
    assert ingetrazo_core.tag_styles.layer_brick(scene, scene.layer("Muros")) == chile
    assert ingetrazo_core.tag_styles.layer_brick(scene, scene.layer("Tabiques"))[0] == 240.0
    assert ingetrazo_core.tag_styles.layer_brick(scene, scene.layer("Bloques")) == block
    forms = ingetrazo_core.tag_styles.cut_hatch_forms(scene, 10.0)
    assert forms["Muros"]["brick"] == chile
    assert forms["Bloques"]["brick"] == block
    path = tmp_path / "brick.igz"
    igz.save_scene(scene, path)
    back = Scene()
    igz.load_into(back, path)
    assert back.bricks == {"brick": chile, "block": block}
    igz.save_scene(Scene(), tmp_path / "plain.igz")
    igz.load_into(back, tmp_path / "plain.igz")
    assert back.bricks == {}                       # no leak to the next
    data = json.loads(json.dumps(ingetrazo_core.tag_edits.template_of(scene)))
    other = Scene()
    ingetrazo_core.tag_edits.apply_template(other, data)
    assert other.bricks == {"brick": chile, "block": block}


def test_a_block_and_a_ticholo_are_drawn_their_own_size():
    from core.hatches import bond_course, piece_kind, real_unit
    assert piece_kind("block") == "block"
    assert piece_kind("hollow_brick") == "hollow"
    assert piece_kind("brick_bond") == "brick"
    assert piece_kind("ashlar") is None and piece_kind("concrete") is None
    _d, course, rects = bond_course("block", 0, 190.0,
                                    (390.0, 190.0, 190.0, 10.0), 0, 2000)
    assert course == pytest.approx(200.0)
    assert rects[1][1] - rects[1][0] == pytest.approx(400.0)
    _d, course, rects = bond_course("block", 0, 190.0,
                                    (440.0, 190.0, 240.0, 10.0), 0, 2000)
    assert course == pytest.approx(250.0)          # a taller block
    assert real_unit("block", (440.0, 190.0, 240.0, 10.0))[1] == \
        pytest.approx(0.25)
    # The ticholo: 12 × 17 × 25 by default, a course 18 cm.
    _d, course, _r = bond_course("hollow_brick", 0, 120.0, None, 0, 1000)
    assert course == pytest.approx(180.0)


# ---- Bed joint and head joint ------------------------------------------------------

def test_a_brick_may_have_two_joints():
    from core.hatches import brick_dims, clean_brick
    assert clean_brick((290, 140, 71, 10, 10)) == (290.0, 140.0, 71.0, 10.0)
    assert clean_brick((290, 140, 71, 12, 8)) == (290.0, 140.0, 71.0, 12.0,
                                                  8.0)
    assert clean_brick((290, 140, 71, 12, 99)) is None
    assert brick_dims((290, 140, 71, 10)) == (290.0, 140.0, 71.0, 10.0, 10.0)
    assert Layer.from_dict(Layer("X", brick=(290, 140, 71, 12, 8))
                           .to_dict()).brick[4] == 8.0


def test_the_bed_joint_sets_the_courses_the_head_joint_the_bricks():
    from core.hatches import bond_course, brick_families
    brick = (240.0, 115.0, 50.0, 15.0, 5.0)        # 15 bed, 5 head
    depth, course, rects = bond_course("brick", 0, 115.0, brick, 0, 1000)
    assert course == pytest.approx(65.0)           # height + bed joint
    assert rects[1][1] - rects[1][0] == pytest.approx(245.0)  # + head
    assert brick_families("brick", brick)[1][4] * 2 == pytest.approx(
        1.5 * 245.0 / 65.0)


def test_double_joints_show_each_joint_its_own_width():
    from core.hatches import bond_marks
    brick = (240.0, 115.0, 50.0, 20.0, 6.0)
    # A paved area at 1:10 (2 m: no wall, the bond's face), × 1 the
    # course (70 mm → 1.5 paper mm): per_mm known.
    face = bond_marks("brick", ((0.0, 0.0), (1.0, 0.0), (0.0, 1.0),
                                200.0, 200.0), "plan", 1.0, 10.0, brick,
                      "double")
    one = face["bricks"][0]
    xs = sorted({round(x, 6) for x, _y in one})
    ys = sorted({round(y, 6) for _x, y in one})
    per_mm = 1.5 / 70.0
    assert xs[1] - xs[0] == pytest.approx(240.0 * per_mm)   # head off
    assert ys[1] - ys[0] == pytest.approx(50.0 * per_mm)    # bed off


# ---- Aerated concrete, load-bearing ceramic blocks, Uruguay ----------------------

def test_aerated_concrete_is_big_glued_and_porous():
    from core.hatches import bond_course, bond_marks, piece_kind
    assert piece_kind("aac_block") == "aac"
    _d, course, rects = bond_course("aac_block", 0, 150.0, None, 0, 2000)
    assert course == pytest.approx(203.0)          # 200 and a 3 mm joint
    assert rects[1][1] - rects[1][0] == pytest.approx(603.0)
    wall = ((0.0, 0.0), (1.0, 0.0), (0.0, 1.0), 60.0, 15.0)   # 1:10
    marks = bond_marks("aac_block", wall, "plan", 5.0, 10.0)
    assert marks["polys"]                          # its pores
    assert hatch_marks_have_circles("aac_block")


def hatch_marks_have_circles(key):
    from core.hatches import hatch_marks
    return bool(hatch_marks(key, (0.0, 0.0, 20.0, 20.0))["circles"])


def test_a_load_bearing_ceramic_block_shows_its_upright_holes():
    from core.hatches import bond_course, bond_marks, piece_kind
    assert piece_kind("perforated_block") == "perforated"
    _d, course, rects = bond_course("perforated_block", 0, 120.0, None,
                                    0, 1000)
    assert course == pytest.approx(200.0)          # 190 and its joint
    assert {r[4] for r in rects} == {"perforated"}
    wall = ((0.0, 0.0), (1.0, 0.0), (0.0, 1.0), 60.0, 12.0)   # 1:10
    plan = bond_marks("perforated_block", wall, "plan", 5.0, 10.0)
    assert len(plan["polys"]) > 20                 # the holes, from above
    section = bond_marks("perforated_block",
                         ((0.0, 40.0), (0.0, -1.0), (1.0, 0.0), 40.0, 12.0),
                         "vertical", 5.0, 10.0)
    webs = [ln for ln in section["lines"] if abs(ln[0] - ln[2]) < 1e-9]
    assert webs                                    # cut through them


def test_uruguay_and_the_new_pieces_are_in_the_lists():
    from core.hatches import PIECE_SIZES
    names = {k: [n for n, _s in v] for k, v in PIECE_SIZES.items()}
    assert "Uruguay" in names["brick"]
    assert "Rejillón" in names["perforated"]
    assert names["aac"][0] == "Aerated 15"
    assert dict(PIECE_SIZES["aac"])["Aerated 15"][3] == 3.0


def test_a_precast_concrete_brick_shows_its_holes():
    from core.hatches import PIECE_SIZES, bond_course, bond_marks, piece_kind
    assert piece_kind("concrete_brick") == "cored"
    assert PIECE_SIZES["cored"][0][0] == "Concrete brick 12"
    _d, course, rects = bond_course("concrete_brick", 0, 120.0, None, 0, 1000)
    assert course == pytest.approx(70.0)
    assert {r[4] for r in rects} == {"cored"}
    wall = ((0.0, 0.0), (1.0, 0.0), (0.0, 1.0), 60.0, 12.0)   # 1:10
    plan = bond_marks("concrete_brick", wall, "plan", 5.0, 10.0)
    assert 4 <= len(plan["polys"]) < 40              # a few holes each


# ---- The layers' hatches in the model --------------------------------------

def _model_cut(scene):
    """The active plane through ``scene``: its chords and each one's layer,
    as the viewer tags them."""
    from core.hlr import collect_geometry
    tris, _h, _s, _hl, _sl, tags = collect_geometry(scene, layers=True)
    t = np.asarray(tris, dtype=np.float64)
    sp = scene.active_section()
    chords, rows = ingetrazo_core.section_cut_chords(t[:, 0], t[:, 1] - t[:, 0], t[:, 2] - t[:, 0],
                              sp.normal, sp.point, rows=True)
    return chords, np.asarray(tags, dtype=object)[rows]


def test_the_model_cut_shows_each_layers_hatch():
    from ingetrazo_core.section import layer_fills
    scene = _wall_and_door(Scene())
    scene.layer("Muros").hatch = "brick"
    scene.layer("Carpinterías").hatch = "solid"
    scene.layer("Carpinterías").hatch_color = (0.2, 0.4, 0.1)
    chords, tags = _model_cut(scene)
    grey = (0.35, 0.37, 0.41)
    fills = layer_fills(chords, tags, scene.active_section().normal, scene,
                        grey)
    assert len(fills) == 3
    tris, rgb, lines, _ink = fills[0]           # the loose box: no hatch
    assert rgb == grey and lines is None and len(tris)
    door = [f for f in fills if f[1] == (0.2, 0.4, 0.1)]
    assert len(door) == 1 and door[0][2] is None         # solid, its ink
    walls = [f for f in fills if f[2] is not None]
    assert len(walls) == 1
    _tris, ground, lines, ink = walls[0]
    assert ink == grey                                   # as the frame says
    assert ground == pytest.approx(tuple(c + (1 - c) * 0.85 for c in grey))
    assert np.allclose(lines[:, :, 2], 1.0)              # in the plane
    xy = lines[:, :, :2].reshape(-1, 2)
    assert (xy[:, 0] > -1e-4).all() and (xy[:, 0] < 4 + 1e-4).all()
    assert (xy[:, 1] > -1e-4).all() and (xy[:, 1] < 0.3 + 1e-4).all()
    d = np.abs(lines[:, 1, :2] - lines[:, 0, :2])
    assert (d[:, 1] < 1e-6).any() and (d[:, 0] < 1e-6).any()   # courses
    assert len(lines) > 20


def test_a_section_cut_lays_its_courses_level():
    u, v = ingetrazo_core.section_sheet_basis(V(0, 1, 0))                      # a section
    assert v == pytest.approx((0, 0, 1)) and abs(u[2]) < 1e-9
    u, v = ingetrazo_core.section_sheet_basis(V(0, 0, 1))                      # a plan
    assert u == pytest.approx((1, 0, 0)) and v == pytest.approx((0, 1, 0))


def test_the_plane_keeps_the_layers_fill():
    sp = SectionPlane(V(0, 0, 1), V(0, 0, 1))
    sp.fill = {"pattern": "layers"}
    assert SectionPlane.from_dict(sp.to_dict()).fill == {"pattern": "layers"}


# ---- Where the pieces start ------------------------------------------------

def _shifted(marks, dx, dy):
    """The lines moved back by ``(dx, dy)``, sorted and flattened — one
    list of numbers, which pytest.approx compares (it takes no nesting)."""
    lines = sorted((round(x0 - dx, 6), round(y0 - dy, 6), round(x1 - dx, 6),
                    round(y1 - dy, 6)) for x0, y0, x1, y1 in marks["lines"])
    return [v for line in lines for v in line]


def test_a_wall_starts_with_a_whole_brick_wherever_it_stands():
    from core.hatches import bond_marks
    here = ((0.0, 0.0), (1.0, 0.0), (0.0, 1.0), 60.0, 12.0)
    there = ((7.3, 4.1), (1.0, 0.0), (0.0, 1.0), 60.0, 12.0)
    a = bond_marks("brick_bond", here, "plan", 5.0, 10.0)
    b = bond_marks("brick_bond", there, "plan", 5.0, 10.0)
    assert _shifted(a, 0, 0) == pytest.approx(_shifted(b, 7.3, 4.1))
    # The first head joint at the wall's end, a whole brick from it.
    heads = sorted(x0 for x0, y0, x1, y1 in a["lines"]
                   if abs(x0 - x1) < 1e-9 and x0 > 1e-6)
    assert heads[0] > 1.0


def test_a_section_starts_its_first_course_at_the_foot():
    from core.hatches import bond_marks
    # A wall cut across, 12 mm thick on paper, 40 mm tall, its foot at y=40.
    band = ((0.0, 40.0), (0.0, -1.0), (1.0, 0.0), 40.0, 12.0)
    marks = bond_marks("brick", band, "vertical", 5.0, 10.0)
    beds = sorted({round(y0, 6) for x0, y0, x1, y1 in marks["lines"]
                   if abs(y0 - y1) < 1e-9}, reverse=True)
    assert beds[0] == pytest.approx(40.0)            # the first, at the foot
    turned = ((0.0, 0.0), (0.0, 1.0), (1.0, 0.0), 40.0, 12.0)   # drawn down
    again = bond_marks("brick", turned, "vertical", 5.0, 10.0)
    assert _shifted(marks, 0, 0) == pytest.approx(_shifted(again, 0, 0))


def test_the_offset_moves_the_first_joint_and_the_first_course():
    from core.hatches import bond_marks, real_scale
    wall = ((0.0, 0.0), (1.0, 0.0), (0.0, 1.0), 60.0, 12.0)
    a = bond_marks("brick_bond", wall, "plan", 5.0, 10.0)
    b = bond_marks("brick_bond", wall, "plan", 5.0, 10.0, offset=(50.0, 0.0))
    xs_a = sorted(x0 for x0, y0, x1, y1 in a["lines"] if abs(x0 - x1) < 1e-9)
    xs_b = sorted(x0 for x0, y0, x1, y1 in b["lines"] if abs(x0 - x1) < 1e-9)
    assert xs_a != xs_b
    shift = xs_b[1] - xs_a[1]
    assert shift > 0                                  # moved along the wall
    band = ((0.0, 40.0), (0.0, -1.0), (1.0, 0.0), 40.0, 12.0)
    up = bond_marks("brick", band, "vertical", 5.0, 10.0, offset=(0.0, 30.0))
    beds = sorted({round(y0, 6) for x0, y0, x1, y1 in up["lines"]
                   if abs(y0 - y1) < 1e-9}, reverse=True)
    inside = [b for b in beds if b <= 40.0 + 1e-6]
    assert 30.0 < inside[0] < 40.0 - 1e-6             # moved up the wall
    assert real_scale("brick", 10.0) is not None


def test_a_pattern_of_pieces_moves_and_a_texture_does_not():
    from core.hatches import PLACED, hatch_lines, hatch_marks
    box = (0.0, 0.0, 40.0, 40.0)
    a = hatch_marks("tile", box, 1.0)
    b = hatch_marks("tile", box, 1.0, offset=(1.5, -2.0))
    assert a["lines"] != b["lines"]
    assert "tile" in PLACED and "concrete" not in PLACED
    ring = [[(0, 0), (40, 0), (40, 40), (0, 40)]]
    still = hatch_lines("diagonal", ring, {"offset": (500.0, 0.0),
                                           "scale_n": 50.0})
    assert still == hatch_lines("diagonal", ring, {"scale_n": 50.0})
    moved = hatch_lines("tile", ring, {"offset": (500.0, 0.0),
                                       "scale_n": 50.0})
    assert moved != hatch_lines("tile", ring, {"scale_n": 50.0})


def test_the_layer_keeps_where_its_pieces_start():
    ly = Layer("Muros", hatch="brick", hatch_offset=(115.0, -20.0))
    assert ly.has_style()
    back = Layer.from_dict(ly.to_dict())
    assert back.hatch_offset == (115.0, -20.0)
    assert Layer.from_dict(Layer("Muros").to_dict()).hatch_offset is None
    assert ingetrazo_core.tag_styles.clean_offset((0, 0)) is None
    assert ingetrazo_core.tag_styles.clean_offset(("a", 1)) is None
    assert ingetrazo_core.tag_styles.clean_offset((float("nan"), 1)) is None
    assert ingetrazo_core.tag_styles.clean_offset((1e9, 1)) == (10000.0, 1.0)
    scene = Scene()
    scene.layers.append(ly)
    assert ingetrazo_core.tag_styles.cut_hatch_forms(scene, 50.0)["Muros"]["offset"] == (115.0, -20.0)


# ---- A line type's own colour ----------------------------------------------

def _gas_scene(scene):
    """A wall on «Gas», its line type yellow."""
    ingetrazo_core.tag_edits.load_line_types(scene, {"Gas": {"pattern": [6.0, -2.0],
                                    "color": [0.85, 0.65, 0.0]}})
    scene.layers.append(Layer("Gas", linetype="lin:Gas"))
    pipe = Group(Mesh(), name="caño")
    _box(pipe.mesh, 0, 0, 0, 4, 0.1, 0.1)
    pipe.layer = "Gas"
    scene.groups.append(pipe)
    return scene


def test_a_line_type_keeps_its_colour_and_inks_with_it():
    scene = _gas_scene(Scene())
    assert ingetrazo_core.tag_styles.linetype_color("lin:Gas", scene) == (0.85, 0.65, 0.0)
    assert ingetrazo_core.tag_styles.linetype_color("dashed", scene) is None
    # On any sheet: with the layers' colours or without.
    assert ingetrazo_core.tag_styles.line_styles(scene, False)["Gas"][0] == (0.85, 0.65, 0.0)
    assert ingetrazo_core.tag_styles.line_styles(scene, True)["Gas"][0] == (0.85, 0.65, 0.0)
    # A bad colour is dropped, the line type kept.
    ingetrazo_core.tag_edits.load_line_types(scene, {"Agua": {"pattern": [6.0, -2.0],
                                     "color": ["x", 1, 2]}})
    assert "color" not in scene.line_types["Agua"]


def test_the_line_type_colour_is_saved(tmp_path):
    from formats import igz
    scene = _gas_scene(Scene())
    path = tmp_path / "gas.igz"
    igz.save_scene(scene, path)
    back = Scene()
    igz.load_into(back, path)
    assert tuple(back.line_types["Gas"]["color"]) == (0.85, 0.65, 0.0)


def test_document_line_ink_is_kept_until_changed_back_to_by_tag():
    from tests.test_project_drawing_filters import Project
    p = Project(Layer("Red"))
    edit_line(p, "save", name="Agua", pieces=[6,-2], tags=["Red"], text="AGUA", text_size=2.5, color=[0,.40,.85])
    assert p.scene.line_types["Agua"]["color"] == (0., .40, .85)
    assert p.scene.layer("Red").linetype == "lin:Agua"
    edit_line(p, "save", name="Agua", original="Agua", pieces=[6,-2])
    assert "color" not in p.scene.line_types["Agua"]


def test_a_dash_pattern_cuts_an_edge_as_a_sheet_at_1_50():
    import numpy as np
    from core.layers import dash_segments
    seg = np.array([[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]]])
    dashes, gaps = dash_segments(seg, (6.0, 2.0))       # 0.30 m, 0.10 m
    assert len(dashes) == 3 and len(gaps) == 2
    assert np.allclose(dashes[0], [[0, 0, 0], [0.3, 0, 0]])
    assert np.allclose(gaps[0], [[0.3, 0, 0], [0.4, 0, 0]])
    whole, none = dash_segments(seg, ())                 # continuous
    assert len(whole) == 1 and not len(none)
    far = np.array([[[0.0, 0, 0], [1000.0, 0, 0]]])
    whole, none = dash_segments(far, (1.0, 1.0))         # too fine: whole
    assert len(whole) == 1 and not len(none)

def test_a_built_in_line_type_takes_a_colour_of_the_document(tmp_path):
    from formats import igz
    scene = Scene()
    scene.layers.append(Layer("Ejes", linetype="dashdot"))
    assert ingetrazo_core.tag_styles.linetype_color("dashdot", scene) is None
    assert ingetrazo_core.tag_styles.set_linetype_color(scene, "dashdot", (0.85, 0.10, 0.10))
    assert not ingetrazo_core.tag_styles.set_linetype_color(scene, "dashdot", (0.85, 0.10, 0.10))
    assert ingetrazo_core.tag_styles.line_styles(scene, False)["Ejes"][0] == (0.85, 0.10, 0.10)
    path = tmp_path / "ejes.igz"
    igz.save_scene(scene, path)
    back = Scene()
    igz.load_into(back, path)
    assert ingetrazo_core.tag_styles.linetype_color("dashdot", back) == (0.85, 0.10, 0.10)
    assert ingetrazo_core.tag_edits.template_of(scene)["linetype_colors"]["dashdot"] == [
        0.85, 0.10, 0.10]
    assert ingetrazo_core.tag_styles.set_linetype_color(scene, "dashdot", None)          # by layer
    assert ingetrazo_core.tag_styles.linetype_color("dashdot", scene) is None
    assert not scene.linetype_colors


def test_standard_and_custom_line_inks_are_independent():
    from tests.test_project_drawing_filters import Project
    p = Project()
    edit_line(p, "ink", name="dashed", color=[0., .4, .85])
    assert p.scene.linetype_colors["dashed"] == (0., .4, .85)
    edit_line(p, "save", name="Agua", pieces=[6,-2], color=[.85, .10, .10])
    assert p.scene.line_types["Agua"]["color"] == (.85, .10, .10)
    edit_line(p, "ink", name="dashed", color=None)
    assert "dashed" not in p.scene.linetype_colors
    assert p.win.viewport.history.undo() and p.scene.linetype_colors["dashed"] == (0., .4, .85)


def test_an_open_cell_of_a_honeycomb_bond_is_closed_on_both_sides():
    """Palomero: each open cell is crossed and closed — the side after a
    brick too (a brick draws only the joint before it)."""
    from core.hatches import bond_marks
    wall = ((0.0, 0.0), (1.0, 0.0), (0.0, 1.0), 80.0, 12.0)    # 1:10
    marks = bond_marks("brick_honeycomb", wall, "plan", 5.0, 10.0)
    uprights = {round(x0, 3) for x0, y0, x1, y1 in marks["lines"]
                if abs(x0 - x1) < 1e-9 and abs(y1 - y0) > 1e-6}
    crosses = {(round(min(x0, x1), 3), round(max(x0, x1), 3))
               for x0, y0, x1, y1 in marks["lines"]
               if abs(x0 - x1) > 1e-6 and abs(y0 - y1) > 1e-6}
    inside = [c for c in crosses if c[0] > 1e-3 and c[1] < 80.0 - 1e-3]
    assert inside                                     # cells to look at
    for left, right in inside:
        assert left in uprights and right in uprights


# ---- A solid of two layers; picking with dashed layers ---------------------

def _wall_with_a_face_elsewhere(scene):
    scene = _wall_and_door(scene)
    scene.layers.append(Layer("Revestimiento"))
    wall = scene.groups[0]
    side = next(f for f in wall.mesh.faces
                if all(abs(v.y()) < 1e-9 for v in f.vertices))
    L.assign_layer(side, "Revestimiento")
    return scene


def test_a_solid_with_a_face_on_another_layer_keeps_its_cut():
    """The rings are chained layer by layer, and neither the wall's three
    sides nor the one moved to another layer close alone: the wall had no
    fill at all. It closes whole, and is the layer of most of its outline."""
    scene = _wall_with_a_face_elsewhere(Scene())
    cam = _top_camera(scene)
    plain = hlr_drawing(scene, cam, profiles=False)
    d = hlr_drawing(scene, cam, profiles=False, layers=True)
    assert len(plain.loops) == 3
    assert len(d.loops) == 3
    assert sorted(d.loop_layers) == sorted(
        ["Muros", "Carpinterías", DEFAULT_LAYER])


def test_the_model_cut_fills_a_solid_of_two_layers():
    from ingetrazo_core.section import layer_fills
    scene = _wall_with_a_face_elsewhere(Scene())
    scene.layer("Muros").hatch = "solid"
    scene.layer("Muros").hatch_color = (0.6, 0.1, 0.1)
    chords, tags = _model_cut(scene)
    assert "Revestimiento" in set(tags)
    fills = layer_fills(chords, tags, scene.active_section().normal, scene,
                        (0.35, 0.37, 0.41))
    wall = [f for f in fills if f[1] == (0.6, 0.1, 0.1)]
    assert len(wall) == 1
    area = sum(0.5 * float(np.linalg.norm(np.cross(t[1] - t[0], t[2] - t[0])))
               for t in np.asarray(wall[0][0], dtype=np.float64))
    assert abs(area - 4 * 0.3) < 1e-6                    # the whole wall


def test_solids_each_of_one_layer_are_tagged_as_they_were():
    scene = _wall_and_door(Scene())
    chords, tags = _model_cut(scene)
    assert ingetrazo_core.hlr_closing_tags(chords, tags) == list(tags)
