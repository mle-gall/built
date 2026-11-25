# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Drawing records, history, archive and sheet export on synthetic models."""
from __future__ import annotations

import built_core
from tests.shell_isolation import one_off_export, restore_shell_patches  # noqa: F401
import json
import random

import pytest
from built_core import Vec3 as QVector3D

from built_core import layers as L
from built_core.group import Group
from built_core.history import DeleteLayerCommand, History, MakeGroupCommand
from built_core.layers import DEFAULT_LAYER, Layer, assign_layer, layer_of
from built_core.mesh import Mesh
from built_core.saved_views import SavedView
from built_core.scene import Scene


def V(x, y, z=0.0):
    return QVector3D(x, y, z)


def _square(mesh, x0=0.0, y0=0.0, z=0.0):
    return mesh.add_face([V(x0, y0, z), V(x0 + 1, y0, z),
                          V(x0 + 1, y0 + 1, z), V(x0, y0 + 1, z)])


def test_layer_round_trips_every_field_and_drops_defaults():
    ly = Layer("Muros", visible=False, locked=True, color=(0.2, 0.4, 0.6),
               folder="Arquitectura", linetype="dashdot", lineweight=0.35,
               opacity=0.5)
    raw = ly.to_dict()
    back = Layer.from_dict(json.loads(json.dumps(raw)))
    assert (back.name, back.visible, back.locked, back.folder, back.linetype,
            back.lineweight, back.opacity) == (
        "Muros", False, True, "Arquitectura", "dashdot", 0.35, 0.5)
    assert back.color == pytest.approx((0.2, 0.4, 0.6))
    plain = Layer("Plain")
    assert plain.to_dict() == {"name": "Plain", "uid": plain.uid}
    assert back.uid == ly.uid


@pytest.mark.parametrize("raw", [
    {"name": "X", "color": "red"}, {"name": "X", "color": [1, 2]},
    {"name": "X", "linetype": "zigzag"}, {"name": "X", "lineweight": "thick"},
    {"name": "X", "opacity": None}, {"name": "X", "folder": 3},
])
def test_a_malformed_record_loads_with_defaults(raw):
    ly = Layer.from_dict(raw)
    assert ly.name == "X" and ly.linetype in L.LINETYPES
    assert 0.1 <= ly.opacity <= 1.0


def test_automatic_colour_is_stable_and_the_default_is_white():
    assert built_core.tag_styles.layer_color(Layer(DEFAULT_LAYER)) == L.DEFAULT_LAYER_COLOR
    a = built_core.tag_styles.layer_color(Layer("Estructura"))
    assert a == built_core.tag_styles.layer_color(Layer("Estructura")) and a in L.PALETTE
    assert built_core.tag_styles.layer_color(Layer("E", color=(1, 0, 0))) == (1, 0, 0)


def test_igz_keeps_active_layer_folders_and_looks(tmp_path):
    from formats import igz
    scene = Scene()
    scene.layers += [Layer("Muros", color=(0.1, 0.2, 0.3), folder="Arq",
                           linetype="dashed", lineweight=0.5, opacity=0.75)]
    scene.layer_folders = ["Arq", "Vacía"]
    scene.active_layer = "Muros"
    _square(scene.mesh)
    path = tmp_path / "capas.igz"
    igz.save_scene(scene, path)
    other = Scene()
    igz.load_into(other, path)
    ly = other.layer("Muros")
    assert other.active_layer == "Muros"
    assert other.layer_folders == ["Arq", "Vacía"]
    assert (ly.folder, ly.linetype, ly.lineweight, ly.opacity) == (
        "Arq", "dashed", 0.5, 0.75)
    # …and a document without them leaves nothing behind (#602).
    plain = tmp_path / "plain.igz"
    igz.save_scene(Scene(), plain)
    igz.load_into(other, plain)
    assert other.active_layer is None and other.layer_folders == []


def test_new_groups_go_to_the_active_layer_and_their_insides_do_not():
    scene = Scene()
    history = History(scene)
    scene.layers.append(Layer("Muebles"))
    built_core.tag_edits.set_active(scene, "Muebles")
    f = _square(scene.mesh)
    history.execute(MakeGroupCommand([f], list(scene.mesh.edges)))
    g = scene.groups[-1]
    assert layer_of(g) == "Muebles"
    assert all(layer_of(x) == DEFAULT_LAYER for x in g.mesh.faces)
    history.undo()
    history.redo()                                  # still on it after redo
    assert layer_of(scene.groups[-1]) == "Muebles"


def test_loose_lines_and_faces_stay_on_layer_0_and_their_group_does_not():
    """With a layer active, what is drawn loose stays on Layer 0 — a layer
    hidden never leaves half a solid (a box pushed up with another layer
    active would have had its sides there). Grouping it puts the group on
    the active layer, which is how drawing «on» a layer works."""
    from built_core.edits import build_add_edge
    from built_core.history import AddEdgeCommand, AddFaceCommand
    scene = Scene()
    history = History(scene)
    scene.layers.append(Layer("Muros"))
    built_core.tag_edits.set_active(scene, "Muros")
    history.execute(AddEdgeCommand(V(0, 0), V(5, 0)))
    history.execute(AddFaceCommand([V(10, 0), V(12, 0), V(12, 2), V(10, 2)]))
    history.execute(build_add_edge(scene, V(11, 0), V(11, 2)))   # a split
    assert all(layer_of(x) == DEFAULT_LAYER
               for x in list(scene.mesh.edges) + list(scene.mesh.faces))
    history.execute(MakeGroupCommand(list(scene.mesh.faces),
                                     list(scene.mesh.edges)))
    assert layer_of(scene.groups[-1]) == "Muros"


def test_no_active_layer_walks_no_group_and_tags_no_group():
    """With no layer active only the guides and dimensions are looked at
    (they go to their automatic layers, test_layer_guides): a model's
    thousands of groups are never walked before each command."""
    scene = Scene()
    scene.groups.append(Group(Mesh(), name="g"))
    history = History(scene)
    assert history._native.objects_before(history.scene) == set()
    f = _square(scene.mesh)
    history.execute(MakeGroupCommand([f], list(scene.mesh.edges)))
    assert layer_of(scene.groups[-1]) == DEFAULT_LAYER


def test_a_replacement_or_a_tagged_object_keeps_its_layer():
    scene = Scene()
    scene.layers += [Layer("A"), Layer("B")]
    old = Group(Mesh(), name="viejo")
    scene.groups.append(old)
    built_core.tag_edits.set_active(scene, "A")
    before = L.object_ids(scene)
    twin = Group(Mesh(), name="reemplazo")
    twin.uid = old.uid                     # Make Unique / Replace keep it
    pasted = Group(Mesh(), name="pegado")
    pasted.layer = "B"
    fresh = Group(Mesh(), name="nuevo")
    scene.groups[:] = [twin, pasted, fresh]
    assert L.tag_new_objects(scene, before) == 1
    assert (layer_of(twin), layer_of(pasted), layer_of(fresh)) == (
        DEFAULT_LAYER, "B", "A")


def test_hiding_or_locking_the_active_layer_hands_back_to_the_default():
    scene = Scene()
    scene.layers += [Layer("A"), Layer("B")]
    built_core.tag_edits.set_active(scene, "A")
    assert built_core.tag_edits.set_visible(scene, ["B"], False) is False
    assert built_core.tag_edits.set_visible(scene, ["A"], False) is True
    assert scene.active_layer is None
    built_core.tag_edits.set_active(scene, "A")                         # shows it again
    assert scene.layer("A").visible and scene.active_layer == "A"
    assert built_core.tag_edits.set_locked(scene, ["A"], True) is True
    assert L.active_layer(scene) == DEFAULT_LAYER


def test_rename_and_delete_follow_the_active_layer():
    scene = Scene()
    history = History(scene)
    scene.layers.append(Layer("A"))
    built_core.tag_edits.set_active(scene, "A")
    assert L.rename_layer(scene, "A", "Z")
    assert scene.active_layer == "A"                 # the key stays
    assert scene.layer(scene.active_layer).name == "Z"
    history.execute(DeleteLayerCommand("Z"))         # found by its label
    assert scene.active_layer is None
    history.undo()
    assert scene.active_layer == "A"


def test_isolate_keeps_the_default_layer_on():
    scene = Scene()
    scene.layers += [Layer("A"), Layer("B"), Layer("C")]
    built_core.tag_edits.isolate(scene, ["B"])
    assert [ly.visible for ly in scene.layers] == [True, False, True, False]


def test_merge_moves_everything_and_undoes():
    scene = Scene()
    history = History(scene)
    scene.layers += [Layer("A"), Layer("B")]
    f = _square(scene.mesh)
    g = Group(Mesh(), name="g")
    inner = _square(g.mesh)
    scene.groups.append(g)
    for e in (f, g, inner):
        assign_layer(e, "A")
    scene.saved_views.append(SavedView("S", hidden_layers=["A"]))
    built_core.tag_edits.set_active(scene, "A")
    history.execute(DeleteLayerCommand("A", into="B"))
    assert scene.layer("A") is None
    assert all(layer_of(e) == "B" for e in (f, g, inner))
    assert scene.active_layer == "B"
    history.undo()
    assert all(layer_of(e) == "A" for e in (f, g, inner))
    assert scene.active_layer == "A"
    assert scene.saved_views[0].hidden_layers == ["A"]


def test_merge_into_itself_or_nowhere_is_refused():
    scene = Scene()
    history = History(scene)
    scene.layers.append(Layer("A"))
    history.execute(DeleteLayerCommand("A", into="A"))
    history.execute(DeleteLayerCommand("A", into="Nope"))
    assert scene.layer("A") is not None and history.undo_stack == []


def test_folders_add_rename_move_and_remove():
    scene = Scene()
    scene.layers += [Layer("A"), Layer("B")]
    name = built_core.tag_edits.add_folder(scene, "Carpeta")
    assert name == "Carpeta 1" and built_core.tag_edits.add_folder(scene, "Carpeta") == "Carpeta 2"
    built_core.tag_edits.move_to_folder(scene, ["A", "B", DEFAULT_LAYER], name)
    assert scene.layer("A").folder == name and scene.layer(DEFAULT_LAYER).folder is None
    assert built_core.tag_edits.rename_folder(scene, name, "Arq")
    assert scene.layer("B").folder == name           # layers name its key
    assert built_core.tag_edits.folder_label(scene, name) == "Arq"
    assert built_core.tag_edits.rename_folder(scene, "Carpeta 2", "Arq")  # labels may repeat
    assert not built_core.tag_edits.rename_folder(scene, name, "")
    built_core.tag_edits.remove_folder(scene, name)
    assert scene.layer("A").folder is None and name not in L.folders_of(scene)
    assert name not in scene.layer_folder_names


def test_hide_in_every_scene_and_back():
    scene = Scene()
    scene.layers.append(Layer("Nueva"))
    scene.saved_views += [SavedView("Planta"), SavedView("Alzado",
                                                          hidden_layers=["X"])]
    assert built_core.tag_edits.hide_in_scenes(scene, ["Nueva"], True) == 2
    assert scene.saved_views[1].hidden_layers == ["X", "Nueva"]
    assert built_core.tag_edits.hide_in_scenes(scene, ["Nueva"], True) == 0
    assert built_core.tag_edits.hide_in_scenes(scene, ["Nueva"], False) == 2


def test_template_round_trip_adds_and_updates_without_removing():
    src = Scene()
    src.layers += [Layer("Muros", color=(0.5, 0.1, 0.1), folder="Arq",
                         linetype="dashed"), Layer("Ejes", linetype="dashdot")]
    data = json.loads(json.dumps(built_core.tag_edits.template_of(src)))
    dst = Scene()
    dst.layers += [Layer("Muros", visible=False), Layer("Propia")]
    added, updated = built_core.tag_edits.apply_template(dst, data)
    assert (added, updated) == (1, 2)               # Ejes; Layer 0 + Muros
    assert [ly.name for ly in dst.layers] == [DEFAULT_LAYER, "Muros",
                                               "Propia", "Ejes"]
    muros = dst.layer("Muros")
    assert not muros.visible                        # keeps its visibility
    assert (muros.folder, muros.linetype) == ("Arq", "dashed")
    with pytest.raises(ValueError):
        built_core.tag_edits.apply_template(dst, {"layers": []})


def test_a_section_plane_on_a_hidden_layer_cuts_nothing():
    from built_core.section import SectionPlane
    scene = Scene()
    scene.layers.append(Layer("Cortes"))
    sp = SectionPlane(V(0, 0, 1), V(0, 0, 1), active=True)
    scene.section_planes.append(sp)
    assign_layer(sp, "Cortes")
    assert scene.active_section() is sp
    scene.layer("Cortes").visible = False
    assert scene.active_section() is None
    assert not scene.entity_visible(sp)
    back = SectionPlane.from_dict(json.loads(json.dumps(sp.to_dict())))
    assert back.layer == "Cortes"


def test_select_content_takes_the_context_and_skips_locked():
    from built_core import on_layers
    scene = Scene()
    scene.layers += [Layer("A"), Layer("B", locked=True)]
    f1 = _square(scene.mesh)
    f2 = _square(scene.mesh, x0=3)
    assign_layer(f1, "A")
    assign_layer(f2, "B")
    g = Group(Mesh(), name="g")
    g.layer = "A"
    scene.groups.append(g)
    assert set(on_layers(scene, ["A"])) == {f1, g}
    assert on_layers(scene, ["B"]) == []


def test_effective_attrs_by_layer_and_layer_look():
    from built_core.materials import effective_attrs, material_sig
    scene = Scene()
    scene.layers += [Layer("A", color=(1, 0, 0)),
                     Layer("B", color=(0, 0, 1), opacity=0.5),
                     Layer("C", locked=True)]
    g = Group(Mesh(), name="g")
    g.layer = "A"
    paint = built_core.tag_styles.layer_paint(scene, g)
    textured = {"texture": {"path": "x.png"}, "color": (0, 1, 0)}
    got = effective_attrs(textured, paint)
    assert got["color"] == (1, 0, 0) and "texture" not in got
    assert effective_attrs({"layer": "B"}, paint)["color"] == (0, 0, 1)
    look = built_core.tag_styles.layer_look(scene, g, paint)
    assert effective_attrs({"layer": "B"}, look)["opacity"] == 0.5
    # Locked, over the face's own material (Colour by layer off).
    plain_look = built_core.tag_styles.layer_look(scene, g, None)
    faded = effective_attrs({"layer": "C", "color": (0.0, 0.0, 0.0)},
                            plain_look)
    assert faded["color"] == pytest.approx((L.LOCKED_FADE,) * 3)
    # Distinct signatures for distinct looks: the chunk caches key on them.
    sigs = {material_sig(paint), material_sig(look), material_sig(None),
            material_sig(plain_look), material_sig({"color": (1, 0, 0)})}
    assert len(sigs) == 5


def test_no_look_means_no_change():
    scene = Scene()
    scene.layers.append(Layer("A"))
    assert built_core.tag_styles.layer_fx(scene) == {}
    assert built_core.tag_styles.layer_look(scene, None, None) is None


def test_viewport_paint_follows_the_style_and_the_layers():
    from types import SimpleNamespace
    from built_core.style import Style
    from views.document_interaction import _paint_of
    scene = Scene()
    scene.layers += [Layer("A", color=(1, 0, 0))]
    g = Group(Mesh(), name="g")
    g.layer = "A"
    vp = SimpleNamespace(scene=scene, plano_style=None, style_override=None)
    scene.display_style = Style()
    assert _paint_of(vp, g) is None                 # nothing on: as before
    scene.display_style.color_by_layer = True
    scene.version += 1
    assert _paint_of(vp, g)["color"] == (1, 0, 0)
    scene.display_style = Style(face_mode="hidden_line", color_by_layer=True)
    scene.version += 1
    assert _paint_of(vp, g) is None                 # plan styles stay flat
    scene.layer("A").locked = True
    scene.version += 1
    assert _paint_of(vp, g)["fx_self"] == (1.0, L.LOCKED_FADE)
    vp.plano_style = "tecnico"
    assert _paint_of(vp, g) is None                 # a sheet's own look


def test_the_group_bake_takes_the_layer_colour_and_comes_back():
    """The baked vertex colours follow Colour by layer, locked and opacity
    — and go back to the material when it is switched off (the chunk cache
    keys on the paint)."""
    import numpy as np
    from built_core.style import Style
    from tests.test_group_material import _stub, _vcol_colours
    scene = Scene()
    scene.display_style = Style()
    scene.layers.append(Layer("Rojo", color=(1.0, 0.0, 0.0)))
    g = Group(Mesh(), name="caja")
    f = _square(g.mesh)
    f.attrs["color"] = [0.0, 0.0, 1.0]
    g.layer = "Rojo"
    scene.groups.append(g)
    vp = _stub(scene)

    def shaded(rgb):
        return tuple(round(float(c), 3)
                     for c in vp._shaded_color(rgb, V(0, 0, 1)))

    assert _vcol_colours(vp._group_chunk(g)) == {shaded((0.0, 0.0, 1.0))}
    scene.display_style.color_by_layer = True
    scene.version += 1
    assert _vcol_colours(vp._group_chunk(g)) == {shaded((1.0, 0.0, 0.0))}
    scene.display_style.color_by_layer = False
    scene.layer("Rojo").locked = True
    scene.version += 1
    fade = tuple(c + (1 - c) * L.LOCKED_FADE for c in (0.0, 0.0, 1.0))
    assert _vcol_colours(vp._group_chunk(g)) == {shaded(fade)}
    scene.layer("Rojo").locked = False
    scene.layer("Rojo").opacity = 0.5
    scene.version += 1
    entry = vp._group_chunk(g)
    assert not len(np.frombuffer(bytes(entry["vcol"]), dtype=np.float32))
    assert entry.get("tcol")                         # drawn see-through


def test_style_remembers_colour_by_layer_only_when_on():
    from built_core.style import Style
    assert "color_by_layer" not in Style().to_dict()
    s = Style(color_by_layer=True)
    assert Style.from_dict(s.to_dict()).color_by_layer


def _two_boxes_scene():
    scene = Scene()
    scene.layers += [Layer("A", linetype="dashed", lineweight=0.5),
                     Layer("B")]
    ga = Group(Mesh(), name="a")
    _square(ga.mesh)
    ga.layer = "A"
    gb = Group(Mesh(), name="b")
    _square(gb.mesh, x0=3)
    gb.layer = "B"
    scene.groups += [ga, gb]
    return scene


def _top_camera(scene=None):
    from built_core.camera import OrbitCamera
    from built_core.composition import MarcoVista, apply_frame_camera
    cam = OrbitCamera()
    f = MarcoVista(view_key="std:top", scale_n=100.0, w_mm=100.0, h_mm=100.0)
    apply_frame_camera(cam, f, saved_view=None, scene=scene or Scene())
    return cam


def test_hlr_tags_every_line_with_its_layer():
    from built_core.hlr import KIND_CUT, hlr_drawing
    scene = _two_boxes_scene()
    d = hlr_drawing(scene, _top_camera(), profiles=False, layers=True)
    assert d.layers is not None and len(d.layers) == len(d.segs) > 0
    xs = {tag: [] for tag in ("A", "B")}
    for seg, tag in zip(d.segs, d.layers):
        xs[tag].append(abs(seg[0]) + abs(seg[2]))
    assert xs["A"] and xs["B"]
    assert all(k != KIND_CUT for k in d.kinds)
    plain = hlr_drawing(scene, _top_camera(), profiles=False)
    assert plain.layers is None and len(plain.segs) == len(d.segs)


def test_hlr_tags_survive_a_section_cut():
    from built_core.hlr import KIND_CUT, hlr_drawing
    from built_core.section import SectionPlane
    scene = _two_boxes_scene()
    for g in scene.groups:
        _square(g.mesh, x0=g.mesh.faces[0].loop[0].position.x(), z=1.0)
    scene.section_planes.append(SectionPlane(V(0, 0, 0.5), V(0, 0, 1),
                                             active=True))
    d = hlr_drawing(scene, _top_camera(), profiles=False, layers=True)
    assert len(d.layers) == len(d.segs)
    for k, tag in zip(d.kinds, d.layers):
        assert (tag is None) == (k == KIND_CUT)


def test_line_styles_and_wanted():
    scene = _two_boxes_scene()
    assert built_core.tag_styles.layer_lines_wanted(scene)
    st = built_core.tag_styles.line_styles(scene, colored=False)
    assert st["A"] == (None, "dashed", 0.5, None, (3.0, 1.5), None)
    assert st["B"][0] is None
    assert built_core.tag_styles.line_styles(scene, colored=True)[DEFAULT_LAYER][0] is None
    plain = Scene()
    plain.layers.append(Layer("X"))
    assert not built_core.tag_styles.layer_lines_wanted(plain)


def test_legend_snapshots_colour_and_line():
    scene = _two_boxes_scene()
    scene.layer("B").visible = False
    rows, swatches = L.legend_rows(scene)
    assert rows == [DEFAULT_LAYER, "A"]
    assert swatches["A"]["linetype"] == "dashed"


def test_nested_folders_move_rename_remove():
    scene = Scene()
    a = built_core.tag_edits.add_folder(scene, "A")
    b = built_core.tag_edits.add_folder(scene, "B", parent=a)
    c = built_core.tag_edits.add_folder(scene, "C", parent=b)
    scene.layers.append(Layer("x", folder=b))
    assert L.subfolders(scene, a, deep=True) == [b, c]
    assert not built_core.tag_edits.move_folder(scene, a, c)             # no loops
    assert not built_core.tag_edits.move_folder(scene, a, a)
    assert built_core.tag_edits.rename_folder(scene, b, "Bee")
    assert L.folder_parent(scene, c) == b and built_core.tag_edits.folder_label(scene, b) == "Bee"
    assert L.layers_in_folder(scene, a) == ["x"]
    built_core.tag_edits.remove_folder(scene, b)                         # contents move up
    assert scene.layer("x").folder == a
    assert L.folder_parent(scene, c) == a
    assert built_core.tag_edits.move_folder(scene, c, None) and L.folder_parent(scene, c) is None


def test_nested_folders_travel_in_the_file_and_the_template(tmp_path):
    from formats import igz
    scene = Scene()
    a = built_core.tag_edits.add_folder(scene, "A")
    b = built_core.tag_edits.add_folder(scene, "B", parent=a)
    _square(scene.mesh)
    path = tmp_path / "f.igz"
    igz.save_scene(scene, path)
    back = Scene()
    igz.load_into(back, path)
    assert L.folder_parent(back, b) == a
    other = Scene()
    built_core.tag_edits.apply_template(other, json.loads(json.dumps(built_core.tag_edits.template_of(scene))))
    assert L.folder_parent(other, b) == a


@pytest.mark.parametrize("seed", range(10))
def test_random_documents_round_trip_their_layers(seed, tmp_path):
    """Synthetic documents with random looks, folders and an active layer
    come back from the file exactly as they went in."""
    from formats import igz
    rng = random.Random(seed)
    scene = Scene()
    folders = [f"F{i}" for i in range(rng.randint(0, 3))]
    scene.layer_folders = list(folders)
    for i in range(rng.randint(1, 6)):
        scene.layers.append(Layer(
            f"L{i}", visible=rng.random() < 0.8, locked=rng.random() < 0.2,
            color=(rng.random(), rng.random(), rng.random())
            if rng.random() < 0.5 else None,
            folder=rng.choice(folders) if folders and rng.random() < 0.6
            else None,
            linetype=rng.choice(list(L.LINETYPES)),
            lineweight=rng.choice([None, 0.18, 0.35, 0.7]),
            opacity=rng.choice([1.0, 0.75, 0.5])))
    names = [ly.name for ly in scene.layers[1:]]
    scene.active_layer = rng.choice(names)
    _square(scene.mesh)
    path = tmp_path / f"r{seed}.igz"
    igz.save_scene(scene, path)
    back = Scene()
    igz.load_into(back, path)
    assert [ly.to_dict() for ly in back.layers] == \
        [Layer.from_dict(ly.to_dict()).to_dict() for ly in scene.layers]
    assert back.active_layer == scene.active_layer
    assert back.layer_folders == folders


def test_a_layer_not_printed_round_trips_and_leaves_the_legend():
    ly = Layer("Ref", printable=False)
    assert ly.to_dict()["printable"] is False and ly.has_style()
    assert Layer.from_dict(ly.to_dict()).printable is False
    assert "printable" not in Layer("Muros").to_dict()   # the default: kept out
    assert Layer.from_dict({"name": "X"}).printable is True
    scene = Scene()
    scene.layers += [Layer("Muros"), Layer("Ref", printable=False)]
    rows, _sw = L.legend_rows(scene)
    assert "Muros" in rows and "Ref" not in rows
    assert [ly.name for ly in L.unprinted(scene)] == ["Ref"]
    built_core.tag_styles.hide_unprinted(scene)
    assert not scene.layer("Ref").visible and scene.layer("Muros").visible


def test_a_layer_not_printed_survives_save_and_open(tmp_path):
    from formats import igz
    scene = Scene()
    scene.layers += [Layer("Ref", printable=False)]
    path = tmp_path / "noprint.igz"
    igz.save_scene(scene, path)
    back = Scene()
    igz.load_into(back, path)
    assert back.layer("Ref") is not None
    assert back.layer("Ref").printable is False


def test_the_view_dxf_writes_each_layer_with_its_colour(native_host, tmp_path,
                                                        monkeypatch):
    import re
    win = native_host.window
    vp = win.viewport
    native_host.resize(800, 600, 1.)
    vp.camera.set_aspect(800, 600)
    src = _two_boxes_scene()
    vp.scene.layers[:] = src.layers
    vp.scene.groups[:] = _moved(src)
    vp.scene.layer("B").color = (1.0, 0.0, 0.0)
    vp.notify_scene_changed()
    lo, hi = vp.scene.bounds()
    vp.camera.fit_to(lo, hi, 1.2)
    vp.camera.perspective = False
    path = tmp_path / "vista.dxf"
    one_off_export(win, "dxf", path)
    text = path.read_text(encoding="ascii")
    assert re.search(r"0\nLAYER\n2\nB(-PERFIL)?\n70\n0\n62\n1\n",
                     text)                               # red
    assert re.search(r"\n2\nA(-PERFIL)?\n", text)


def test_every_line_type_has_a_name_a_dxf_twin_and_a_pattern(tmp_path):
    from formats.dxf_out import _LTYPES, save_dxf_layers
    from tests.test_project_drawing_filters import Project
    import numpy as np
    project = Project()
    project.action("lens", "1")
    labels = project.view.renderer.calls["ui_set_style_catalog"][1]
    assert set(labels) == set(L.LINETYPES)               # each one offered
    assert len(L.LINETYPES) >= 10
    for key, pattern in L.LINETYPES.items():
        dxf = L.DXF_LINETYPES[key]
        assert key == "continuous" or dxf in _LTYPES      # defined in the file
        assert len(pattern) % 2 == 0                      # dash, gap pairs
        assert Layer("X", linetype=key).linetype == key   # kept, not reset
    # Written out, each one used is defined in the file's LTYPE table.
    seg = np.array([[0.0, 0.0, 1.0, 0.0]])
    groups = [(k, seg, L.DXF_LINETYPES[k], None) for k in L.LINETYPES]
    path = tmp_path / "tipos.dxf"
    save_dxf_layers(path, groups)
    text = path.read_text(encoding="ascii")
    for key in L.LINETYPES:
        if key != "continuous":
            assert f"0\nLTYPE\n2\n{L.DXF_LINETYPES[key]}\n" in text


def test_recount_after_host_drawing_preserves_other_mesh_counts(monkeypatch):
    import built_core
    from built_core.purge import layer_count_steps
    from built_core.history import AddEdgeCommand
    scene = Scene(groups=[Group(Mesh())])
    _square(scene.groups[0].mesh)
    history = History(scene)
    def counted():
        return list(layer_count_steps(scene, history=history))[-1]
    before = counted()
    history.execute(AddEdgeCommand(V(0, 0), V(1, 0)))
    monkeypatch.setattr(built_core, "purge_iter_groups", lambda *_: pytest.fail("placement walk"))
    assert counted()[DEFAULT_LAYER] == before[DEFAULT_LAYER] + 1
    history.undo()
    assert counted() == before
    history.redo()
    assert counted()[DEFAULT_LAYER] == before[DEFAULT_LAYER] + 1


def _moved(src):
    """``src``'s top level, taken out of it to go into another scene (a
    placement has one parent)."""
    groups = list(src.groups)
    src.groups.clear()
    return groups
