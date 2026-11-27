# SPDX-License-Identifier: GPL-3.0-or-later
"""Fillet and Chamfer after a size that does not fit (owner report,
2026-11-27): an I profile, its four inner edges selected, the remembered
100 mm too large. Every later value recomputes from the same selection;
the refusal says what fits; a size that does not fit is not proposed; the
round strips are exact cylinders and the flat faces beside them shade flat.
"""
from __future__ import annotations

import json
import math

import numpy as np
import pytest
from built_core import Vec3 as V

from built_core.history import History
from built_core.orient import is_closed
from built_core.protocol import commands
from built_core.scene import Scene
from built_core.command_protocol import to_json_key
from tools.fillet import ChamferTool, FilletTool

# Flanges 100 x 10, web 10, 100 high, 300 long (metres).
B, H, TF, TW, L = 0.1, 0.1, 0.01, 0.01, 0.3


def outline():
    b, w = B / 2, TW / 2
    return [(-b, 0), (b, 0), (b, TF), (w, TF), (w, H - TF), (b, H - TF), (b, H),
            (-b, H), (-b, H - TF), (-w, H - TF), (-w, TF), (-b, TF)]


INNER = (3, 4, 9, 10)   # the web/flange corners of the outline


def i_profile(mesh):
    pts = outline()
    bottom = [V(x, y, 0) for x, y in pts]
    top = [V(x, y, L) for x, y in pts]
    mesh.add_face(list(reversed(bottom)))
    mesh.add_face(top)
    for i in range(len(pts)):
        j = (i + 1) % len(pts)
        mesh.add_face([bottom[i], bottom[j], top[j], top[i]])


def inner_edges(mesh):
    pts = outline()
    out = []
    for k in INNER:
        x, y = pts[k]
        out.append(mesh.find_edge(mesh.vertex_at(V(x, y, 0)), mesh.vertex_at(V(x, y, L))))
    assert all(out)
    return out


@pytest.fixture
def viewport():
    from tests.controller_surface import ControllerSurface as Viewport
    vp = Viewport(None)
    vp.resize(1000, 700)
    vp.camera.set_aspect(1000, 700)
    vp.said = []
    vp.flash_status = lambda text, *a, **k: vp.said.append(str(text))
    vp.scene.mesh.clear()
    i_profile(vp.scene.mesh)
    vp.scene.version += 1
    yield vp
    FilletTool.radius = 0.1
    ChamferTool.radius = 0.1


def activate(vp, cls, remembered=0.1):
    cls.radius = remembered
    vp.scene.select(inner_edges(vp.scene.mesh))
    tool = cls()
    vp.set_active_tool(tool)
    assert len(tool.edges) == 4
    return tool


@pytest.mark.parametrize("cls", [FilletTool, ChamferTool])
def test_typed_sizes_after_a_refused_one_round_the_same_selection(viewport, cls):
    """The exact report: 100 mm refused, then 5 typed, then (undone) 2."""
    vp = viewport
    tool = activate(vp, cls)
    tool._replan()
    faces = len(vp.scene.mesh.faces)
    tool.on_value(vp, 0.1)                           # Enter on the refused size
    assert len(vp.scene.mesh.faces) == faces
    assert any("too large" in s for s in vp.said)
    assert tool.on_value(vp, 0.005)
    assert len(vp.scene.mesh.faces) > faces and is_closed(vp.scene.mesh)
    assert cls.radius == pytest.approx(0.005)       # remembered for the session
    vp.history.undo()
    tool = activate(vp, cls, remembered=0.1)
    assert tool.on_value(vp, 0.002)
    assert len(vp.scene.mesh.faces) > faces and is_closed(vp.scene.mesh)


@pytest.mark.parametrize("cls", [FilletTool, ChamferTool])
def test_the_form_recomputes_after_each_size(viewport, cls):
    """Too large, valid, too large, valid in the form's Size field: each
    size replans from the selection; Apply rounds the last valid one."""
    vp = viewport
    tool = activate(vp, cls)
    faces = len(vp.scene.mesh.faces)
    for size, fits in [(0.1, False), (0.005, True), (0.2, False), (0.002, True)]:
        vp.said.clear()
        tool.set_size(vp, size)
        assert (tool._plan is not None) == fits, size
        assert bool(tool.preview_faces()) == fits
        assert tool.value_error == (not fits)
        if not fits:
            assert vp.said and "too large" in vp.said[-1] and "fits" in vp.said[-1]
        assert len(tool.edges) == 4
    tool.apply_from_panel(vp)
    assert len(vp.scene.mesh.faces) > faces and is_closed(vp.scene.mesh)
    assert cls.radius == pytest.approx(0.002)


def test_the_refusal_says_the_largest_size_that_fits(viewport):
    vp = viewport
    tool = activate(vp, FilletTool, remembered=0.005)   # fits: no proposal
    assert tool._plan is not None and FilletTool.radius == 0.005
    tool.set_size(vp, 0.1)
    message = tool.value_label()[0]
    assert message.startswith("Radius too large: ") and message.endswith(" fits."), message
    said, proposed = tool._fit
    assert tool.value_error
    # The web (80 mm) takes two fillets: 40 mm would close it. What is said
    # fits; the next millimetre does not.
    assert said == pytest.approx(0.039)
    tool.set_size(vp, said)
    assert tool._plan is not None
    tool.set_size(vp, said + 0.001)
    assert tool._plan is None
    assert proposed <= said


def test_a_remembered_size_that_does_not_fit_is_not_proposed(viewport):
    vp = viewport
    tool = activate(vp, FilletTool, remembered=0.1)
    # The web (80 mm between the flanges, two fillets) bounds it.
    assert tool._plan is not None, tool._message
    proposed = tool.radius
    assert proposed == pytest.approx(0.036)         # 39.99 mm fits: about 10 % under
    assert proposed * 1000 == pytest.approx(round(proposed * 1000))     # a clean value
    assert any("does not fit this selection" in s for s in vp.said)
    assert FilletTool.radius == 0.1                  # the remembered value stays
    tool.apply_from_panel(vp)
    assert is_closed(vp.scene.mesh)
    assert FilletTool.radius == 0.1
    # Typed or sized by the user: that is remembered, the proposal goes.
    vp.history.undo()
    tool = activate(vp, FilletTool, remembered=0.1)
    assert tool.radius != 0.1
    tool.set_size(vp, 0.003)
    assert FilletTool.radius == 0.003 and "radius" not in vars(tool)


def test_a_refusal_not_about_the_size_proposes_nothing(viewport):
    vp = viewport
    tool = activate(vp, FilletTool, remembered=0.005)
    tool.set_size(vp, 0.005)
    FilletTool.shape = ((0.0, 0.2, False), (0.5, 0.5, False))   # does not reach the far face
    try:
        tool._replan()
        assert tool._plan is None and "fits" not in tool._message
    finally:
        FilletTool.shape = None


# ---- exact result and shading ------------------------------------------------

def run(h, name, **kw):
    result = commands.run(h, name, **kw)
    assert result["ok"], (name, result.get("error"))
    return result


def rounded(radius=0.005, tool="fillet"):
    scene = Scene()
    h = History(scene)
    run(h, "add_face", outer=[[x, y, 0] for x, y in outline()])
    run(h, "push_pull", face=scene.mesh.faces[0].key, distance=L)
    z = sorted({round(v.position.z(), 9) for v in scene.mesh.vertices})
    edges = []
    for k in INNER:
        x, y = outline()[k]
        m = scene.mesh
        edges.append(m.find_edge(m.vertex_at(V(x, y, z[0])), m.vertex_at(V(x, y, z[-1]))))
    kw = dict(edges=[to_json_key(e.key) for e in edges], radius=radius, segments=8)
    if tool == "chamfer":
        kw.update(profile=0.25, measure="offset")     # the Chamfer tool's preset
    run(h, "bevel_selection", **kw)
    rounded.history = h
    return scene


def test_fillet_strips_are_exact_cylinders_with_circle_sections():
    scene = rounded()
    st = scene.mesh._store
    faces = {f.key: json.loads(st.exact_face(f.key)) for f in scene.mesh.faces if st.exact_face(f.key)}
    assert len(faces) == 4 * 8
    assert {next(iter(s["surface"])) for s in faces.values()} == {"cylinder"}
    assert len({json.dumps(s, sort_keys=True) for s in faces.values()}) == 4     # one per fillet
    edges = [json.loads(st.exact_edge(e.key)) for e in scene.mesh.edges if st.exact_edge(e.key)]
    assert {next(iter(e["curve"])) for e in edges} == {"circle"}
    assert len(edges) == 4 * 2 * 8                   # both end sections of each fillet
    assert st.exact_report[3] == 0
    # Undo drops them with the fillet; redo brings them back.
    h, counts = rounded.history, st.exact_counts()
    assert h.undo() and st.exact_counts() == (0, 0)
    assert h.redo() and st.exact_counts() == counts


def test_a_filleted_profile_exports_real_fillets_to_step(tmp_path):
    from formats import step
    scene = rounded()
    out = tmp_path / "profile.step"
    step.save_step(scene, out)
    # One exact patch per strip on its fillet's cylinder, as for a pushed
    # circle (merging the patches of one support is the writer's own step).
    assert out.read_text().count("= CYLINDRICAL_SURFACE(") == 4 * 8
    assert step.prepare_step(out)["exact"] == 1


def corner_normals(scene):
    """Per face: (unbound?, plane normal, the shading normals of its corners)."""
    st = scene.mesh._store
    d = st.display_arrays(True)
    pos = np.asarray(d["pos"])
    corners = np.asarray(d["corners"])
    sizes = np.asarray(d["ring_sizes"])
    counts = np.asarray(d["ring_counts"])
    normals = np.asarray(d["corner_normals"])
    bound = [bool(st.exact_face(f.key)) for f in scene.mesh.faces]
    assert len(bound) == len(counts)
    out, r, k = [], 0, 0
    for f, count in enumerate(counts):
        first = None
        rows = []
        for _ in range(count):
            ring = corners[k:k + sizes[r]]
            if first is None:
                p = pos[ring]
                n = np.cross(p - p[0], np.roll(p, -1, axis=0) - p[0]).sum(axis=0)
                first = n / np.linalg.norm(n)
            rows.extend(normals[k:k + sizes[r]])
            k += sizes[r]
            r += 1
        out.append((not bound[f], first, np.asarray(rows)))
    return out


def seams(scene):
    st = scene.mesh._store
    out = []
    for e in scene.mesh.edges:
        fs = list(e.faces)
        # Strip beside a flat face, along the fillet (not a section arc on a cap).
        if len(fs) == 2 and e.soft and sum(bool(st.exact_face(f.key)) for f in fs) == 1:
            out.append(e)
    return out


def check_shading(faces, flattened_at_least):
    flattened = 0
    for planar, normal, rows in faces:
        given = rows[np.isfinite(rows[:, 0])]
        if planar:
            # The web and flange faces: their own plane wherever a normal is given.
            assert all(np.dot(n, normal) > 1 - 1e-9 for n in given)
            flattened += len(given)
        else:
            assert len(given) and all(abs(np.linalg.norm(n) - 1) < 1e-9 for n in given)
    assert flattened >= flattened_at_least
    # Strip corners on the tangent lines take the plane's normal (axis
    # directions here): strip and plane meet without a crease.
    axis = [n for planar, _, rows in faces if not planar for n in rows
            if np.isfinite(n[0]) and max(abs(n)) > 1 - 1e-9]
    assert len(axis) >= 4 * 2 * 2


def test_flat_faces_beside_a_fillet_shade_flat():
    scene = rounded()
    # The tangent lines are soft (no line) with smooth normals off: no
    # soft-edge average reaches the web or the flanges, in any bake.
    tangent = seams(scene)
    assert len(tangent) == 4 * 2
    assert all(e.soft and e.flat for e in tangent)
    st = scene.mesh._store
    for e in scene.mesh.edges:
        if e.soft and not e.flat:
            assert all(st.exact_face(f.key) for f in e.faces), "a smooth edge beside a flat face"
    check_shading(corner_normals(scene), 0)


def test_a_flat_face_keeps_its_normal_beside_a_smooth_seam():
    """A fillet whose tangent lines were smoothed (an older file, or the
    user's Smooth): the exact display still keeps the planes flat."""
    scene = rounded()
    for e in seams(scene):
        e.flat = False
    assert all(e.soft and not e.flat for e in seams(scene))
    check_shading(corner_normals(scene), 4 * 2 * 2)


def test_chamfer_is_flat():
    """Coplanar segments may be soft among themselves; the seams with the
    web and flanges (45°) stay hard, so no flat face is smoothed."""
    scene = rounded(tool="chamfer")
    st = scene.mesh._store
    assert not any(st.exact_face(f.key) for f in scene.mesh.faces)
    assert is_closed(scene.mesh)
    assert len(scene.mesh.faces) == 14 + 4 * 8
    for e in scene.mesh.edges:
        faces = list(e.faces)
        if len(faces) != 2 or not e.soft:
            continue
        a, b = (np.asarray(f.normal().toTuple()) for f in faces)
        assert np.dot(a, b) > 1 - 1e-9, "a soft chamfer seam between faces that are not coplanar"
    # And the seams with the web and flanges exist and are hard.
    creases = [e for e in scene.mesh.edges if len(list(e.faces)) == 2 and not e.soft
               and abs(np.dot(*(np.asarray(f.normal().toTuple()) for f in e.faces))) < 0.8]
    assert len(creases) >= 4 * 2
