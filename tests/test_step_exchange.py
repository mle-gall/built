# SPDX-License-Identifier: GPL-3.0-or-later
from pathlib import Path

import pytest

from built_core.scene import Scene
from formats import step


def test_cylinder_seam_and_cap_survive_exchange(tmp_path):
    src = Path(__file__).parent / "fixtures" / "step" / "capped-cylinder.step"
    prepared = step.prepare_step(src)
    assert (prepared["exact"], prepared["faceted"], prepared["problems"]) == (1, 0, [])
    scene = Scene()
    step.apply_step(scene, prepared)
    mesh = scene.groups[0].mesh
    assert mesh._store.native_exact
    assert (len(mesh.vertices), len(mesh.edges), len(mesh.faces)) == (2, 3, 3)
    path = tmp_path / "roundtrip.step"
    step.save_step(scene, path)
    doc = step.prepare_step(path)
    assert doc["exact"] == 1 and not doc["problems"]
    text = path.read_text()
    assert text.count("= CYLINDRICAL_SURFACE(") == 1
    assert text.count("= CIRCLE(") == 2


def test_partial_import_requires_acceptance():
    scene = Scene()
    with pytest.raises(ValueError, match="confirmation"):
        step.apply_step(scene, {"problems": ["unsupported body"], "groups": []})
    assert not scene.groups


@pytest.mark.parametrize("year", [2017, 2021])
def test_source_arcs_keep_edge_ids_and_file_bindings(tmp_path, year):
    from formats import igz
    from formats.skp import load_skp

    scene = Scene()
    path = Path(__file__).parent / "data" / f"skp_arcs_{year}.skp"
    load_skp(scene, path)

    def intervals(model):
        mesh = model.groups[0].mesh
        return sorted((mesh._store.id_of("edge", e.key), interval)
                      for e in mesh.edges
                      if (interval := mesh._store.exact_circle_interval(e.key)))

    original = intervals(scene)
    assert len(original) == 48
    saved = tmp_path / "arcs.built"
    igz.save_scene(scene, saved)
    reopened = Scene()
    igz.load_into(reopened, saved)
    assert intervals(reopened) == original


def test_planar_recovery_and_make_faceted_are_undoable():
    from built_core import Vec3
    from built_core.history import History
    from built_core.protocol import commands

    scene = Scene()
    mesh = scene.mesh
    face = mesh.add_face([Vec3(0, 0, 0), Vec3(2, 0, 0), Vec3(0, 2, 1)])
    key = face.key
    history = History(scene)
    plan = commands.run(history, "recover_exact_geometry", faces=[key])
    assert plan["faces"] == 1 and plan["max_residual_m"] < 1e-12
    assert mesh._store.exact_plane(key) is None
    assert commands.run(history, "recover_exact_geometry", faces=[key], preview=False)["ok"]
    assert mesh._store.exact_plane(key)
    assert commands.run(history, "make_faceted", faces=[key])["ok"]
    assert mesh._store.exact_plane(key) is None
    history.undo()
    assert mesh._store.exact_plane(key)


def test_open_sheet_and_two_placements(tmp_path):
    from built_core import Matrix4, Vec3
    from built_core.group import Group
    from built_core.mesh import Mesh

    mesh = Mesh()
    mesh.add_face([Vec3(0, 0, 0), Vec3(0.1, 0, 0), Vec3(0, 0.2, 0)])
    scene = Scene()
    for x in (1, 2):
        g = Group(mesh, "Sheet")
        g.xform = Matrix4()
        g.xform.translate(x, 0, 0)
        scene.groups.append(g)
    path = tmp_path / "placed.step"
    step.save_step(scene, path)
    doc = step.prepare_step(path)
    assert not doc["problems"] and doc["exact"] == 1
    root, = doc["groups"]
    a, b = root.children
    assert a.mesh is b.mesh
    assert sorted(g.xform.copyDataTo()[3] for g in (a, b)) == [1, 2]


def ruled_band(n=16, top_radius=1., smooth=True, scale=0.01):
    import math

    def rim(z, r):
        return [(r*math.cos(math.tau*i/n), r*math.sin(math.tau*i/n), z) for i in range(n)]
    lower, upper = rim(0., scale), rim(2.*scale, top_radius*scale)
    sides = [[lower[i], lower[(i+1)%n], upper[(i+1)%n], upper[i]] for i in range(n)]
    segments = [list(zip(r, r[1:]+r[:1])) for r in (lower, upper)]
    frames = [([(0, 0, z), (0, 0, 1), (1, 0, 0)], r, 0., math.tau, edges)
              for (z, r), edges in zip([(0., scale), (2.*scale, top_radius*scale)], segments)]
    return {"faces": [(r, [], {}) for r in [lower[::-1], upper, *sides]],
            "loose_edges": [], "curves": segments, "source_arcs": frames,
            "soft_edges": list(zip(lower, upper)) if smooth else []}


@pytest.mark.parametrize("radius,entity", [(1., "CYLINDRICAL_SURFACE"), (1.2, "CONICAL_SURFACE")])
def test_proven_original_strips_merge_only_on_exchange(tmp_path, radius, entity):
    from formats.skp import apply_payload

    scene = Scene()
    apply_payload(scene, {"backend": "openskp", "groups": [ruled_band(top_radius=radius)]})
    mesh = scene.groups[0].mesh
    info = [mesh._store.exact_source_strip(f.key) for f in mesh.faces]
    assert len(mesh.faces) == 18 and sum(s is not None for s in info) == 16
    assert len({s[0] for s in info if s}) == 1
    path = tmp_path / "merged.step"
    step.save_step(scene, path)
    assert path.read_text().count("= "+entity+"(") == 1
    assert step.prepare_step(path)["exact"] == 1


def test_hard_generators_stay_faceted():
    from formats.skp import apply_payload

    scene = Scene()
    apply_payload(scene, {"backend": "openskp", "groups": [ruled_band(smooth=False)]})
    mesh = scene.groups[0].mesh
    assert all(mesh._store.exact_source_strip(f.key) is None for f in mesh.faces)


def test_cylinder_fit_checks_chord_deviation_before_binding():
    from built_core import Vec3
    from built_core.history import History
    from built_core.protocol import commands

    scene = Scene()
    mesh = scene.mesh
    for outer, _, _ in ruled_band()["faces"]:
        mesh.add_face([Vec3(*p) for p in outer])
    keys = [f.key for f in mesh.faces]
    history = History(scene)

    def recover(**opts):
        return commands.run(history, "recover_exact_geometry", faces=keys, **opts)

    assert recover(tolerance=1e-6)["method"] == "Planar supports"
    plan = recover(tolerance=0.001)
    assert "circular" in plan["method"]
    assert 0 < plan["max_residual_m"] < 0.001
    assert plan["input_volume_m3"] < plan["output_volume_m3"]
    assert recover(tolerance=0.001, preview=False)["ok"]
    assert sum(mesh._store.exact_source_strip(k) is not None for k in keys) == 16
    history.undo()
    assert all(mesh._store.exact_source_strip(k) is None for k in keys)


@pytest.mark.parametrize("name,counts", [
    ("cavity", (1, 0)), ("display-mesh", (0, 1)), ("mixed-parts", (1, 1))
])
def test_cavity_and_tessellation_ownership(tmp_path, name, counts):
    from formats import igz

    src = Path(__file__).parent / "fixtures" / "step" / (name + ".step")
    doc = step.prepare_step(src)
    assert (doc["exact"], doc["faceted"]) == counts and not doc["problems"]
    scene = Scene()
    step.apply_step(scene, doc)
    save = tmp_path / "roundtrip.built"
    igz.save_scene(scene, save)
    reopened = Scene()
    igz.load_into(reopened, save)
    path = tmp_path / "restored.step"
    step.save_step(reopened, path)
    text = path.read_text()
    assert text.count("= BREP_WITH_VOIDS(") == counts[0]
    assert text.count("= ORIENTED_CLOSED_SHELL(") == counts[0]
    assert text.count("= NEXT_ASSEMBLY_USAGE_OCCURRENCE(") == (name == "mixed-parts")
    out = step.prepare_step(path)
    assert out["exact"] == sum(counts) and not out["problems"]


def test_authored_seam_pcurves_are_checked(tmp_path):
    src = Path(__file__).parent / "fixtures" / "step" / "seam-pcurves-ap214.step"
    doc = step.prepare_step(src)
    assert doc["exact"] == 1 and not doc["problems"]
    broken = tmp_path / "wrong-pcurve.step"
    broken.write_text(src.read_text().replace("(6.283185307179586,0.)", "(3.1415,0.)"))
    with pytest.raises(ValueError, match="No importable STEP bodies"):
        step.prepare_step(broken)


def test_mixed_units_scale_each_definition_in_its_own_context(tmp_path):
    src = Path(__file__).parent / "fixtures" / "step" / "mixed-parts.step"
    text = src.read_text().replace(
        "#6 = ( GEOMETRIC_REPRESENTATION_CONTEXT", "#999 = ( LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT($,.METRE.) );\n#6 = ( GEOMETRIC_REPRESENTATION_CONTEXT"
    )
    rows = [r.replace("((#1,#2,#3))", "((#999,#2,#3))") if r.startswith("#6 =") else r
            for r in text.splitlines()]
    path = tmp_path / "mixed-units.step"
    path.write_text("\n".join(rows) + "\n")
    doc = step.prepare_step(path)
    assert not doc["problems"]
    mesh_part = doc["groups"][0].children[0]
    assert max(v.position.x() for v in mesh_part.mesh.vertices) == pytest.approx(10.)
    assert mesh_part.xform.copyDataTo()[3] == pytest.approx(0.025)


def test_rational_sheet_roundtrip(tmp_path):
    src = Path(__file__).parent / "fixtures" / "step" / "rational-sheet.step"
    doc = step.prepare_step(src)
    assert doc["exact"] == 1 and not doc["problems"]
    scene = Scene()
    step.apply_step(scene, doc)
    output = tmp_path / "roundtrip.step"
    step.save_step(scene, output)
    text = output.read_text()
    assert "RATIONAL_B_SPLINE_SURFACE" in text
    assert text.count("RATIONAL_B_SPLINE_CURVE") == 2
    assert step.prepare_step(output)["exact"] == 1


@pytest.mark.parametrize("with_bore", [False, True])
def test_rounded_profile_and_bore_export_verified_surfaces(tmp_path, with_bore):
    import math
    from built_core import Vec3
    from built_core.history import History
    from built_core.protocol import commands

    scene = Scene()
    mesh = scene.mesh
    rounded = []
    for cx, cy, start in [(0.015, 0.01, 0), (-0.015, 0.01, 90),
                          (-0.015, -0.01, 180), (0.015, -0.01, 270)]:
        for j in range(5):
            a = math.radians(start + 90 * j / 4)
            rounded.append((cx + 0.005 * math.cos(a), cy + 0.005 * math.sin(a)))
    outer = rounded if not with_bore else [(-.02,-.015),(.02,-.015),(.02,.015),(-.02,.015)]
    rings = [outer]
    if with_bore:
        rings.append([(.007*math.cos(-math.tau*i/32), .007*math.sin(-math.tau*i/32))
                      for i in range(32)])
    def xyz(r, z):
        return [Vec3(x,y,z) for x,y in r]
    for z, rev in [(0, True), (.03, False)]:
        cap = [xyz(r[::-1] if rev else r,z) for r in rings]
        mesh.add_face(cap[0], cap[1:])
    for r in rings:
        for i in range(len(r)):
            a, b = r[i], r[(i+1)%len(r)]
            mesh.add_face([Vec3(*a,0),Vec3(*b,0),Vec3(*b,.03),Vec3(*a,.03)])
    keys = [f.key for f in mesh.faces]
    h = History(scene)
    report = commands.run(h, "recover_exact_geometry", faces=keys, tolerance=0.001)
    assert ("bore" if with_bore else "rounded-rectangle") in report["method"]
    assert report["parts"][0]["curved_supports"] == (1 if with_bore else 4)
    assert commands.run(h, "recover_exact_geometry", faces=keys, tolerance=0.001, preview=False)["ok"]
    out = tmp_path / "recovered.step"
    step.save_step(scene,out)
    assert out.read_text().count("= CYLINDRICAL_SURFACE(") == (1 if with_bore else 4)
    assert step.prepare_step(out)["exact"] == 1
    h.undo()
    assert all(mesh._store.exact_circle_interval(e.key) is None for e in mesh.edges)


@pytest.mark.parametrize("major,kind", [(0., "SPHERICAL_SURFACE"), (.03, "TOROIDAL_SURFACE")])
def test_two_authored_meridians_prove_rotational_patch(tmp_path, major, kind):
    import math
    from formats.skp import apply_payload

    r = .01
    u0, u1, v0, v1 = 0., .35, .2, .5
    def point(u, v):
        rho = major + r*math.cos(v)
        return (rho*math.cos(u), rho*math.sin(u), r*math.sin(v))
    a, b, c, d = [point(u,v) for u,v in [(u0,v0),(u1,v0),(u1,v1),(u0,v1)]]
    arcs = []
    for v, p, q in [(v0,a,b),(v1,d,c)]:
        arcs.append(([(0,0,r*math.sin(v)),(0,0,1),(1,0,0)], major+r*math.cos(v), u0,u1,[(p,q)]))
    for u, p, q in [(u0,a,d),(u1,b,c)]:
        x,y = math.cos(u),math.sin(u)
        arcs.append(([(major*x,major*y,0),(y,-x,0),(x,y,0)],r,v0,v1,[(p,q)]))
    group = {"faces":[([a,b,c,d],[],{})], "curves":[row[-1] for row in arcs],
             "source_arcs":arcs, "soft_edges":[(a,d),(b,c)]}
    scene = Scene()
    apply_payload(scene, {"backend":"openskp", "groups":[group]})
    mesh = scene.groups[0].mesh
    assert mesh._store.exact_source_strip(next(iter(mesh.faces)).key)
    path = tmp_path / "patch.step"
    step.save_step(scene,path)
    assert kind in path.read_text()
    assert step.prepare_step(path)["exact"] == 1


def test_real_partial_report_names_source_record_and_preserves_scene(tmp_path):
    src = Path(__file__).parent / "fixtures" / "step" / "mixed-parts.step"
    text = src.read_text()
    # Invert one face without changing its neighbours' edge uses.
    import re
    face = re.search(r"(#[0-9]+ = ADVANCED_FACE\([^\n]+),\.T\.\);", text)
    assert face
    path = tmp_path / "damaged-part.step"
    # Reverse an edge occurrence to make the closed shell inconsistent.
    path.write_text(re.sub(r"(ORIENTED_EDGE\([^\n]+),\.T\.\);", r"\1,.F.);", text, count=1))
    doc = step.prepare_step(path)
    assert (doc["exact"], doc["faceted"], doc["rejected"]) == (0,1,1)
    assert "BREP_WITH_VOIDS #" in "\n".join(doc["problems"])
    scene = Scene()
    with pytest.raises(ValueError,match="confirmation"):
        step.apply_step(scene,doc)
    step.apply_step(scene,doc,accept_partial=True)
    assert len(scene.groups)==1
