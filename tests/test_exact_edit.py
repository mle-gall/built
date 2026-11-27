# SPDX-License-Identifier: GPL-3.0-or-later
"""Tools keep exactness (docs/exact-geometry.md §3, lane 3).

Each edit row has a keep case (the support survives), a re-trim case (the
support is rederived or split), and a fallback case (the binding drops and
the facets remain). Undo restores bindings exactly.
"""
import json
import math

import pytest

from built_core.history import History
from built_core.protocol import commands
from built_core.scene import Scene

TAU = 2 * math.pi


def ring(r=1.0, n=24, z=0.0, cx=0.0, cy=0.0):
    return [[cx + r * math.cos(TAU * i / n), cy + r * math.sin(TAU * i / n), z] for i in range(n)]


def run(h, name, **kw):
    result = commands.run(h, name, **kw)
    assert result["ok"], (name, result.get("error"))
    return result


def doc():
    scene = Scene()
    return scene, History(scene)


def store(scene):
    return scene.mesh._store


def curves(scene, kind="edge"):
    st = store(scene)
    items = scene.mesh.edges if kind == "edge" else scene.mesh.faces
    read = st.exact_edge if kind == "edge" else st.exact_face
    out = {}
    for x in items:
        raw = read(x.key)
        if raw:
            out[x.key] = json.loads(raw)
    return out


def kinds(rows, field):
    return sorted({next(iter(r[field])) for r in rows.values()})


def bindings(scene):
    st = store(scene)
    return (sorted((st.id_of("edge", k), json.dumps(v, sort_keys=True)) for k, v in curves(scene).items()),
            sorted((st.id_of("face", k), json.dumps(v, sort_keys=True)) for k, v in curves(scene, "face").items()))


def circle(h, r=1.0, n=24, z=0.0, cx=0.0, cy=0.0):
    run(h, "add_face", outer=ring(r, n, z, cx, cy), curve=True, exact=True)


def cylinder(r=1.0, n=24, height=2.0):
    scene, h = doc()
    circle(h, r, n)
    run(h, "push_pull", face=scene.mesh.faces[0].key, distance=height)
    return scene, h


def cap(scene, z):
    for f in scene.mesh.faces:
        if all(abs(v.z() - z) < 1e-9 for v in f.vertices) and len(f.vertices) > 4:
            return f
    raise AssertionError("no cap")


# ---- drawing ---------------------------------------------------------------

def test_circle_edges_are_one_exact_circle_and_polygon_stays_straight():
    scene, h = doc()
    circle(h, r=2, n=12, cx=5)
    rows = curves(scene)
    assert len(rows) == 12 and kinds(rows, "curve") == ["circle"]
    (c,) = {json.dumps(r["curve"], sort_keys=True) for r in rows.values()}
    c = json.loads(c)["circle"]
    assert c["radius"] == pytest.approx(2) and c["frame"]["origin"] == pytest.approx([5, 0, 0])
    spans = sorted(abs(r["interval"][1] - r["interval"][0]) for r in rows.values())
    assert spans == pytest.approx([TAU / 12] * 12)
    # Polygon: the same points without the exact flag stay straight sides.
    other, h2 = doc()
    run(h2, "add_face", outer=ring(2, 12), curve=True)
    assert store(other).exact_counts() == (0, 0)


def test_arc_is_exact_and_display_follows_the_segments():
    scene, h = doc()
    pts = [[math.cos(a), math.sin(a), 0] for a in [i * math.pi / 8 for i in range(5)]]
    run(h, "add_arc", points=pts)
    rows = curves(scene)
    assert len(rows) == 4 and kinds(rows, "curve") == ["circle"]
    coarse, h2 = doc()
    circle(h2, n=6)
    fine, h3 = doc()
    circle(h3, n=48)
    # Segment count sets the number of editable and displayed edges.
    def drawn(s):
        d = s.mesh._store.display_arrays()
        import numpy as np
        p = np.asarray(d["pos"]).reshape(-1, 3)
        return len(d["edges"]), float(np.abs(np.hypot(p[:, 0], p[:, 1]) - 1).max())
    (n6, err6), (n48, err48) = drawn(coarse), drawn(fine)
    assert err6 < 1e-12 and err48 < 1e-12
    assert (n6, n48) == (6, 48)


def test_resegmenting_an_exact_circle_keeps_the_circle():
    scene, h = doc()
    circle(h, n=12)
    before = {json.dumps(r["curve"], sort_keys=True) for r in curves(scene).values()}
    run(h, "resegment_curve", edge=scene.mesh.edges[0].key, segments=30)
    rows = curves(scene)
    assert len(rows) == 30
    assert {json.dumps(r["curve"], sort_keys=True) for r in rows.values()} == before


# ---- push / pull -------------------------------------------------------------

def test_push_pull_circle_makes_one_cylinder_with_soft_generators():
    scene, h = cylinder()
    faces = curves(scene, "face")
    assert len(faces) == 24 and kinds(faces, "surface") == ["cylinder"]
    assert len({json.dumps(s, sort_keys=True) for s in faces.values()}) == 1
    assert len(curves(scene)) == 48
    import numpy as np
    d = store(scene).display_arrays(True)
    owner = np.arange(len(d["edges"]))
    flags = np.asarray(d["flags"])
    # Every generator (vertical logical edge) draws soft; rims stay hard.
    st = store(scene)
    for i, e in enumerate(scene.mesh.edges):
        a, b = e.v0.position, e.v1.position
        vertical = abs(a.x() - b.x()) < 1e-12 and abs(a.y() - b.y()) < 1e-12
        drawn = flags[owner == i]
        assert len(drawn) and all((drawn & 1) == (1 if vertical else 0)), (i, vertical)
    assert st.exact_report[3] == 0


def test_push_pull_ids_keep_base_and_undo():
    scene, h = doc()
    circle(h)
    st = store(scene)
    face = scene.mesh.faces[0]
    fid = st.id_of("face", face.key)
    before = bindings(scene)
    run(h, "push_pull", face=face.key, distance=1.5, keep_base=True)
    # keep_base: the face left in place keeps the id, walls are exact.
    assert st.keys_of_ids("face", [fid])[0] is not None
    kept = [f for f in scene.mesh.faces if st.id_of("face", f.key) == fid][0]
    assert all(abs(v.z()) < 1e-12 for v in kept.vertices)
    assert kinds(curves(scene, "face"), "surface") == ["cylinder"]
    after = bindings(scene)
    h.undo()
    assert bindings(scene) == before
    h.redo()
    assert bindings(scene) == after


def test_second_push_extends_the_same_cylinder():
    scene, h = cylinder(height=1)
    first = {json.dumps(s, sort_keys=True) for s in curves(scene, "face").values()}
    run(h, "push_pull", face=cap(scene, 1).key, distance=1)
    faces = curves(scene, "face")
    assert {json.dumps(s, sort_keys=True) for s in faces.values()} == first
    # The cap moved: its rim curves moved with it.
    zs = {round(r["curve"]["circle"]["frame"]["origin"][2], 9) for r in curves(scene).values()}
    assert zs == {0, 1, 2} or zs == {0, 2}


def test_push_through_ends_the_removed_face_and_keeps_the_bore_exact():
    scene, h = doc()
    run(h, "add_face", outer=[[-3, -3, 0], [3, -3, 0], [3, 3, 0], [-3, 3, 0]])
    run(h, "push_pull", face=scene.mesh.faces[0].key, distance=2)
    circle(h, z=2)
    st = store(scene)
    disk = [f for f in scene.mesh.faces if len(f.vertices) == 24][0]
    did = st.id_of("face", disk.key)
    run(h, "push_pull", face=disk.key, distance=-2)
    assert st.keys_of_ids("face", [did])[0] is None
    faces = curves(scene, "face")
    assert faces and kinds(faces, "surface") == ["cylinder"]


# ---- Follow Me ---------------------------------------------------------------

def test_follow_me_straight_path_is_an_extrusion():
    scene, h = doc()
    circle(h)
    run(h, "follow_me", face=scene.mesh.faces[0].key, path=[[0, 0, 0], [0, 0, 3]])
    assert kinds(curves(scene, "face"), "surface") == ["cylinder"]


def test_follow_me_about_an_axis_is_a_revolution_and_torus():
    scene, h = doc()
    run(h, "add_face", outer=[[3, 0, 0], [4, 0, 0], [4, 0, 1], [3, 0, 1]])
    pid = store(scene).id_of("face", scene.mesh.faces[0].key)
    run(h, "follow_me", face=scene.mesh.faces[0].key, path=ring(2, 24), closed=True)
    faces = curves(scene, "face")
    assert kinds(faces, "surface") == ["cylinder"]
    radii = sorted({round(s["surface"]["cylinder"]["radius"], 9) for s in faces.values()})
    assert radii == [3, 4]
    # Annular end faces are planar with exact circular sides.
    assert kinds(curves(scene), "curve") == ["circle"]
    del pid
    tor, h2 = doc()
    run(h2, "add_face", outer=[[3 + 0.5 * math.cos(a), 0, 0.5 * math.sin(a)]
                              for a in [TAU * i / 12 for i in range(12)]], curve=True, exact=True)
    run(h2, "follow_me", face=tor.mesh.faces[0].key, path=ring(2, 24), closed=True)
    faces = curves(tor, "face")
    assert kinds(faces, "surface") == ["torus"] and len(faces) == len(tor.mesh.faces)
    (t,) = {json.dumps(s, sort_keys=True) for s in faces.values()}
    t = json.loads(t)["surface"]["torus"]
    assert t["major"] == pytest.approx(3) and t["minor"] == pytest.approx(0.5)


def test_follow_me_cone_from_a_slanted_profile():
    scene, h = doc()
    run(h, "add_face", outer=[[1, 0, 0], [2, 0, 0], [1, 0, 1]])
    run(h, "follow_me", face=scene.mesh.faces[0].key, path=ring(1, 24), closed=True)
    assert "cone" in kinds(curves(scene, "face"), "surface")


def test_follow_me_with_a_mitred_bend_keeps_both_spans_cylindrical():
    scene, h = doc()
    circle(h, r=0.5)
    run(h, "follow_me", face=scene.mesh.faces[0].key, path=[[0, 0, 0], [0, 0, 2], [0, 2, 4]])
    faces = curves(scene, "face")
    # Each straight span is its own cylinder; the joint is their common
    # elliptic section (the mitre plane bisects the path directions).
    assert kinds(faces, "surface") == ["cylinder"]
    assert len({json.dumps(s, sort_keys=True) for s in faces.values()}) == 2
    assert kinds(curves(scene), "curve") == ["circle", "ellipse"]
    assert len(curves(scene)) == len(scene.mesh.edges) - 48


# ---- Offset ------------------------------------------------------------------

def test_offset_of_a_disk_is_concentric_and_exact():
    scene, h = doc()
    circle(h, r=2)
    st = store(scene)
    fid = st.id_of("face", scene.mesh.faces[0].key)
    run(h, "offset_outline", face=scene.mesh.faces[0].key, distance=0.5)
    radii = sorted({round(r["curve"]["circle"]["radius"], 9) for r in curves(scene).values()})
    assert len(radii) == 2 and radii[1] == pytest.approx(2)
    assert radii[0] == pytest.approx(2 - 0.5 / math.cos(math.pi / 24), abs=1e-6) or radii[0] == pytest.approx(1.5)
    assert st.keys_of_ids("face", [fid])[0] is not None


# ---- Move, rotate, scale -----------------------------------------------------

@pytest.mark.parametrize("op,args,check", [
    ("move_vertices", dict(delta=[1, 2, 3]), lambda s: s["frame"]["origin"][:2] == pytest.approx([1, 2])),
    ("rotate_selection", dict(axis=[1, 0, 0], center=[0, 0, 0], angles=[90]),
     lambda s: [abs(x) for x in s["frame"]["axis"]] == pytest.approx([0, 1, 0])),
    ("scale_selection", dict(anchor=[0, 0, 0], factors=[2, 2, 2]), lambda s: s["radius"] == pytest.approx(2)),
    ("scale_selection", dict(anchor=[0, 0, 0], factors=[1, 1, 3]), lambda s: s["radius"] == pytest.approx(1)),
])
def test_transforms_carry_the_support(op, args, check):
    scene, h = cylinder()
    before = bindings(scene)
    run(h, op, vertices=[v.key for v in scene.mesh.vertices], **args)
    faces = curves(scene, "face")
    assert len(faces) == 24 and kinds(faces, "surface") == ["cylinder"]
    assert check(next(iter(faces.values()))["surface"]["cylinder"])
    assert len(curves(scene)) == 48 and store(scene).exact_report[3] == 0
    h.undo()
    assert bindings(scene) == before


def test_nonuniform_scale_gives_ellipses_and_an_elliptic_extrusion():
    scene, h = cylinder()
    run(h, "scale_selection", vertices=[v.key for v in scene.mesh.vertices], anchor=[0, 0, 0], factors=[2, 1, 1])
    assert kinds(curves(scene), "curve") == ["ellipse"]
    faces = curves(scene, "face")
    assert len(faces) == 24 and kinds(faces, "surface") == ["extrusion"]
    e = next(iter(curves(scene).values()))["curve"]["ellipse"]
    assert (e["major"], e["minor"]) == pytest.approx((2, 1))


def test_partial_vertex_move_facets_only_the_affected_patch():
    scene, h = cylinder()
    before = bindings(scene)
    v = scene.mesh.vertices[0]
    p = v.position
    # Radially off the cylinder: the patch around it cannot stay exact.
    run(h, "move_vertices", vertices=[v.key], delta=[0.3 * p.x(), 0.3 * p.y(), 0])
    edges, faces = curves(scene), curves(scene, "face")
    assert 40 <= len(edges) < 48 and 18 <= len(faces) < 24
    assert store(scene).exact_report[3] > 0
    h.undo()
    assert bindings(scene) == before


# ---- divide, join, erase, merge ---------------------------------------------

def test_divide_splits_the_interval_on_the_curve():
    scene, h = doc()
    circle(h, n=6)
    st = store(scene)
    e = scene.mesh.edges[0]
    # A curve divides as a whole: four pieces add two vertices mid-segment,
    # which go onto the circle (the disk stays planar).
    run(h, "divide_edges", edges=[e.key], n=4)
    rows = curves(scene)
    assert len(rows) == 8 == len(scene.mesh.edges)
    assert len({json.dumps(r["curve"], sort_keys=True) for r in rows.values()}) == 1
    for v in scene.mesh.vertices:
        assert math.hypot(v.position.x(), v.position.y()) == pytest.approx(1, abs=1e-12)
    total = sum(abs(r["interval"][1] - r["interval"][0]) for r in rows.values())
    assert total == pytest.approx(TAU)
    h.undo()
    assert len(curves(scene)) == 6


def test_vertex_moved_along_a_generator_stays_on_the_cylinder():
    scene, h = cylinder()
    v = scene.mesh.vertices[0]
    run(h, "move_vertices", vertices=[v.key], delta=[0, 0, 0.3])
    # Still on the surface: its rim edges re-trim as sections of the
    # cylinder, the walls keep the support.
    assert len(curves(scene)) == 48
    assert kinds(curves(scene, "face"), "surface") == ["cylinder"]


def test_divide_a_cylinder_rim_facets_the_walls_it_would_bend():
    scene, h = cylinder()
    rim = [e for e in scene.mesh.edges if store(scene).exact_edge(e.key)
           and abs(e.v0.position.z()) < 1e-12][0]
    before = len(curves(scene, "face"))
    run(h, "divide_edges", edges=[rim.key], n=5)
    # The new vertices cannot leave the planar wall facets: those facets
    # and the rim pieces fall back to facets, the rest stays exact.
    assert 0 < len(curves(scene, "face")) < before
    assert store(scene).exact_report[3] > 0


def test_erase_releases_bindings_and_undo_restores_them():
    scene, h = cylinder()
    before = bindings(scene)
    walls = [f.key for f in scene.mesh.faces if store(scene).exact_face(f.key)]
    run(h, "erase", faces=walls[:5])
    assert len(curves(scene, "face")) == 19
    h.undo()
    assert bindings(scene) == before


def test_line_across_disk_splits_it_and_both_halves_keep_exact_sides():
    scene, h = doc()
    circle(h, n=24)
    run(h, "add_edge", a=[1, 0, 0], b=[-1, 0, 0])
    assert len(scene.mesh.faces) == 2 and len(curves(scene)) == 24
    import numpy as np
    d = store(scene).display_arrays()
    p = np.asarray(d["pos"]).reshape(-1, 3)
    rim = np.hypot(p[:, 0], p[:, 1])
    assert (np.abs(rim - 1) < 1e-12).sum() == 24
    # Erasing the diameter merges the halves back; the rim stays exact.
    diameter = [e for e in scene.mesh.edges if not store(scene).exact_edge(e.key)][0]
    run(h, "erase", edges=[diameter.key])
    assert len(scene.mesh.faces) == 1 and len(curves(scene)) == 24


# ---- Intersect faces ---------------------------------------------------------

@pytest.mark.parametrize("tilt,kind", [(0, "circle"), (0.5, "ellipse")])
def test_intersect_cylinder_with_a_plane(tilt, kind):
    scene, h = cylinder(height=2)
    z = lambda x: 1 + tilt * x  # noqa: E731
    run(h, "add_face", outer=[[-3, -3, z(-3)], [3, -3, z(3)], [3, 3, z(3)], [-3, 3, z(-3)]])
    st = store(scene)
    walls_before = len(curves(scene, "face"))
    plane = [f for f in scene.mesh.faces if len(f.vertices) == 4 and not st.exact_face(f.key)
             and abs(f.vertices[0].x()) == 3][0]
    run(h, "intersect_faces", mode="model", faces=[plane.key])
    rows = curves(scene)
    assert kind in kinds(rows, "curve")
    cut = [r for r in rows.values() if kind in r["curve"]]
    assert len(cut) >= 24
    # Split walls keep the cylinder on both halves.
    assert len(curves(scene, "face")) >= 2 * walls_before


# ---- persistence, failure ----------------------------------------------------

def test_make_group_and_explode_keep_bindings_through_the_placement():
    from built_core.history import MakeGroupCommand
    scene, h = cylinder()
    want = len(curves(scene)), len(curves(scene, "face"))
    h.execute(MakeGroupCommand(list(scene.mesh.faces), list(scene.mesh.edges)))
    group = scene.groups[0]
    assert group.mesh._store.exact_counts() == want
    run(h, "explode_group", placement=group.uid)
    assert store(scene).exact_counts() == want
    assert kinds(curves(scene, "face"), "surface") == ["cylinder"]


# ---- line across a curved face ---------------------------------------------

def test_line_around_a_cylinder_with_line_on_surface_follows_generators():
    scene, h = cylinder(height=2)
    st = store(scene)
    # Two points at one height on facets a quarter turn apart.
    a = [0.99 * math.cos(0.13), 0.99 * math.sin(0.13), 1.0]
    b = [0.99 * math.cos(1.70), 0.99 * math.sin(1.70), 1.0]
    def on_facet(p):
        # Project onto the facet plane under p along the radius.
        k = math.floor(math.atan2(p[1], p[0]) / (TAU / 24))
        t0, t1 = k * TAU / 24, (k + 1) * TAU / 24
        mid = (t0 + t1) / 2
        r = math.cos(TAU / 48) / math.cos(math.atan2(p[1], p[0]) - mid)
        ang = math.atan2(p[1], p[0])
        return [r * math.cos(ang), r * math.sin(ang), p[2]]
    a, b = on_facet(a), on_facet(b)
    path = st.surface_path(a, b)
    # The ends move to the nearest generators: every corner is on the
    # cylinder at the line's height, so every segment is its circle.
    assert path is not None and len(path) >= 6
    for p in path:
        assert math.hypot(p[0], p[1]) == pytest.approx(1, abs=1e-12) and p[2] == pytest.approx(1)
    run(h, "add_edges", segments=[[p, q] for p, q in zip(path, path[1:])])
    at_one = [r for r in curves(scene).values()
              if "circle" in r["curve"] and r["curve"]["circle"]["frame"]["origin"][2] == pytest.approx(1)]
    assert len(at_one) == len(path) - 1
    assert kinds(curves(scene, "face"), "surface") == ["cylinder"]
    assert len(curves(scene, "face")) == 24 + len(path) - 1
    # The drawn circle stays a visible edge; the split generators stay soft.
    import numpy as np
    d = st.display_arrays(True)
    owner, flags = np.arange(len(d["edges"])), np.asarray(d["flags"])
    for i, e in enumerate(scene.mesh.edges):
        a, b = e.v0.position, e.v1.position
        if abs(a.z() - 1) < 1e-9 and abs(b.z() - 1) < 1e-9:
            assert ((flags[owner == i] & 1) == 0).all()
        elif abs(a.x() - b.x()) < 1e-10 and abs(a.y() - b.y()) < 1e-10:
            assert flags[i] & 1
    assert st.surface_path([5, 5, 5], [6, 6, 6]) is None


@pytest.mark.parametrize("span", [1, 6])
def test_chord_mode_of_plain_line_stays_straight_across_facets(span):
    scene, h = cylinder(height=2)
    st = store(scene)
    k = 3
    p = lambda t, z: [math.cos(TAU * t / 24), math.sin(TAU * t / 24), z]  # noqa: E731
    run(h, "add_edge", a=p(k, 1), b=p(k + span, 1), chord=True)
    chord = [e for e in scene.mesh.edges if abs(e.v0.position.z() - 1) < 1e-12 and abs(e.v1.position.z() - 1) < 1e-12]
    assert chord and not any(st.exact_edge(e.key) for e in chord)
    # The surface tool uses this parameter section along a wall facet.
    other, h2 = cylinder(height=2)
    run(h2, "add_edge", a=p(k, 1), b=p(k + 1, 1))
    arc = [e for e in other.mesh.edges if abs(e.v0.position.z() - 1) < 1e-12 and abs(e.v1.position.z() - 1) < 1e-12]
    assert arc and all(store(other).exact_edge(e.key) for e in arc)


def test_save_open_and_pickle_keep_bindings(tmp_path):
    import pickle
    from formats import igz
    scene, h = cylinder()
    want = bindings(scene)
    path = tmp_path / "cyl.built"
    igz.save_scene(scene, path)
    opened = Scene()
    igz.load_into(opened, path)
    assert bindings(opened) == want
    copy = pickle.loads(pickle.dumps(scene.mesh))
    assert copy._store.exact_counts() == (48, 24)


def test_refused_step_leaves_bindings_unchanged():
    scene, h = cylinder()
    before = bindings(scene)
    result = commands.run(h, "push_pull", face=cap(scene, 2).key, distance=0)
    assert not result["ok"]
    assert bindings(scene) == before


def test_imported_step_torus_seams_draw_soft_with_exact_normals():
    from pathlib import Path
    import numpy as np
    from formats import step
    data = step.prepare_step(Path(__file__).parent / "fixtures" / "step" / "stepmesh" / "torus.step")
    scene = Scene()
    step.apply_step(scene, data)
    st = scene.groups[0].mesh._store
    assert st.native_exact and st.counts() == (1, 2, 1)
    d = st.display_arrays(True)
    owner, flags = np.asarray(d["edge_owner"]), np.asarray(d["flags"])
    # One face, two seams each used twice: neither is a crease.
    assert all(((flags[owner == e] & 1) == 1).all() for e in (0, 1))
    n = np.asarray(d["corner_normals"])
    assert len(n) == len(d["corners"]) and np.isfinite(n).all()
    assert np.allclose(np.linalg.norm(n, axis=1), 1, atol=1e-6)


def test_native_cylinder_seam_draws_soft():
    import os
    import sys
    sys.path.insert(0, os.path.dirname(__file__))
    from test_exact_geometry import native_cylinder
    import numpy as np
    _, _, group = native_cylinder()
    st = group.mesh._store
    d = st.display_arrays(True)
    owner, flags = np.asarray(d["edge_owner"]), np.asarray(d["flags"])
    assert ((flags[owner == 2] & 1) == 1).all()
    assert all(((flags[owner == r] & 1) == 0).all() for r in (0, 1))


# ---- display: conforming tessellation, exact normals, cache ----------------

def revolved(kind):
    scene, h = doc()
    if kind == "torus":
        run(h, "add_face", outer=[[3 + 0.8 * math.cos(a), 0, 0.8 * math.sin(a)]
                                  for a in [TAU * i / 12 for i in range(12)]], curve=True, exact=True)
        run(h, "follow_me", face=scene.mesh.faces[0].key, path=ring(2, 16), closed=True)
    elif kind == "cone":
        run(h, "add_face", outer=[[0.2, 0, 0], [1.5, 0, 0], [0.2, 0, 2]])
        run(h, "follow_me", face=scene.mesh.faces[0].key, path=ring(1, 16), closed=True)
    elif kind == "sphere":
        semi = [[math.cos(a), 0, math.sin(a)] for a in [-math.pi / 2 + math.pi * i / 12 for i in range(13)]]
        run(h, "add_arc", points=semi)
        run(h, "add_edge", a=semi[-1], b=semi[0])
        run(h, "follow_me", face=scene.mesh.faces[0].key, path=ring(0.5, 16), closed=True)
    elif kind == "mitred":
        circle(h, r=0.5, n=12)
        run(h, "follow_me", face=scene.mesh.faces[0].key, path=[[0, 0, 0], [0, 0, 2], [0, 2, 4]])
    else:
        circle(h, r=1, n=12)
        run(h, "push_pull", face=scene.mesh.faces[0].key, distance=2.5)
    return scene, h


@pytest.mark.parametrize("kind", ["torus", "cone", "sphere", "cylinder", "mitred"])
def test_bound_faces_draw_the_editable_facets(kind):
    import numpy as np
    scene, _ = revolved(kind)
    st = store(scene)
    display, logical = st.display_arrays(True), st.arrays()
    for field in ("pos", "edges", "flags", "corners", "ring_sizes", "ring_counts"):
        np.testing.assert_array_equal(display[field], logical[field])
    normals = display["corner_normals"]
    finite = normals[np.isfinite(normals).all(axis=1)]
    assert np.allclose(np.linalg.norm(finite, axis=1), 1)
    if kind == "cylinder":
        assert len(finite) == 4 * 12
        pos = display["pos"][display["corners"]]
        valid = np.isfinite(normals[:, 0])
        np.testing.assert_allclose(abs((normals[valid, :2] * pos[valid, :2]).sum(axis=1)), 1)


def test_hardened_cylinder_shades_flat():
    import numpy as np
    scene, _ = cylinder()
    for edge in scene.mesh.edges:
        edge.soft = False
    assert np.isnan(store(scene).display_arrays()["corner_normals"]).all()


def test_helix_segment_can_be_softened_and_smoothed():
    from built_core.soften import SoftenByAngleCommand, selected_edges
    scene, h = cylinder(n=12)
    a, b = [1, 0, 0.25], [math.cos(TAU/12), math.sin(TAU/12), 1.75]
    run(h, "add_edge", a=a, b=b, chord=True)
    diagonal = next(e for e in scene.mesh.edges if abs(abs(e.a.z()-e.b.z())-1.5) < 1e-7)
    assert not store(scene).exact_edge(diagonal.key)
    scene.select([diagonal])
    h.execute(SoftenByAngleCommand(selected_edges(scene), 180))
    assert diagonal.soft and not diagonal.flat
    h.undo()
    assert not diagonal.soft


def test_erase_branch_rejoins_straight_line_and_keeps_longer_id():
    scene, h = doc()
    run(h, "add_edge", a=[0, 0, 0], b=[3, 0, 0])
    run(h, "add_edge", a=[1, 0, 0], b=[1, 2, 0])
    branch = next(e for e in scene.mesh.edges if e.a.y() or e.b.y())
    longer = next(e for e in scene.mesh.edges if max(e.a.x(), e.b.x()) == 3)
    keep = store(scene).id_of("edge", longer.key)
    run(h, "erase", edges=[branch.key])
    assert len(scene.mesh.edges) == 1
    assert store(scene).id_of("edge", scene.mesh.edges[0].key) == keep
    h.undo()
    assert len(scene.mesh.edges) == 3
    h.redo()
    assert len(scene.mesh.edges) == 1


@pytest.mark.parametrize("reopen", [False, True])
def test_erasing_an_arc_branch_restores_its_authored_segments(tmp_path, reopen):
    scene, h = doc()
    run(h, "add_arc", points=ring(n=12)[:5])
    e = scene.mesh.edges[1]
    p = (e.a + e.b) * 0.5
    run(h, "add_edge", a=[p.x(), p.y(), 0], b=[p.x(), p.y(), 1])
    if reopen:
        from formats import igz
        path = tmp_path / "branched-arc.built"
        igz.save_scene(scene, path)
        scene, h = doc()
        igz.load_into(scene, path)
    branch = next(e for e in scene.mesh.edges if e.a.z() or e.b.z())
    run(h, "erase", edges=[branch.key])
    assert len(scene.mesh.edges) == len(curves(scene)) == 4
    assert len({e.curve for e in scene.mesh.edges}) == 1


def test_weld_group_exact_fit_and_undo():
    from built_core.history import WeldEdgesCommand
    scene, h = doc()
    run(h, "add_face", outer=ring(n=12))
    keys = [e.key for e in scene.mesh.edges]
    assert not store(scene).can_weld_edges(keys[::2])
    assert store(scene).can_weld_edges(keys)
    h.execute(WeldEdgesCommand(keys))
    assert len(scene.mesh.curve_edges(scene.mesh.edges[0])) == len(curves(scene)) == 12
    h.undo()
    assert len(curves(scene)) == 0 and all(e.curve is None for e in scene.mesh.edges)
    h.redo()
    assert len(curves(scene)) == 12


def test_make_group_keeps_element_ids():
    from built_core.history import MakeGroupCommand
    scene, h = revolved("cylinder")
    st = store(scene)
    ids = {k: sorted(int(i) for i in st.ids(k)) for k in ("vertex", "edge", "face")}
    h.execute(MakeGroupCommand(list(scene.mesh.faces), list(scene.mesh.edges)))
    g = scene.groups[0].mesh._store
    assert {k: sorted(int(i) for i in g.ids(k)) for k in ("vertex", "edge", "face")} == ids
    h.undo()
    h.redo()
    g = scene.groups[0].mesh._store
    assert sorted(int(i) for i in g.ids("face")) == ids["face"]


def test_dividing_a_soft_generator_keeps_every_piece_soft_through_save(tmp_path):
    from formats import igz

    scene, h = cylinder(n=12)
    generator = next(e for e in scene.mesh.edges if e.soft)
    x, y, _ = generator.a.toTuple()
    run(h, "divide_edges", edges=[generator.key], n=3)
    path = tmp_path / "split.built"
    igz.save_scene(scene, path)
    reopened = Scene()
    igz.load_into(reopened, path)
    pieces = [e for e in reopened.mesh.edges if e.a.x() == e.b.x() == x and e.a.y() == e.b.y() == y]
    assert len(pieces) == 3 and all(e.soft and not e.flat and len(e.faces) == 2 for e in pieces)


def test_welding_a_non_circular_chain_drops_partial_circle_bindings():
    from built_core.history import WeldEdgesCommand

    scene, hist = doc()
    points = ring(n=12)[:4]
    run(hist, "add_arc", points=points)
    run(hist, "add_edge", a=points[-1], b=[0, 2, 1])
    previous = bindings(scene)
    hist.execute(WeldEdgesCommand([e.key for e in scene.mesh.edges]))
    assert len({e.curve for e in scene.mesh.edges}) == 1
    assert not curves(scene)
    hist.undo()
    assert bindings(scene) == previous


@pytest.mark.parametrize("different_paint", [False, True])
def test_erasing_coplanar_seam_preserves_material_boundary_corners(different_paint):
    from built_core import Vec3
    scene, hist = doc()
    mesh = scene.mesh
    for left, right in ((0, 2), (2, 3)):
        mesh.add_face([Vec3(left, 0, 0), Vec3(right, 0, 0), Vec3(right, 2, 0), Vec3(left, 2, 0)])
    if different_paint:
        mesh.faces[0].attrs["color"] = [1., 0., 0.]
    seam = next(e for e in mesh.edges if len(e.faces) == 2)
    longer = next(e for e in mesh.edges if e.a.y() == e.b.y() == 0 and abs(e.a.x()-e.b.x()) == 2)
    pid = mesh._store.id_of("edge", longer.key)
    run(hist, "erase", edges=[seam.key])
    assert len(mesh.faces) == 1
    assert len(mesh.faces[0].vertices) == (6 if different_paint else 4)
    assert any(mesh._store.id_of("edge", e.key) == pid for e in mesh.edges)
    hist.undo()
    assert len(mesh.faces) == 2


def test_erasing_branch_at_authored_arc_corner_rejoins_group_and_keeps_vertex(tmp_path):
    from formats import igz

    scene, hist = doc()
    points = ring(n=12)[:5]
    run(hist, "add_arc", points=points)
    run(hist, "add_edge", a=points[2], b=[0, 0, 0])
    assert len({e.curve for e in scene.mesh.edges if e.curve is not None}) == 2
    path = tmp_path / "original-corner.built"
    igz.save_scene(scene, path)
    reopened, history = doc()
    igz.load_into(reopened, path)
    branch = next(e for e in reopened.mesh.edges if e.curve is None)
    run(history, "erase", edges=[branch.key])
    assert len(reopened.mesh.vertices) == 5
    assert len(curves(reopened)) == 4
    assert len({e.curve for e in reopened.mesh.edges}) == 1
