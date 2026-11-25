"""Export the view as SVG: the hidden-line drawing the DXF export writes,
as a vector drawing to finish in an illustration program — one SVG layer
per model layer (and per line class), each in its colour, line type and
pen, at paper size, its touching segments chained into one path."""
from __future__ import annotations

from tests.raster_samples import pixels, solid_raster
from ingetrazo_render import SurveyRaster
import xml.etree.ElementTree as ET

import numpy as np
import pytest
from ingetrazo_core import Vec3 as QVector3D
from tests.shell_isolation import restore_shell_patches  # noqa: F401

SVG = "{http://www.w3.org/2000/svg}"
INK = "{http://www.inkscape.org/namespaces/inkscape}"


def _layers(path):
    """``{label: <g>}`` of the SVG's top-level layers."""
    root = ET.parse(path).getroot()
    assert root.tag == SVG + "svg"
    out = {}
    for g in root.findall(SVG + "g"):
        assert g.get(INK + "groupmode") == "layer"
        out[g.get(INK + "label")] = g
    return root, out


def _points(path_el):
    d = path_el.get("d").replace("M", "").replace("Z", "").split("L")
    return [tuple(float(v) for v in p.split(",")) for p in d]


# ---- Chaining ---------------------------------------------------------------

def test_a_square_comes_apart_into_its_four_sides():
    """A corner ends a line: each side of a rectangle is a line of its own
    in the illustration program, not one stroke round the corners."""
    from formats.svg_out import chain_segments
    segs = [(1, 0, 1, 1), (0, 0, 1, 0), (0, 1, 0, 0), (0, 1, 1, 1)]
    chains = chain_segments(segs)
    assert len(chains) == 4
    assert all(len(p) == 2 and not c for p, c in chains)


def test_a_junction_ends_every_line_that_meets_there():
    """A T: the bar's two halves and the stem are three lines — chaining
    across the junction welded lines going different ways into one."""
    from formats.svg_out import chain_segments
    segs = [(0, 0, 1, 0), (1, 0, 2, 0), (1, 0, 1, -1), (9, 9, 9, 9)]
    chains = chain_segments(segs)
    assert sorted(len(p) for p, _c in chains) == [2, 2, 2]


def test_the_pieces_of_one_edge_and_an_arc_stay_one_line():
    import math as m
    from formats.svg_out import chain_segments
    # One edge the hidden-line pass split in three, drawn out of order.
    edge = [(2, 0, 3, 0), (0, 0, 1, 0), (1, 0, 2, 0)]
    chains = chain_segments(edge)
    assert len(chains) == 1 and len(chains[0][0]) == 4
    # A circle of 24 facets: one closed line.
    pts = [(m.cos(k * m.pi / 12), m.sin(k * m.pi / 12)) for k in range(24)]
    ring = [(*pts[k], *pts[(k + 1) % 24]) for k in range(24)]
    chains = chain_segments(ring)
    assert len(chains) == 1
    assert chains[0][1] and len(chains[0][0]) == 24


# ---- The writer -------------------------------------------------------------

def test_each_group_is_a_layer_with_its_colour_line_type_and_pen(tmp_path):
    from formats.svg_out import save_svg_layers
    square = [(0, 0, 2, 0), (2, 0, 2, 1), (2, 1, 0, 1), (0, 1, 0, 0)]
    groups = [
        ("PLANTA-CORTE", np.array([(0, 0, 2, 1)], float)),
        ("Muros ñ", np.array(square, float), "DASHDOT", (1.0, 0.0, 0.0),
         None, 0.7),
        ("Muros ñ-PERFIL", np.array([(0, 0, 0, 1)], float), "DASHDOT",
         (1.0, 0.0, 0.0), None, None),
        ("Vacía", np.zeros((0, 4))),
        ("PLANTA", np.array([(0, 0, 1, 1)], float)),
    ]
    path = tmp_path / "v.svg"
    n = save_svg_layers(path, groups, scale_n=50.0,
                        pens={"cut": 0.6, "profile": 0.4, "edge": 0.2})
    assert n == 7
    root, layers = _layers(path)
    assert list(layers) == ["PLANTA-CORTE", "Muros ñ", "Muros ñ-PERFIL",
                            "PLANTA"]                    # the empty one: out
    # 2 m × 1 m at 1:50 = 40 × 20 mm, plus a 10 mm margin all round.
    assert root.get("width") == "60.000mm"
    assert root.get("height") == "40.000mm"
    assert root.get("viewBox") == "0 0 60.000 40.000"
    muros = layers["Muros ñ"]
    assert muros.get("stroke") == "#ff0000"
    assert muros.get("stroke-width") == "0.7"            # the layer's own pen
    assert muros.get("stroke-dasharray")
    assert layers["Muros ñ-PERFIL"].get("stroke-width") == "0.4"
    assert layers["PLANTA-CORTE"].get("stroke-width") == "0.6"
    assert layers["PLANTA"].get("stroke-width") == "0.2"
    assert layers["PLANTA"].get("stroke") == "#000000"
    assert layers["PLANTA"].get("stroke-dasharray") is None
    paths = muros.findall(SVG + "path")
    assert len(paths) == 4                       # one per side
    # The model's y up is the page's y down: (0, 1) m sits at the top.
    corners = {p for el in paths for p in _points(el)}
    assert corners == {(10.0, 30.0), (50.0, 30.0), (50.0, 10.0),
                       (10.0, 10.0)}
    ids = [g.get("id") for g in root.findall(SVG + "g")]
    assert len(set(ids)) == len(ids) and all(i[0].isalpha() or i[0] == "_"
                                             for i in ids)


def test_an_empty_view_still_writes_a_valid_svg(tmp_path):
    from formats.svg_out import save_svg_layers
    path = tmp_path / "vacia.svg"
    assert save_svg_layers(path, [("VISTA", np.zeros((0, 4)))]) == 0
    _root, layers = _layers(path)
    assert layers == {}


def test_the_dxf_still_reads_groups_that_carry_a_pen(tmp_path):
    """The layer groups now carry a sixth element, the pen (the fifth is a
    custom line type's dashes); the DXF writer keeps reading them as
    before."""
    from formats.dxf_out import save_dxf_layers
    path = tmp_path / "v.dxf"
    n = save_dxf_layers(path, [("A", [(0, 0, 1, 0)], "DASHED",
                                (1.0, 0.0, 0.0), None, 0.5)])
    assert n == 1
    assert "0\nLAYER\n2\nA\n70\n0\n62\n1\n6\nDASHED\n" in path.read_text(
        encoding="ascii")


# ---- From the program ------------------------------------------------------

def _two_layer_scene(scene):
    from core.group import Group
    from core.layers import DEFAULT_LAYER, Layer
    from core.mesh import Mesh

    def box(mesh, x0):
        V = QVector3D
        mesh.add_face([V(x0, 0, 0), V(x0 + 1, 0, 0), V(x0 + 1, 1, 0),
                       V(x0, 1, 0)])

    scene.layers[:] = [Layer(DEFAULT_LAYER),
                       Layer("Muros", linetype="dashed", lineweight=0.5),
                       Layer("Mobiliario", color=(0.0, 0.0, 1.0))]
    a, b = Group(Mesh(), name="a"), Group(Mesh(), name="b")
    box(a.mesh, 0.0)
    box(b.mesh, 3.0)
    a.layer, b.layer = "Muros", "Mobiliario"
    scene.groups[:] = [a, b]
    scene.version += 1


def test_the_export_menu_writes_the_view_as_svg(native_host, tmp_path):
    win = native_host.window
    vp = win.viewport
    native_host.resize(800, 600, 1.)
    vp.camera.set_aspect(800, 600)
    _two_layer_scene(vp.scene)
    vp.notify_scene_changed()
    lo, hi = vp.scene.bounds()
    vp.camera.fit_to(lo, hi, 1.2)
    for mode in ("parallel", "perspective"):
        vp.camera.perspective = mode == "perspective"
        vp.camera.two_point = False
        asked = tmp_path / mode                  # no extension typed
        _export_sheet(win, asked, opener=lambda: native_host.menu("export_view_svg"))
        root, layers = _layers(tmp_path / f"{mode}.svg")
        assert root.findall(f".//{SVG}path"), mode
        # The faces go under the lines, on their own layers.
        names = list(layers)
        faces = [i for i, n in enumerate(names) if n.endswith("-CARAS")]
        lines = [i for i, n in enumerate(names) if "-CARAS" not in n]
        assert faces and lines and max(faces) < min(lines), names
    _root, layers = _layers(tmp_path / "parallel.svg")
    names = " ".join(layers)
    assert "Muros" in names and "Mobiliario" in names
    blue = [g for name, g in layers.items()
            if name.startswith("Mobil") and "-CARAS" not in name]
    assert blue
    assert all(g.get("stroke") == "#0000ff" for g in blue)


# ---- The faces --------------------------------------------------------------

def _box(mesh, x0=0.0, y0=0.0, z0=0.0, s=1.0):
    """A closed cube, every face wound outwards."""
    V = QVector3D
    x1, y1, z1 = x0 + s, y0 + s, z0 + s
    for loop in (
            [(x0, y0, z0), (x0, y1, z0), (x1, y1, z0), (x1, y0, z0)],
            [(x0, y0, z1), (x1, y0, z1), (x1, y1, z1), (x0, y1, z1)],
            [(x0, y0, z0), (x1, y0, z0), (x1, y0, z1), (x0, y0, z1)],
            [(x0, y1, z0), (x0, y1, z1), (x1, y1, z1), (x1, y1, z0)],
            [(x0, y0, z0), (x0, y0, z1), (x0, y1, z1), (x0, y1, z0)],
            [(x1, y0, z0), (x1, y1, z0), (x1, y1, z1), (x1, y0, z1)]):
        mesh.add_face([V(*p) for p in loop])


def _camera(scene, view="iso"):
    from core.camera import OrbitCamera
    cam = OrbitCamera()
    cam.set_aspect(800, 600)
    cam.perspective = False
    cam.set_view(view)
    lo, hi = scene.bounds()
    cam.fit_to(lo, hi, 1.2)
    return cam


def _ring_area(f):
    from core.vista_caras import _area
    return sum(abs(_area(r)) for r in f.rings)


def test_the_matte_light_is_the_viewport_s():
    from core.vista_caras import MATTE_LIGHT
    from tests.controller_surface import ControllerSurface as Viewport
    light = Viewport._LIGHT
    assert np.allclose(MATTE_LIGHT, [light.x(), light.y(), light.z()])


def test_an_unpainted_box_is_ingetrazo_white_lit_as_in_the_view():
    from core.scene import Scene
    from core.style import DEFAULT_FACE_COLOR
    from core.vista_caras import shade_factor, shaded, view_faces
    scene = Scene()
    _box(scene.loose_mesh)
    faces = view_faces(scene, _camera(scene))
    assert len(faces) == 3                       # top and two sides
    assert not any(f.back for f in faces)
    tones = sorted({f.rgb for f in faces})
    assert len(tones) == 3                       # three lights, three tones
    top = shaded(DEFAULT_FACE_COLOR, shade_factor((0, 0, 1)))
    assert max(tones) == top                     # the top is the lightest
    flat = view_faces(scene, _camera(scene), as_view=False)
    assert {f.rgb for f in flat} == {tuple(DEFAULT_FACE_COLOR)}


def test_a_painted_object_wears_its_colour_and_a_face_its_own():
    from core.group import Group
    from core.mesh import Mesh
    from core.scene import Scene
    from core.vista_caras import view_faces
    scene = Scene()
    g = Group(Mesh(), name="caja")
    _box(g.mesh)
    g.material = {"color": (0.0, 0.0, 1.0)}
    top = next(f for f in g.mesh.faces if f.normal().z() > 0.5)
    top.attrs["color"] = (1.0, 0.0, 0.0)
    scene.groups.append(g)
    faces = view_faces(scene, _camera(scene), as_view=False)
    assert sorted(f.rgb for f in faces) == [(0.0, 0.0, 1.0), (0.0, 0.0, 1.0),
                                            (1.0, 0.0, 0.0)]
    assert {f.layer for f in faces} == {"Layer 0"}


def test_a_nearer_face_hides_what_is_behind_it():
    """Seen from the top, a square floating over a bigger one: the lower
    keeps only the ring round the upper — no face lies under another."""
    from core.scene import Scene
    from core.vista_caras import view_faces
    V = QVector3D
    scene = Scene()
    scene.loose_mesh.add_face([V(0, 0, 0), V(4, 0, 0), V(4, 4, 0),
                               V(0, 4, 0)])
    scene.loose_mesh.add_face([V(1, 1, 1), V(3, 1, 1), V(3, 3, 1),
                               V(1, 3, 1)])
    from core.vista_caras import _area
    faces = view_faces(scene, _camera(scene, "top"))
    assert len(faces) == 2
    upper, lower = sorted(faces, key=lambda f: len(f.rings))
    assert _ring_area(upper) == pytest.approx(4.0, rel=1e-4)
    # The lower one: its outline with a hole where the upper one is.
    assert sorted(abs(_area(r)) for r in lower.rings) == pytest.approx(
        [4.0, 16.0], rel=1e-4)


def test_glass_hides_nothing_and_a_face_seen_from_behind_is_back_colour():
    from core.scene import Scene
    from core.style import effective_back_color
    from core.vista_caras import view_faces
    V = QVector3D
    scene = Scene()
    scene.loose_mesh.add_face([V(0, 0, 0), V(4, 0, 0), V(4, 4, 0),
                               V(0, 4, 0)])
    glass = scene.loose_mesh.add_face([V(1, 1, 1), V(3, 1, 1), V(3, 3, 1),
                                       V(1, 3, 1)])
    glass.attrs.update({"color": (0.5, 0.8, 1.0), "opacity": 0.3})
    faces = view_faces(scene, _camera(scene, "top"), as_view=False)
    assert sum(_ring_area(f) for f in faces) == pytest.approx(20.0,
                                                               rel=1e-4)
    see = [f for f in faces if f.opacity < 1]
    assert len(see) == 1 and faces[-1] is see[0]     # drawn last, on top
    # From below, the floor shows its back.
    back = [f for f in view_faces(scene, _camera(scene, "bottom"),
                                  as_view=False) if f.opacity >= 1]
    assert back and all(f.back for f in back)
    assert back[0].rgb == tuple(effective_back_color(scene.display_style,
                                                     scene))


def _png(path, w=8, h=4):
    img = solid_raster(w, h, (200, 40, 40))
    assert img.save(str(path))
    return str(path)


def _quadrants(path):
    """An 8 x 8 texture of four plain quadrants, to read orientation."""
    cols = {(0, 0): (220, 30, 30), (1, 0): (30, 200, 30),
            (0, 1): (30, 30, 220), (1, 1): (230, 220, 30)}
    rgba = b"".join(bytes((*cols[x // 4, y // 4], 255)) for y in range(8) for x in range(8))
    img = SurveyRaster(8, 8, rgba)
    assert img.save(str(path))
    return str(path), img


def test_a_textured_face_is_one_shape_filled_as_the_view_lays_it(tmp_path):
    """A 2 x 1 m wall with a 0.5 x 0.5 m tile, seen from the front at 1:100:
    one path filled with a pattern that needs no transform — the texture is
    laid onto the face here, as the viewport does (its world → UV map and
    its image turned bottom-up), so any program shows it the same."""
    import base64
    from core.scene import Scene
    from core.texture import face_uv_axes
    from core.vista_caras import view_faces
    from formats.svg_out import save_svg_layers
    XL = "{http://www.w3.org/1999/xlink}href"
    V = QVector3D
    scene = Scene()
    wall = scene.loose_mesh.add_face([V(0, 0, 0), V(2, 0, 0), V(2, 0, 1),
                                      V(0, 0, 1)])
    path, src = _quadrants(tmp_path / "cuadrantes.png")
    tex = {"path": path, "sw": 0.5, "sh": 0.5}
    wall.attrs["texture"] = tex
    faces = view_faces(scene, _camera(scene, "front"), as_view=False)
    assert len(faces) == 1 and faces[0].texture is not None
    out = tmp_path / "muro.svg"
    save_svg_layers(out, [], scale_n=100.0, faces=faces)
    root, layers = _layers(out)
    paths = layers["Layer 0-CARAS"].findall(SVG + "path")
    assert len(paths) == 1
    fill = paths[0].get("fill")
    assert fill.startswith("url(#")
    pat = root.find(f".//{SVG}pattern[@id='{fill[5:-1]}']")
    assert pat.get("patternTransform") is None
    # The fill covers the wall's box: 2 x 1 m at 1:100.
    # The tile is the page from its origin; the image covers the wall's
    # box (2 x 1 m at 1:100) where the wall is.
    assert pat.get("x") == "0" and pat.get("y") == "0"
    assert float(pat.get("width")) == pytest.approx(
        float(root.get("width").rstrip("mm")))
    pic = pat.find(SVG + "image")
    assert float(pic.get("width")) == pytest.approx(20.0, abs=0.01)
    assert float(pic.get("height")) == pytest.approx(10.0, abs=0.01)
    assert float(pic.get("x")) == pytest.approx(10.0, abs=0.01)  # margin
    uri = pic.get(XL)
    baked = SurveyRaster.decode(base64.b64decode(uri.split(",", 1)[1]))
    gu, cu, gv, cv = face_uv_axes(tex, wall.normal())
    for wx, wz in ((0.1, 0.1), (0.4, 0.9), (0.6, 0.3), (1.3, 0.6),
                   (1.85, 0.15)):
        p = V(wx, 0, wz)
        u = QVector3D.dotProduct(gu, p) + cu
        v = QVector3D.dotProduct(gv, p) + cv
        # The viewport uploads the image bottom-up: v = 0 is its last row.
        tx = int((u % 1.0) * 8)
        ty = int((1.0 - v % 1.0) * 8) % 8
        want = pixels(src)[ty, tx].astype(int)
        # Front view: page x follows world x, page y runs down world z.
        got = pixels(baked)[int((1 - wz) * baked.height), int(wx / 2 * baked.width)].astype(int)
        assert abs(got[0] - want[0]) < 40, (wx, wz)
        assert abs(got[1] - want[1]) < 40, (wx, wz)
        assert abs(got[2] - want[2]) < 40, (wx, wz)
    # Without textures: its mean colour, no pattern.
    wall.attrs["texture"] = {"path": _png(tmp_path / "rojo.png"),
                             "sw": 1.0, "sh": 1.0}
    flat = view_faces(scene, _camera(scene, "front"), textures=False)
    assert flat[0].texture is None
    assert flat[0].rgb[0] > flat[0].rgb[1]       # reddish, as the image


def test_a_shaded_texture_is_darkened_in_the_file(tmp_path):
    import base64
    from formats.svg_out import _image_uri
    png = _png(tmp_path / "t.png")
    imgs = [SurveyRaster.decode(base64.b64decode(u.split(",", 1)[1]))
            for u in (_image_uri(png, 1.0), _image_uri(png, 0.5))]
    r_full = int(pixels(imgs[0])[1, 1, 0])
    r_dark = int(pixels(imgs[1])[1, 1, 0])
    assert r_dark == pytest.approx(r_full * 0.5, abs=4)


def _svg(
    *,
    advanced=True,
    prefs=None,
    screen=(278.0, 162.0),
    axes=False,
    shadows=False,
    selection=False,
    scenes=(),
    frame_scale=None,
    switch=None,
):
    import json
    from ingetrazo_render import SvgOptions

    return SvgOptions.current_view(
        json.dumps({"advanced": advanced} if prefs is None else prefs),
        screen,
        axes,
        shadows,
        selection,
        list(scenes),
        frame_scale,
        switch,
    )


def _rows(model):
    return {r[0]: r for r in model.wire()[4]}


def _opts(model):
    import json

    return json.loads(model.options_json())


def _saved(model):
    import json

    return json.loads(model.view_preferences())


def _export_sheet(win, path, fill="2", *, opener=None, scene=None):
    from ingetrazo_render import chrome_sheet

    (opener or win._on_export_view_svg)()
    provider = chrome_sheet.current(win)
    ctx = chrome_sheet.Ctx(win, None)
    form, model = provider.build(ctx).forms()
    model.receive("advanced", "1")
    model.receive("svg_preview", "0")
    model.receive("fill", fill)
    if scene:
        scene(model)
    form.set("destination", str(path))
    provider.act(ctx, "verb", ["export"])
    assert form.answer == "export"


def test_svg_face_options_and_lighting():
    m = _svg()
    assert _opts(m)["faces"] == {"image": True}
    assert _opts(m)["paper"] == {"area": "view", "size": "screen"}
    for i in range(2, 5):
        m.receive("fill", str(i))
        assert _opts(m)["faces"] == {
            "materials": i != 4,
            "textures": i == 2,
            "as_view": True,
            "shadows": False,
        }
    m.receive("as_view", "0")
    assert not _opts(m)["faces"]["as_view"]
    m.receive("fill", "0")
    assert _opts(m)["faces"] is None and m.field("as_view") is None


def test_svg_restores_the_last_choice():
    m = _svg()
    m.receive("fill", "4")
    m.receive("as_view", "0")
    assert _opts(_svg(prefs=_saved(m))) == _opts(m)


def test_default_colour_only_drops_every_material_and_texture(tmp_path):
    """Painted, textured, see-through: with «Default colour only» every
    face comes out in IngeTrazo's default colour, lit as the view or flat,
    opaque, with no image — a clean model to colour elsewhere."""
    from core.group import Group
    from core.mesh import Mesh
    from core.scene import Scene
    from core.style import DEFAULT_FACE_COLOR
    from core.vista_caras import shade_factor, shaded, view_faces
    from formats.svg_out import save_svg_layers
    scene = Scene()
    g = Group(Mesh(), name="caja")
    _box(g.mesh)
    g.material = {"color": (0.0, 0.0, 1.0), "opacity": 0.4}
    top = next(f for f in g.mesh.faces if f.normal().z() > 0.5)
    top.attrs["texture"] = {"path": _png(tmp_path / "t.png"), "sw": 1.0,
                            "sh": 1.0}
    scene.groups.append(g)
    flat = view_faces(scene, _camera(scene), as_view=False,
                      materials=False, textures=False)
    assert len(flat) == 3
    assert {f.rgb for f in flat} == {tuple(DEFAULT_FACE_COLOR)}
    assert all(f.texture is None and f.opacity == 1.0 for f in flat)
    lit = view_faces(scene, _camera(scene), materials=False,
                     textures=False)
    tones = {f.rgb for f in lit}
    assert len(tones) == 3
    assert shaded(DEFAULT_FACE_COLOR, shade_factor((0, 0, 1))) in tones
    out = tmp_path / "limpio.svg"
    save_svg_layers(out, [], faces=lit)
    assert "data:image" not in out.read_text(encoding="utf-8")


def _slab(mesh, x0, y0, z0, sx, sy, sz):
    """A closed box sx x sy x sz, faces wound outwards."""
    V = QVector3D
    x1, y1, z1 = x0 + sx, y0 + sy, z0 + sz
    for loop in (
            [(x0, y0, z0), (x0, y1, z0), (x1, y1, z0), (x1, y0, z0)],
            [(x0, y0, z1), (x1, y0, z1), (x1, y1, z1), (x0, y1, z1)],
            [(x0, y0, z0), (x1, y0, z0), (x1, y0, z1), (x0, y0, z1)],
            [(x0, y1, z0), (x0, y1, z1), (x1, y1, z1), (x1, y1, z0)],
            [(x0, y0, z0), (x0, y0, z1), (x0, y1, z1), (x0, y1, z0)],
            [(x1, y0, z0), (x1, y1, z0), (x1, y1, z1), (x1, y0, z1)]):
        mesh.add_face([V(*p) for p in loop])


@pytest.mark.parametrize("perspective", [False, True])
def test_a_thin_slab_shows_its_top_not_its_underside(perspective):
    """Sorted by mean depth, the underside of a thin, wide slab came out
    nearer than its top and hid it (the test house's roof slope)."""
    from core.scene import Scene
    from core.vista_caras import view_faces
    scene = Scene()
    _slab(scene.loose_mesh, 0, 0, 3, 8.0, 4.0, 0.05)
    cam = _camera(scene)
    cam.perspective = perspective
    faces = view_faces(scene, cam, as_view=False)
    assert not any(f.back for f in faces)
    top = max(faces, key=_ring_area)
    total = sum(_ring_area(f) for f in faces)
    assert _ring_area(top) > 0.8 * total


def test_a_box_at_the_far_end_of_a_big_floor_stays_in_view():
    """A floor reaching toward the eye has its mean depth nearer than a box
    standing at its far end; the box must still show, the floor round it."""
    from core.scene import Scene
    from core.vista_caras import view_faces
    V = QVector3D
    scene = Scene()
    scene.loose_mesh.add_face([V(-20, -20, 0), V(2, -20, 0), V(2, 2, 0),
                               V(-20, 2, 0)])
    _slab(scene.loose_mesh, 0, 0, 0, 1, 1, 1)
    faces = view_faces(scene, _camera(scene), as_view=False)
    assert len(faces) == 4                       # floor + three box faces
    floor = max(faces, key=_ring_area)
    box = sum(_ring_area(f) for f in faces if f is not floor)
    assert box == pytest.approx(0.5 + 2 * 0.6124, rel=0.01)   # top, sides


def test_two_faces_that_cross_each_show_the_part_in_front():
    """An X seen from the front: each upright face is in front of the other
    on one side of the crossing — no order by depth can draw that."""
    from core.scene import Scene
    from core.vista_caras import view_faces
    V = QVector3D
    scene = Scene()
    scene.loose_mesh.add_face([V(-1, -1, 0), V(1, 1, 0), V(1, 1, 1),
                               V(-1, -1, 1)])
    scene.loose_mesh.add_face([V(-1, 1, 0), V(1, -1, 0), V(1, -1, 1),
                               V(-1, 1, 1)])
    faces = view_faces(scene, _camera(scene, "front"), as_view=False)
    assert len(faces) == 2
    areas = sorted(_ring_area(f) for f in faces)
    assert areas == pytest.approx([1.0, 1.0], rel=1e-3)   # half of 2 x 1 each


def test_in_perspective_the_texture_lands_where_the_camera_puts_it():
    """Seen in perspective a textured wall's map to the drawing is
    projective: any point of the wall, taken through its texture
    coordinates, must land where the camera projects it — an affine guess
    from three points broke the brick courses at every piece of wall."""
    from ingetrazo_core import ScreenPoint as QPointF
    from core.scene import Scene
    from core.texture import face_uv_axes
    from core.vista_caras import _Perspective, view_faces
    from formats.svg_out import _uv_to_page
    V = QVector3D
    scene = Scene()
    wall = scene.loose_mesh.add_face([V(0, 0, 0), V(6, 0, 0), V(6, 0, 3),
                                      V(0, 0, 3)])
    tex = {"path": _png(_tmp_png_dir() / "t.png"), "sw": 0.5, "sh": 0.25}
    wall.attrs["texture"] = tex
    cam = _camera(scene)
    cam.perspective = True
    cam.distance *= 0.6                        # strong perspective
    faces = view_faces(scene, cam, as_view=False)
    assert len(faces) == 1
    pins = faces[0].texture["uv"]
    assert len(pins) == 4
    t = _uv_to_page(pins, lambda x: x, lambda y: y)
    proj = _Perspective(cam)
    gu, cu, gv, cv = face_uv_axes(tex, wall.normal())
    for wx, wz in ((0.2, 0.2), (5.8, 0.3), (5.5, 2.9), (0.3, 2.7),
                   (3.1, 1.4)):
        p = V(wx, 0, wz)
        u = QVector3D.dotProduct(gu, p) + cu
        v = QVector3D.dotProduct(gv, p) + cv
        got = t.map(QPointF(u, -v))
        want = proj.point(np.array([wx, 0.0, wz]))
        assert got.x() == pytest.approx(want[0], abs=1e-6), (wx, wz)
        assert got.y() == pytest.approx(want[1], abs=1e-6), (wx, wz)


def _tmp_png_dir():
    import tempfile
    from pathlib import Path
    return Path(tempfile.mkdtemp())


def test_default_colour_only_also_inks_every_line_black(tmp_path, native_host):
    win = native_host.window
    vp = win.viewport
    native_host.resize(800, 600, 1.)
    vp.camera.set_aspect(800, 600)
    _two_layer_scene(vp.scene)
    vp.notify_scene_changed()
    lo, hi = vp.scene.bounds()
    vp.camera.fit_to(lo, hi, 1.2)
    vp.camera.perspective = False
    strokes = {}
    for name, fill in (("color", "2"), ("defecto", "4")):
        _export_sheet(win, tmp_path / name, fill)
        _root, layers = _layers(tmp_path / f"{name}.svg")
        strokes[name] = {g.get("stroke") for n, g in layers.items()
                         if "-CARAS" not in n}
    assert "#0000ff" in strokes["color"]           # Mobiliario, blue
    assert strokes["defecto"] == {"#000000"}


# ---- Sun shadows, picture of the view, paper, preview ----------------------

def _sunny(scene):
    scene.shadows.enabled = True
    scene.shadows.month, scene.shadows.day = 6, 21
    scene.shadows.hour, scene.shadows.minute = 10, 0
    from core.sun import scene_sun_direction
    return np.array(scene_sun_direction(scene))


def test_the_ground_shadow_falls_away_from_the_sun():
    from core.scene import Scene
    from core.vista_caras import GROUND_LAYER, _Parallel, view_faces
    scene = Scene()
    _slab(scene.loose_mesh, 0, 0, 0, 1, 1, 3)        # a 3 m post
    d = _sunny(scene)
    cam = _camera(scene, "top")
    faces = view_faces(scene, cam, shadows=True)
    ground = [f for f in faces if f.extra.get("ground_shadow")]
    assert len(ground) == 1 and ground[0].layer == GROUND_LAYER
    # Its far end is where the post's top throws it.
    tip = np.array([0.5, 0.5, 3.0]) - d * (3.0 / d[2])
    want = _Parallel(cam).point(tip)
    pts = np.concatenate(ground[0].rings)
    far = pts[np.argmax(pts @ (want / np.linalg.norm(want)))]
    assert np.linalg.norm(far - want) < 0.75
    # Without shadows asked, or with the document's off: none.
    assert not any(f.extra.get("ground_shadow")
                   for f in view_faces(scene, cam))
    scene.shadows.enabled = False
    assert not any(f.extra.get("ground_shadow") or f.shadow
                   for f in view_faces(scene, cam, shadows=True))


def test_a_post_shades_part_of_a_slab_and_a_wall_turned_away_is_dark():
    """A slab with a post standing beside it on the sun's side: part of
    the slab's top is in the post's shadow, not all of it; the post's side
    turned from the sun is in shadow whole."""
    from core.scene import Scene
    from core.vista_caras import view_faces
    scene = Scene()
    d = _sunny(scene)
    _slab(scene.loose_mesh, -3, -3, 0, 6, 6, 0.2)    # the slab
    # The post on the sun's side, so its shadow crosses the slab.
    ox, oy = -0.5 + 2.0 * d[0] / np.hypot(d[0], d[1]), \
        -0.5 + 2.0 * d[1] / np.hypot(d[0], d[1])
    _slab(scene.loose_mesh, ox, oy, 0.2, 1, 1, 3)
    faces = view_faces(scene, _camera(scene, "iso"), shadows=True)
    top = max((f for f in faces if not f.extra.get("ground_shadow")),
              key=_ring_area)                       # the slab's top
    assert top.shadow
    dark = sum(abs(_area_of(r)) for r in top.shadow)
    assert 0.02 * _ring_area(top) < dark < 0.6 * _ring_area(top)
    assert 0.0 < top.shadow_alpha < 1.0
    turned = [f for f in faces if f.extra.get("seen_normal") is not None
              and float(np.asarray(f.extra["seen_normal"]) @ d) < 0]
    assert turned and all(f.shadow for f in turned)


def _area_of(r):
    from core.vista_caras import _area
    return _area(r)


def test_the_shadows_go_out_on_layers_of_their_own(tmp_path):
    from core.scene import Scene
    from core.vista_caras import view_faces
    from formats.svg_out import save_svg_layers
    scene = Scene()
    _slab(scene.loose_mesh, 0, 0, 0, 1, 1, 3)
    _sunny(scene)
    faces = view_faces(scene, _camera(scene), shadows=True)
    out = tmp_path / "sombras.svg"
    save_svg_layers(out, [], faces=faces)
    _root, layers = _layers(out)
    assert "SOMBRAS-SUELO" in layers and "Layer 0-SOMBRAS" in layers
    names = list(layers)
    assert names.index("Layer 0-CARAS") < names.index("Layer 0-SOMBRAS")
    veil = layers["Layer 0-SOMBRAS"].find(SVG + "path")
    assert veil.get("fill") == "#000000"
    assert float(veil.get("fill-opacity")) == pytest.approx(
        1.0 - scene.shadows.darkness, abs=0.01)


def test_a_sheet_of_paper_takes_the_drawing_fitted_and_centred():
    from formats.svg_out import svg_text
    import xml.etree.ElementTree as ET2
    groups = [("PLANTA", np.array([(0, 0, 8, 0), (8, 0, 8, 2)], float))]
    text, _n = svg_text(groups, paper=(297.0, 210.0))
    root = ET2.fromstring(text)
    assert root.get("width") == "297.000mm"
    assert root.get("height") == "210.000mm"
    # 8 m across 277 mm of sheet: 1:28.9 — said as 1:29.
    assert root.find(SVG + "desc").text == "1:29"
    xs = [p[0] for el in root.iter(SVG + "path") for p in _points(el)]
    assert min(xs) == pytest.approx(10.0, abs=0.01)
    assert max(xs) == pytest.approx(287.0, abs=0.01)


def test_the_picture_of_the_view_lies_under_the_lines_on_its_window():
    from formats.svg_out import svg_text
    import xml.etree.ElementTree as ET2
    img = solid_raster(40, 20, (10, 120, 200))
    groups = [("VISTA", np.array([(1, 1, 3, 1)], float))]
    text, _n = svg_text(groups, scale_n=100.0,
                        image=(img, (0.0, 0.0, 4.0, 2.0)))
    root = ET2.fromstring(text)
    layers = [g.get(INK + "label") for g in root.findall(SVG + "g")]
    assert layers[0] == "VISTA-IMAGEN" and "VISTA" in layers
    pic = root.find(f"{SVG}g/{SVG}image")
    assert float(pic.get("width")) == pytest.approx(40.0)    # 4 m at 1:100
    assert float(pic.get("height")) == pytest.approx(20.0)
    assert root.get("width") == "60.000mm"           # the window + margin


@pytest.mark.parametrize("perspective", [False, True])
def test_the_window_rect_holds_what_the_screen_shows(perspective):
    from core.scene import Scene
    from core.vista_caras import _Parallel, _Perspective, view_rect
    scene = Scene()
    _slab(scene.loose_mesh, 0, 0, 0, 2, 2, 2)
    cam = _camera(scene)
    cam.perspective = perspective
    x0, y0, x1, y1 = view_rect(cam)
    assert x1 > x0 and y1 > y0
    assert (x1 - x0) / (y1 - y0) == pytest.approx(cam.aspect, rel=1e-6)
    # The orbit target sits in the middle of the window.
    t = cam.target
    proj = _Perspective(cam) if perspective else _Parallel(cam)
    mid = proj.point(np.array([t.x(), t.y(), t.z()]))
    assert mid[0] == pytest.approx((x0 + x1) / 2, abs=1e-6)


def test_the_builder_works_each_thing_out_once():
    from ingetrazo_render.view_export import SvgBuilder
    counts = {"lines": 0, "faces": 0}

    def lines():
        counts["lines"] += 1
        return [("VISTA", np.array([(0, 0, 1, 0)], float))]

    def faces(**o):
        counts["faces"] += 1
        return []

    b = SvgBuilder(lines, faces)
    opts = {"materials": True, "textures": True, "as_view": True,
            "shadows": False}
    b.build(opts, None, preview=True)
    b.build(opts, (297.0, 210.0), preview=True)
    b.build(dict(opts), (420.0, 297.0))
    assert counts == {"lines": 1, "faces": 1}
    b.build({**opts, "as_view": False}, None)
    assert counts["faces"] == 2


def test_the_screen_size_sheet_is_the_window_edge_to_edge():
    """«Screen size»: the page is what the window shows, at its size, with
    no margin — and what lies past it is cut off at the page."""
    import xml.etree.ElementTree as ET2
    from ingetrazo_render.view_export import SvgBuilder
    lines = [("VISTA", np.array([(-5, 0, 5, 0), (0, -1, 0, 1)], float))]
    b = SvgBuilder(lambda: lines, lambda **o: [],
                   window=lambda: (-2.0, -1.0, 2.0, 1.0),
                   screen_mm=(200.0, 100.0))
    text, _n = b.build(None, "screen")
    root = ET2.fromstring(text)
    assert root.get("width") == "200.000mm"
    assert root.get("height") == "100.000mm"
    assert root.find(f"{SVG}defs/{SVG}clipPath") is not None
    layer = next(g for g in root.findall(SVG + "g")
                 if g.get(INK + "label") == "VISTA")
    assert layer.get("clip-path") == "url(#pagina)"
    # 4 m across 200 mm: the vertical stroke at x = 0 sits in the middle.
    xs = {round(p[0], 3) for el in layer.iter(SVG + "path")
          for p in _points(el)}
    assert 100.0 in xs
    # The drawing at its scale and the sheets keep working.
    assert ET2.fromstring(b.build(None, None)[0]).get("width") != "200.000mm"


def test_the_picture_of_the_view_can_leave_the_axes_out(tmp_path):
    from ingetrazo_render.view_export import SvgBuilder
    m = _svg(axes=True)
    assert _opts(m)["faces"] == {"image": True, "axes": False}
    m.receive("axes", "1")
    assert _opts(m)["faces"] == {"image": True}
    m.receive("fill", "2")
    assert m.field("axes") is None
    assert _svg().field("axes") is None
    # The builder asks for the picture with or without them, each once.
    asked = []

    def image(width, axes=True):
        asked.append((width, axes))
        return None

    b = SvgBuilder(lambda: [], lambda **o: [], image)
    b.build({"image": True, "axes": False}, None, preview=True)
    b.build({"image": True, "axes": False}, None, preview=True)
    b.build({"image": True}, None, preview=True)
    assert [a for _w, a in asked] == [False, True]


def test_the_view_picture_without_axes_leaves_the_view_as_it_was(
        native_host, monkeypatch):
    win = native_host.window
    vp = win.viewport
    seen = []
    monkeypatch.setattr(vp.keys_stand_in, "render_pixels", lambda *a, **k: seen.append(
        vp.scene.show_axes))
    vp.scene.show_axes = True
    builder = win.view_svg_builder()
    builder.image(True, axes=False)
    builder.image(True, axes=True)
    assert seen == [False, True]
    assert vp.scene.show_axes is True


def test_svg_preview_can_be_turned_off_and_remembers_it():
    m = _svg()
    m.receive("svg_preview", "0")
    assert m.preview_ticket("changed") is None
    again = _svg(prefs=_saved(m))
    assert not again.preview_enabled
    again.receive("svg_preview", "1")
    assert again.begin_preview(again.preview_ticket("changed"))


def test_the_sheet_takes_the_part_and_the_size_apart():
    """What you see on an A3 widens the window to the sheet's shape round
    its middle, edge to edge; the whole model on A3 fits with a margin;
    at the drawing's scale the sheet is as big as it needs."""
    import xml.etree.ElementTree as ET2
    from ingetrazo_render.view_export import SvgBuilder
    lines = [("VISTA", np.array([(-5, 0, 5, 0), (0, -1, 0, 1)], float))]
    b = SvgBuilder(lambda: lines, lambda **o: [],
                   window=lambda: (-2.0, -1.0, 2.0, 1.0),
                   screen_mm=(200.0, 100.0))

    def page(sheet):
        root = ET2.fromstring(b.build(None, sheet)[0])
        return (float(root.get("width").rstrip("mm")),
                float(root.get("height").rstrip("mm")),
                root.find(SVG + "desc").text,
                root.find(f"{SVG}defs/{SVG}clipPath") is not None)

    # What you see, like the screen: 4 m across 200 mm, cut at the page.
    assert page({"area": "view", "size": "screen"}) == (200.0, 100.0,
                                                         "1:20", True)
    # What you see on A3 landscape: the 4 x 2 m window (2:1) grows to
    # 420:297 round its middle — taller, its 4 m across 420 mm.
    w, h, scale, clipped = page({"area": "view", "size": (420.0, 297.0)})
    assert (w, h, clipped) == (420.0, 297.0, True)
    assert scale == "1:9.5"
    # The whole model on A3: 10 m across 400 mm, no cutting.
    w, h, scale, clipped = page({"area": "model", "size": (420.0, 297.0)})
    assert (w, h, scale, clipped) == (420.0, 297.0, "1:25", False)
    # The whole model at the drawing's scale: 10 m at 1:100 + margins.
    assert page({"area": "model", "size": None})[:2] == (120.0, 40.0)
    # What you see at the drawing's scale: the window at 1:100, no margin.
    assert page({"area": "view", "size": None})[:2] == (40.0, 20.0)


def test_the_selection_part_exports_only_what_is_selected(tmp_path):
    """«Selection»: only the selected objects, as if the rest were hidden —
    and the model is left as it was."""
    from core.group import Group
    from core.mesh import Mesh
    from core.scene import Scene
    from core.vista_caras import view_faces
    from ingetrazo_render.view_export import SvgBuilder, selection_only
    scene = Scene()
    a, b = Group(Mesh(), name="a"), Group(Mesh(), name="b")
    _slab(a.mesh, 0, 0, 0, 1, 1, 1)
    _slab(b.mesh, 3, 0, 0, 1, 1, 1)
    scene.groups[:] = [a, b]
    scene.selection.add(b)
    cam = _camera(scene)
    everything = view_faces(scene, cam, as_view=False)
    with selection_only(scene):
        only_b = view_faces(scene, cam, as_view=False)
    assert len(everything) == 6 and len(only_b) == 3
    assert scene.entity_visible(a)                  # back as it was
    # Through the builder: the faces with the selection alone, cached apart.
    built = []

    def faces(**o):
        built.append(scene.entity_visible(a))
        return []

    builder = SvgBuilder(lambda fast=True: [], faces,
                         isolate=lambda: selection_only(scene))
    opts = {"materials": True, "textures": True, "as_view": True,
            "shadows": False}
    builder.build(opts, {"area": "selection", "size": None})
    builder.build(opts, {"area": "model", "size": None})
    assert built == [False, True]
    m = _svg(selection=True)
    m.receive("area", "2")
    for fill in ("2", "1"):
        m.receive("fill", fill)
        assert _opts(m)["paper"]["area"] == "selection"
    none = _svg(selection=False)
    none.receive("area", "2")
    assert len(_rows(none)["area"][7]) == 2 and _opts(none)["paper"]["area"] == "view"


@pytest.mark.parametrize("perspective", [False, True])
def test_a_sheet_of_another_shape_shows_what_lay_past_the_screen(
        perspective):
    """«What you see» on a sheet of another shape: the view as in a window
    of that shape — wider shows more at the sides with the same height,
    taller shows more above and below with the same width — and the
    camera is left as it was."""
    from core.scene import Scene
    from core.vista_caras import view_rect
    from ingetrazo_render.view_export import widened
    scene = Scene()
    _slab(scene.loose_mesh, 0, 0, 0, 2, 2, 2)
    cam = _camera(scene)
    cam.perspective = perspective
    before = (cam.aspect, cam.distance, cam.fov_deg)
    x0, y0, x1, y1 = view_rect(cam)
    w, h = x1 - x0, y1 - y0
    with widened(cam, cam.aspect * 2.0):
        a0, b0, a1, b1 = view_rect(cam)
    assert (b1 - b0) == pytest.approx(h, rel=1e-6)
    assert (a1 - a0) == pytest.approx(2.0 * w, rel=1e-6)
    with widened(cam, cam.aspect / 2.0):
        a0, b0, a1, b1 = view_rect(cam)
    assert (a1 - a0) == pytest.approx(w, rel=1e-6)
    assert (b1 - b0) == pytest.approx(2.0 * h, rel=1e-6)
    assert (cam.aspect, cam.distance, cam.fov_deg) == before


def test_the_builder_works_a_new_shape_out_in_the_widened_view():
    from ingetrazo_render.view_export import SvgBuilder
    inside = []

    class Widen:
        def __init__(self, aspect):
            self.aspect = aspect

        def __enter__(self):
            inside.append(self.aspect)

        def __exit__(self, *_a):
            inside.append(None)

    shapes = []

    def image(width, axes=True, aspect=None):
        shapes.append(aspect)
        return None, (0.0, 0.0, 2.0 * (aspect or 1.0), 2.0)

    b = SvgBuilder(lambda fast=True: [], lambda **o: [], image,
                   window=lambda: (0.0, 0.0, 2.0, 2.0),
                   screen_mm=(100.0, 100.0), widen=Widen)
    b.build({"image": True}, {"area": "view", "size": (300.0, 100.0)},
            preview=True)
    assert shapes == [3.0] and 3.0 in inside
    # The screen's own shape, or the whole model: nothing to widen.
    inside.clear()
    b.build({"image": True}, {"area": "view", "size": (200.0, 200.0)},
            preview=True)
    b.build(None, {"area": "model", "size": (300.0, 100.0)})
    assert inside == []


# ---- The sheet: on screen, custom (paper sheet, proportion, mm), at scale --


def test_only_custom_paper_enables_its_dimensions():
    assert len(_rows(_svg())["view_size"][7]) == 2
    m = _svg(frame_scale=50)
    assert "50" in _rows(m)["view_size"][7][2]
    assert _rows(m)["width"][8] != 0 and m.field("paper_preset") is None
    m.receive("view_size", "1")
    assert _rows(m)["width"][8] == 0
    m.receive("paper_preset", "2")
    assert _opts(m)["paper"]["size"] == [420.0, 297.0] and _rows(m)["paper_ratio"][8] != 0
    m.receive("view_size", "2")
    assert m.field("width") is None and _opts(m)["paper"]["size"] is None


def test_preview_renders_the_requested_page_and_remembers_it():
    from ingetrazo_render.view_export import SvgBuilder
    from ingetrazo_render import svg_preview

    m = _svg(shadows=True, selection=True)
    for key, value in [
        ("area", "2"),
        ("view_size", "1"),
        ("paper_ratio", "0"),
        ("width", "500"),
        ("height", "250"),
        ("fill", "2"),
        ("shadows", "1"),
    ]:
        m.receive(key, value)
    opts = _opts(m)
    builder = SvgBuilder(lambda fast=True: [], lambda **o: [])
    text, _ = builder.build(opts["faces"], opts["paper"], preview=True)
    w, h, rgba = svg_preview(text)
    assert (w, h, len(rgba)) == (800, 400, 800 * 400 * 4)
    assert _opts(_svg(prefs=_saved(m), shadows=True, selection=True)) == opts


# ---- Framing: drag and wheel on the preview ---------------------------------

@pytest.mark.parametrize("perspective", [False, True])
def test_the_framing_moves_and_zooms_the_view_and_leaves_it_as_it_was(
        perspective):
    from core.scene import Scene
    from core.vista_caras import _Parallel, _Perspective, view_rect
    from ingetrazo_render.view_export import framed
    scene = Scene()
    _slab(scene.loose_mesh, 0, 0, 0, 2, 2, 2)
    cam = _camera(scene)
    cam.perspective = perspective
    before = (cam.target.x(), cam.target.y(), cam.target.z(),
              cam.distance, cam.fov_deg)
    x0, y0, x1, y1 = view_rect(cam)
    w = x1 - x0
    t = np.array([cam.target.x(), cam.target.y(), cam.target.z()])

    def seen(c):
        proj = _Perspective(c) if perspective else _Parallel(c)
        return proj.point(t)

    # Half a window to the right: the old middle now sits half a window
    # to the left of the new one.
    with framed(cam, (0.5, 0.0, 1.0)):
        assert seen(cam)[0] == pytest.approx(-0.5 * w, rel=1e-6)
    # Twice as close: half as much across.
    with framed(cam, (0.0, 0.0, 2.0)):
        a0, _b0, a1, _b1 = view_rect(cam)
        assert (a1 - a0) == pytest.approx(w / 2.0, rel=1e-6)
    assert (cam.target.x(), cam.target.y(), cam.target.z(),
            cam.distance, cam.fov_deg) == before


def test_the_builder_frames_only_what_the_screen_shows():
    from ingetrazo_render.view_export import NO_FRAME, SvgBuilder
    asked = []

    class Frame:
        def __init__(self, frame):
            self.frame = frame

        def __enter__(self):
            asked.append(self.frame)

        def __exit__(self, *_a):
            pass

    b = SvgBuilder(lambda fast=True: [], lambda **o: [],
                   window=lambda: (0.0, 0.0, 2.0, 1.0),
                   screen_mm=(200.0, 100.0), frame=Frame)
    assert b.can_frame()
    b.build(None, {"area": "view", "size": "screen",
                   "frame": (0.25, 0.0, 2.0)})
    assert (0.25, 0.0, 2.0) in asked
    asked.clear()
    b.build(None, {"area": "view", "size": "screen", "frame": NO_FRAME})
    b.build(None, {"area": "model", "size": None,
                   "frame": (0.25, 0.0, 2.0)})
    assert asked == []
    assert not SvgBuilder(lambda: [], lambda **o: []).can_frame()


def test_drag_and_zoom_reframe_only_the_current_view():
    m = _svg(selection=True)
    m.move_frame(0.25, 0, 1)
    m.move_frame(0, 0, 1.15)
    assert _opts(m)["paper"]["frame"] == pytest.approx([-0.25, 0, 1.15])
    m.receive("framed", "0")
    assert "frame" not in _opts(m)["paper"]
    m.receive("area", "2")
    m.move_frame(1, 1, 2)
    assert "frame" not in _opts(m)["paper"]


# ---- Composition guides over the preview ------------------------------------


def test_preview_guides_persist_without_changing_export():
    m = _svg()
    assert len(_rows(m)["svg_guides"][7]) == 6
    opts = m.options_json()
    m.receive("svg_guides", "2")
    assert m.options_json() == opts
    assert _svg(prefs=_saved(m)).preview_block()[1] == "Golden section"


# ---- The basic dialog --------------------------------------------------------


def test_first_export_has_basic_choices_and_read_only_screen_size():
    m = _svg(prefs={})
    rows = _rows(m)
    assert rows["advanced"][4] == 0 and len(rows["basic_fill"][7]) == 4
    assert [rows[k][4] for k in ("width", "height")] == [278.0, 162.0]
    assert all(rows[k][8] for k in ("width", "height"))
    assert _opts(m)["faces"] == {"image": True}
    m.receive("basic_fill", "2")
    assert not _opts(m)["faces"]["textures"]
    m.receive("basic_fill", "3")
    assert not _opts(m)["faces"]["materials"]


def test_basic_custom_dimensions_lock_the_current_ratio():
    m = _svg(advanced=False)
    m.receive("view_size", "1")
    for key, value, size in [
        ("width", "556", [556, 324]),
        ("basic_lock", "0", [556, 324]),
        ("height", "100", [556, 100]),
        ("basic_lock", "1", [556, 100]),
        ("width", "278", [278, 50]),
        ("view_size", "0", "screen"),
    ]:
        m.receive(key, value)
        assert _opts(m)["paper"]["size"] == size


def test_legacy_basic_custom_size_uses_its_own_lock():
    model = _svg(prefs={"advanced": False, "basic_size": "custom", "basic_width": 300,
                        "basic_height": 200, "basic_lock": False, "ratio": "screen"})
    model.receive("height", "250")
    assert _opts(model)["paper"]["size"] == [300, 250]


def test_basic_and_advanced_share_face_choice_and_dimensions():
    m = _svg(advanced=False)
    for key, value in [
        ("view_size", "1"),
        ("basic_lock", "0"),
        ("width", "300"),
        ("height", "200"),
        ("advanced", "1"),
    ]:
        m.receive(key, value)
    assert _opts(m)["paper"] == {"area": "view", "size": [300.0, 200.0]}
    m.receive("fill", "3")
    m.receive("advanced", "0")
    assert _rows(m)["basic_fill"][4] == 2
    m.receive("advanced", "1")
    m.receive("fill", "0")
    m.receive("advanced", "0")
    assert _rows(m)["basic_fill"][4] == 1
    m.receive("advanced", "1")
    assert _rows(_svg(prefs=_saved(m)))["advanced"][4] == 1


def test_basic_svg_never_schedules_a_preview():
    m = _svg(advanced=False)
    m.receive("view_size", "1")
    assert not m.preview_enabled and m.preview_ticket("new page") is None


def test_both_modes_start_with_a_picture():
    for advanced in (True, False):
        assert _opts(_svg(advanced=advanced))["faces"] == {"image": True}


# ---- Scenes ------------------------------------------------------------------


def test_svg_scene_selection_is_shared_by_both_modes_and_resets_framing():
    assert _svg().field("svg_scene") is None
    calls = []
    m = _svg(scenes=["Plan", "Facade"], switch=calls.append)
    assert _rows(m)["svg_scene"][7][1:] == ["Plan", "Facade"]
    m.move_frame(0.2, 0.1, 2)
    m.receive("svg_scene", "2")
    assert "frame" not in _opts(m)["paper"]
    m.receive("advanced", "0")
    assert _rows(m)["svg_scene"][4] == 2
    m.receive("svg_scene", "1")
    m.receive("svg_scene", "0")
    assert calls == [1, 0, None]


def test_a_scene_is_exported_and_the_view_comes_back(native_host, tmp_path):
    from core.saved_views import SavedView
    win = native_host.window
    vp = win.viewport
    native_host.resize(800, 600, 1.)
    vp.camera.set_aspect(800, 600)
    _two_layer_scene(vp.scene)
    vp.notify_scene_changed()
    lo, hi = vp.scene.bounds()
    vp.camera.fit_to(lo, hi, 1.2)
    vp.camera.perspective = False
    # A scene without the furniture, looking from elsewhere.
    mob = next(ly for ly in vp.scene.layers if ly.name == "Mobiliario")
    mob.visible = False
    yaw = vp.camera.yaw
    vp.camera.yaw = yaw + 1.0
    vp.scene.saved_views.append(
        SavedView.capture("Sin muebles", vp.scene, vp.camera))
    mob.visible = True
    vp.camera.yaw = yaw
    saved_version = vp.scene.content_version          # nothing to save

    def choose(model):
        assert _rows(model)["svg_scene"][7][-1] == "Sin muebles"
        model.receive("svg_scene", "1")
        assert not mob.visible

    from ingetrazo_render import chrome_sheet
    from core.settings import settings as current_settings

    settings = current_settings()
    original_mode = settings.value("export_svg/v5/advanced")
    form = win._on_export_view_svg()
    ctx = chrome_sheet.Ctx(win, None)
    model = chrome_sheet.current(win).build(ctx).forms()[1]
    model.receive("advanced", "1")
    choose(model)
    form.set("destination", str(tmp_path / "cancelled.svg"))
    chrome_sheet.close(win)
    assert mob.visible and vp.camera.yaw == yaw
    assert not (tmp_path / "cancelled.svg").exists()
    assert settings.value("export_svg/v5/advanced") == original_mode

    _export_sheet(win, tmp_path / "escena", scene=choose)
    _root, layers = _layers(tmp_path / "escena.svg")
    names = " ".join(layers)
    assert "Muros" in names and "Mobiliario" not in names
    # The view as it was, and the document still unchanged.
    assert mob.visible and vp.camera.yaw == yaw
    assert vp.scene.content_version == saved_version


def test_the_whole_model_is_all_of_it_not_what_the_screen_shows(native_host):
    """Zoomed in on a corner, «Whole model» still exports every part of
    the model — framed afresh, as Zoom Extents frames it — and leaves the
    camera as it was."""
    win = native_host.window
    vp = win.viewport
    native_host.resize(800, 600, 1.)
    vp.camera.set_aspect(800, 600)
    _two_layer_scene(vp.scene)
    vp.notify_scene_changed()
    lo, hi = vp.scene.bounds()
    whole = {"area": "model", "size": None}
    for perspective in (False, True):
        vp.camera.perspective = perspective
        vp.camera.two_point = False
        vp.camera.fit_box(lo, hi)
        _t, everything = win.view_svg_builder().build(None, whole)
        # Close in on one corner: the screen shows a part of it.
        vp.camera.target = lo
        vp.camera.distance = vp.camera.distance / 4.0
        keep = (vp.camera.target, vp.camera.distance)
        builder = win.view_svg_builder()
        _t, on_screen = builder.build(None, {"area": "view",
                                             "size": "screen"})
        text, n = builder.build({"textures": True, "as_view": True},
                                whole)
        if perspective:              # only what the window shows
            assert on_screen < everything
        assert n == everything, perspective
        assert "Mobiliario-CARAS" in text and "Muros-CARAS" in text
        assert (vp.camera.target, vp.camera.distance) == keep


def test_the_picture_of_the_selection_shows_the_selection_alone(native_host, monkeypatch):
    """«Picture of the view» with «Selection»: the rest of the model is
    hidden for the picture, framed on the selection, and every flag — and
    the document's «nothing to save» — is back afterwards."""
    win = native_host.window
    vp = win.viewport
    native_host.resize(400, 300, 1.)
    vp.camera.set_aspect(400, 300)
    _two_layer_scene(vp.scene)
    vp.notify_scene_changed()
    lo, hi = vp.scene.bounds()
    vp.camera.fit_box(lo, hi)
    a, b = vp.scene.groups
    vp.scene.selection.clear()
    vp.scene.selection.add(b)
    saved_version = vp.scene.content_version
    seen = []
    def render(*args, **kw):
        seen.append((a.hidden, b.hidden))
        return 40, 30, bytes([120, 100, 80, 255]) * 1200

    monkeypatch.setattr(vp.keys_stand_in, "render_pixels", render)
    text, _n = win.view_svg_builder().build(
        {"image": True}, {"area": "selection", "size": None})
    assert seen == [(True, False)]               # the other one hidden
    assert not a.hidden and not b.hidden
    assert vp.scene.content_version == saved_version


def test_a_documents_own_line_type_is_dashed_and_keeps_its_pen(tmp_path):
    """The fifth element of a group is a custom line type's dashes as the
    DXF takes them (gaps negative, drawing units at 1:100); the pen comes
    after it. Read as the pen, the dashes broke the export."""
    from formats.svg_out import save_svg_layers
    path = tmp_path / "v.svg"
    save_svg_layers(path, [("Eje", np.array([(0, 0, 2, 0)], float), "OWN",
                            None, (0.3, -0.1, 0.0, -0.1), 0.35)],
                    scale_n=50.0)
    _root, layers = _layers(path)
    assert layers["Eje"].get("stroke-dasharray") == "3 1 0 1"
    assert layers["Eje"].get("stroke-width") == "0.35"
