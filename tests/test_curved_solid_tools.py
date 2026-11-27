# SPDX-License-Identifier: GPL-3.0-or-later
"""Boolean face provenance across modelling, history, and exchange."""
from pathlib import Path

import numpy as np
import pytest

from built_core import solids
from built_core.group import Group
from built_core.history import History
from built_core.scene import Scene
from formats import igz, step
from formats.skp import load_skp
from tests.test_exact_edit import cylinder, doc, revolved, ring, run
from tests.test_solid_tools import BLUE, _box


def body(kind):
    if kind == "torus":
        scene, hist = doc()
        profile = [[.03 + p[0], 0, p[1]] for p in ring(.008, 12)]
        run(hist, "add_face", outer=profile, curve=True, exact=True)
        run(hist, "follow_me", face=scene.mesh.faces[0].key, path=ring(.02, 16), closed=True)
        return Group(scene.mesh)
    if kind == "mitred":
        return Group(revolved(kind)[0].mesh)
    if kind == "cylinder":
        return Group(cylinder(n=12)[0].mesh)
    scene = Scene()
    if kind == "step":
        source = Path(__file__).parent / "fixtures/step/capped-cylinder.step"
        step.apply_step(scene, step.prepare_step(source))
    else:
        # The shipped file contains a circular disk, a partial disk, and
        # loose arcs. Extrude the recovered disk in its original store.
        path = Path(__file__).parent / "data/skp_arcs_2021.skp"
        load_skp(scene, path)
        mesh = scene.groups[0].mesh
        disk = next(f for f in mesh.faces if len(f.vertices) == 24)
        for f in list(mesh.faces):
            if f != disk:
                mesh.remove_face(f)
        for e in list(mesh.edges):
            if not e.faces:
                mesh.remove_edge(e)
        assert sum(bool(mesh._store.exact_circle_interval(e.key)) for e in mesh.edges) == 24
        model, hist = doc()
        model.mesh = mesh
        run(hist, "push_pull", face=disk.key, distance=.254)
        run(hist, "recover_exact_geometry", faces=[f.key for f in mesh.faces],
            tolerance=.002, preview=False)
        assert sum(bool(mesh._store.exact_source_strip(f.key)) for f in mesh.faces) == 24

    return scene.groups[0]


def exact(mesh):
    st = mesh._store
    return (st.native_exact,
            sorted(v for e in mesh.edges if (v := st.exact_edge(e.key))),
            sorted(v for f in mesh.faces if (v := st.exact_face(f.key))),
            sum(bool(st.exact_circle_interval(e.key)) for e in mesh.edges))


def assert_draws_facets(mesh):
    dense = mesh._store.arrays()
    drawn = mesh._store.display_arrays()
    for key in ("edges", "corners", "ring_sizes", "ring_counts"):
        np.testing.assert_array_equal(drawn[key], dense[key])
    np.testing.assert_allclose(drawn["pos"], dense["pos"])


@pytest.mark.parametrize("kind", ["cylinder", "torus", "mitred", "step", "skp"])
@pytest.mark.parametrize("op", ["union", "subtract", "trim", "intersect", "split", "outer_shell"])
def test_half_cut(kind, op, tmp_path):
    subject = body(kind)
    pts = np.asarray(subject.mesh._store.display_arrays()["pos"])
    lo, hi = pts.min(axis=0), pts.max(axis=0)
    margin = max(hi-lo) / 5
    # The pipe and torus are symmetric across x=0. A circular profile
    # with an odd segment count is still symmetric at half its height.
    axis = 0 if kind in ("torus", "mitred") else 2
    lo, hi = lo-margin, hi+margin
    hi[axis] = (lo[axis] + hi[axis]) / 2
    cutter = _box(*lo, *hi, BLUE)
    for f in subject.mesh.faces:
        f.attrs["source"] = "body"
    volume, box = map(solids.solid_volume, (subject, cutter))
    half = volume / 2
    expected = {"union": [box + half], "outer_shell": [box + half],
                "intersect": [half], "subtract": [half], "trim": [half],
                "split": [half, half, box - half]}[op]
    original = exact(subject.mesh)
    assert any(original)
    scene = Scene()
    scene.groups += [cutter, subject]
    hist = History(scene)
    hist.execute(solids.SolidOperationCommand(op, [cutter, subject]))
    assert not hist.last_error and not hist.last_rejected
    results = [g for g in scene.groups if g is not cutter]
    assert sorted(solids.solid_volume(g) for g in results) == pytest.approx(sorted(expected), rel=1e-5)
    assert exact(subject.mesh) == original
    counts = [exact(g.mesh) for g in results]
    assert any(s[2] for s in counts)
    for g in results:
        assert_draws_facets(g.mesh)
        for e in g.mesh.edges:
            assert len(e.faces) == 2
            a, b = e.faces
            if bool(a.attrs.get("source")) != bool(b.attrs.get("source")):
                assert not e.soft
        if any(f.attrs.get("source") for f in g.mesh.faces):
            assert any(e.soft for e in g.mesh.edges)
    hist.undo()
    assert scene.groups == [cutter, subject] and exact(subject.mesh) == original
    hist.redo()
    results = [g for g in scene.groups if g is not cutter]
    assert [exact(g.mesh) for g in results] == counts
    path = tmp_path / "cut.built"
    igz.save_scene(scene, path)
    reopened = Scene()
    igz.load_into(reopened, path)
    assert [exact(g.mesh) for g in reopened.groups] == [exact(g.mesh) for g in scene.groups]
    # Export individual replacements so a surviving trim cutter cannot
    # hide a missing or invalid result body in the STEP assembly.
    for i, g in enumerate(results):
        export = Scene()
        export.groups += [Group(g.mesh)]
        path = tmp_path / f"result-{i}.step"
        step.save_step(export, path)
        imported = step.prepare_step(path)
        assert imported["groups"] and not imported["problems"]


def test_drilled_plate_keeps_analytic_bore(tmp_path):
    cutter = body("cylinder")
    plate = _box(-2, -2, .25, 2, 2, 1.75, BLUE)
    _, (result,) = solids.run("subtract", [cutter, plate])
    assert solids.solid_volume(result) == pytest.approx(24 - 4.5)
    assert len(exact(result.mesh)[2]) == 12
    scene = Scene()
    scene.groups += [result]
    path = tmp_path / "drilled.step"
    step.save_step(scene, path)
    assert "= CYLINDRICAL_SURFACE(" in path.read_text()
    assert not step.prepare_step(path)["problems"]


@pytest.mark.parametrize("kind", ["step", "cylinder"])
def test_curved_instance_cut_uses_placed_supports(kind):
    from built_core import Matrix4

    subject = body(kind)
    before = exact(subject.mesh)
    xf = Matrix4()
    xf.translate(3., -2., 1.)
    subject.xform = xf
    bounds = np.asarray(subject.mesh._store.display_arrays()["pos"])
    mid = 1. + (bounds[:, 2].min() + bounds[:, 2].max()) / 2
    box = _box(.5, -4., 0., 5., 0., mid, BLUE)
    volume = solids.solid_volume(subject)
    _, (cut,) = solids.run("subtract", [box, subject])
    assert solids.solid_volume(cut) == pytest.approx(volume / 2)
    assert exact(subject.mesh) == before and exact(cut.mesh)[2]
    assert min(p.position.x() for p in cut.mesh.vertices) >= 2.


def test_off_support_intersection_leaves_only_affected_facets_unbound():
    cylinder_group = body("cylinder")
    for face in cylinder_group.mesh.faces:
        if cylinder_group.mesh._store.exact_face(face.key):
            face.attrs["source"] = "wall"
    cutter = _box(.32, -2., -1., 2., 2., 3., BLUE)
    _, (result,) = solids.run("subtract", [cutter, cylinder_group])
    walls = [f for f in result.mesh.faces if f.attrs.get("source") == "wall"]
    st = result.mesh._store
    touched = [f for f in walls if any(abs(v.x()-.32) < 1e-9 for v in f.vertices)]
    assert len(touched) == 2
    assert all(not st.exact_face(f.key) for f in touched)
    assert all(st.exact_face(f.key) for f in walls if f not in touched)
