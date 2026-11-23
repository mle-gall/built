# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""What a vector sheet frame draws (ingetrazo_core sheets::geometry, from
the document and the mesh stores) is what the composer collected by
applying the frame's saved view to the live scene and walking every face
and edge in Python (``core.hlr.collect_geometry``): loose faces and edges
by their tags and Hide, placements through their world matrices (turned,
mirrored, nested), hidden subtrees and forced tags, billboards left out,
tags that do not print left out, each line inked with its own tag or its
object's. The live scene is never touched."""
from __future__ import annotations

import math
import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import numpy as np  # noqa: E402
import pytest  # noqa: E402
from PySide6.QtWidgets import QApplication  # noqa: E402

import ingetrazo_core as NATIVE  # noqa: E402

_app = QApplication.instance() or QApplication([])


def _box(x, y=0.0, z=0.0, *, layer=None, soft=False, hide_top=False):
    from ingetrazo_core import Vec3 as V
    from core.mesh import Mesh
    m = Mesh()
    p = [V(x + a, y + b, z + c) for c in (0, 1) for b in (0, 1) for a in (0, 1)]
    for loop in ((0, 2, 3, 1), (4, 5, 7, 6), (0, 1, 5, 4), (2, 6, 7, 3), (0, 4, 6, 2), (1, 3, 7, 5)):
        f = m.add_face([p[i] for i in loop])
        if layer:
            f.attrs["layer"] = layer
    if hide_top:
        m.faces[1].attrs["hidden"] = True
    if soft:
        for e in m.edges:
            if abs(e.v0.position.z() - e.v1.position.z()) > 0.5:
                e.soft = True
    m.add_edge(V(x, y - 1, 0), V(x + 1, y - 2, 0))           # a loose line
    return m


def _xf(tx, ty, turn=0.0, mirror=False):
    from ingetrazo_core import Matrix4
    m = Matrix4()
    m.translate(tx, ty, 0.0)
    m.rotate(turn, 0.0, 0.0, 1.0)
    if mirror:
        m.scale(-1.0, 1.0, 1.0)
    return m


def _scene():
    from ingetrazo_core import Vec3 as V
    from core.group import Group
    from core.layers import Layer
    from core.saved_views import SavedView
    from core.scene import Scene
    s = Scene()
    for name, folder in (("Muros", "Arq"), ("Losa", "Arq"), ("Mobiliario", "Int"), ("Ejes", None),
                         ("Notas", None), ("Bloq", None)):
        s.layers.append(Layer(name, folder=folder, printable=name != "Notas", locked=name == "Bloq"))
    s.layer_folders = ["Arq", "Int"]
    loose = s.mesh
    for loop in (((0, -5, 0), (2, -5, 0), (2, -3, 0), (0, -3, 0)),
                 ((3, -5, 0), (5, -5, 0), (5, -3, 0), (3, -3, 0)),
                 ((6, -5, 0), (8, -5, 0), (8, -3, 0), (6, -3, 0))):
        loose.add_face([V(*p) for p in loop])
    loose.faces[1].attrs["layer"] = "Ejes"
    loose.faces[2].attrs["hidden"] = True
    e = loose.add_edge(V(0, -7, 0), V(4, -7, 0))
    e.layer = "Notas"
    loose.add_edge(V(0, -8, 0), V(4, -8, 1))

    a = Group(_box(0, soft=True), "a")
    a.layer = "Muros"
    b = Group(_box(0, layer="Losa", hide_top=True), "b")
    b.xform = _xf(5, 1, 30.0)
    b.layer = "Losa"
    c = Group(_box(0), "c")
    c.xform = _xf(10, 0, 0.0, mirror=True)
    c.layer = "Mobiliario"
    nest = Group(_box(20), "nest")
    nest.layer = "Bloq"
    inner = Group(_box(0), "inner")
    inner.xform = _xf(3, 0, 90.0)
    inner.layer = "Ejes"
    deep = Group(_box(0, 3), "deep")
    deep.xform = _xf(0, 2)
    gone = Group(_box(0, 6), "gone")
    gone.hidden = True
    inner.children.extend([deep, gone])
    nest.children.append(inner)
    sign = Group(_box(40), "sign")
    sign.billboard = True
    notes = Group(_box(50), "notes")
    notes.layer = "Notas"
    s.groups.extend([a, b, c, nest, sign, notes])
    s.saved_views.append(SavedView("all"))
    s.saved_views.append(SavedView("no walls", hidden_layers=["Muros"]))
    s.saved_views.append(SavedView("no axes", hidden_layers=["Ejes"]))
    s.saved_views.append(SavedView("folders", hidden_folders=["Arq"]))
    s.saved_views.append(SavedView("objects", hidden_objects=[a.uid, deep.uid]))
    return s


def _canon(tris, hard, soft, soft_n, tags=None):
    r = lambda p: tuple(round(float(v), 6) + 0.0 for v in p)  # noqa: E731
    t = sorted(tuple(sorted(r(p) for p in tri)) for tri in np.asarray(tris).reshape(-1, 3, 3))
    h = sorted(tuple(sorted((r(a), r(b)))) for a, b in np.asarray(hard).reshape(-1, 2, 3))
    s = sorted((tuple(sorted((r(a), r(b)))), tuple(sorted(r(n) if not math.isnan(n[0]) else ("nan",)
                                                            for n in ns)))
               for (a, b), ns in zip(np.asarray(soft).reshape(-1, 2, 3), np.asarray(soft_n).reshape(-1, 2, 3)))
    return t, h, s


def _reference(name):
    """The composer's way, on a second identical scene."""
    from core.hlr import collect_geometry
    from core.layers import hide_unprinted
    scene = _scene()
    view = next((v for v in scene.saved_views if v.name == name), None)
    if view is not None:
        view.apply_state(scene)
    hide_unprinted(scene)
    tris, hard, soft, hard_l, soft_l, tri_l = collect_geometry(scene, layers=True)
    nan3 = (float("nan"),) * 3
    soft_p = [(p0, p1) for p0, p1, _a, _b in soft]
    soft_n = [(na, nan3 if nb is None else nb) for _p0, _p1, na, nb in soft]
    return (tris, hard, soft_p, soft_n), (tri_l, hard_l, soft_l)


@pytest.mark.parametrize("name", [None, "all", "no walls", "no axes", "folders", "objects"])
def test_the_frame_draws_what_the_view_shows_without_touching_the_scene(name):
    from core._native import qt_hypot
    scene = _scene()
    view = next((v for v in scene.saved_views if v.name == name), None)
    before = ([ly.visible for ly in scene.layers], dict(scene.folder_visibility),
              [g.hidden for g in scene.groups])
    (tris, hard, soft, soft_n), tags = NATIVE.sheet_frame_geometry(
        scene._root, list(scene.groups), scene.loose_mesh, view, qt_hypot() or "libc++-fma", True)
    assert ([ly.visible for ly in scene.layers], dict(scene.folder_visibility),
            [g.hidden for g in scene.groups]) == before
    want, want_tags = _reference(name)
    got_c, want_c = _canon(tris, hard, soft, soft_n), _canon(*want)
    for got_part, want_part, what in zip(got_c, want_c, ("triangles", "hard edges", "soft edges")):
        assert got_part == want_part, what
    # Each line's tag: as multisets keyed by the line.
    def tagged(rows, labels):
        r = lambda p: tuple(round(float(v), 6) + 0.0 for v in p)  # noqa: E731
        return sorted((tuple(sorted((r(a), r(b)))), t) for (a, b), t in
                      zip(np.asarray(rows).reshape(-1, 2, 3), labels))
    assert tagged(hard, tags[1]) == tagged(want[1], want_tags[1])
    assert tagged(soft, tags[2]) == tagged(want[2], want_tags[2])
    assert sorted(tags[0]) == sorted(want_tags[0])


@pytest.mark.parametrize("view_key,name", [("std:top", None), ("std:front", None), ("scene:Iso", "Iso"),
                                           ("scene:Iso no walls", "Iso no walls")])
@pytest.mark.parametrize("hidden", [False, True])
def test_a_vector_frame_draws_what_the_composer_drew(view_key, name, hidden):
    """``wgpu_sheet_frames.drawing`` against the composer's way: the view
    applied to the scene, the camera pointed by ``apply_frame_camera``,
    ``collect_geometry``'s walk, the same hidden-line pass."""
    from types import SimpleNamespace
    from core.camera import OrbitCamera
    from core.composition import MarcoVista, apply_frame_camera, model_height_for_frame
    from core.hlr import hlr_drawing
    from core.layers import hide_unprinted
    from core.saved_views import SavedView
    from views import wgpu_sheet_frames

    def scene_with_views():
        s = _scene()
        s.saved_views.append(SavedView("Iso", target=(15.0, 2.0, 0.5), distance=40.0, yaw=math.radians(-35),
                                       pitch=math.radians(30), perspective=False))
        s.saved_views.append(SavedView("Iso no walls", target=(15.0, 2.0, 0.5), distance=40.0,
                                       yaw=math.radians(-35), pitch=math.radians(30), perspective=False,
                                       hidden_layers=["Muros", "Ejes"]))
        return s
    kw = dict(x_mm=10, y_mm=10, w_mm=180, h_mm=120, scale_n=200.0, view_key=view_key,
              style="vectorial", hidden_lines=hidden)
    scene = scene_with_views()
    source = SimpleNamespace(scene=scene, camera=OrbitCamera())
    segs, kinds = wgpu_sheet_frames.drawing(source, MarcoVista(**kw))

    ref = scene_with_views()
    frame = MarcoVista(**kw)
    view = next((v for v in ref.saved_views if v.name == name), None)
    cam = OrbitCamera()
    apply_frame_camera(cam, frame, view, ref)
    hide_unprinted(ref)
    d = hlr_drawing(ref, cam, profiles=True, fills=True, hidden=hidden)
    model_h = model_height_for_frame(frame.h_mm, frame.scale_n)
    k = frame.h_mm / model_h
    half_h, half_w = model_h / 2.0, model_h / 2.0 * (frame.w_mm / frame.h_mm)
    want = np.asarray(d.segs, float).reshape(-1, 4).copy()
    want[:, 0::2] = (want[:, 0::2] + half_w) * k
    want[:, 1::2] = (half_h - want[:, 1::2]) * k

    def canon(segs, kinds):
        rows = []
        for (x0, y0, x1, y1), kind in zip(np.asarray(segs).tolist(), np.asarray(kinds).tolist()):
            a, b = sorted([(round(x0, 4) + 0.0, round(y0, 4) + 0.0), (round(x1, 4) + 0.0, round(y1, 4) + 0.0)])
            rows.append((a, b, int(kind)))
        return sorted(rows)
    assert len(segs) > 10
    assert canon(segs, kinds) == canon(want, d.kinds)
