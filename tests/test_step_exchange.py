# SPDX-License-Identifier: GPL-3.0-or-later
from pathlib import Path

import pytest

from built_core.scene import Scene
from formats import step


@pytest.mark.parametrize("name", ["capped-cylinder", "capped-cylinder-ap203"])
def test_cylinder_seam_and_cap_survive_exchange(tmp_path, name):
    src = Path(__file__).parent / "fixtures" / "step" / (name + ".step")
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
    assert path.read_text().count("= SHAPE_DEFINITION_REPRESENTATION(") == 2
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

@pytest.mark.parametrize("soft, expected", [(True, 1), (False, 2)])
def test_step_preserves_hard_planar_divisions(tmp_path, soft, expected):
    from built_core import Vec3

    scene = Scene()
    a, b, c, d = [Vec3(x, y, 0) for x, y in [(0, 0), (1, 0), (1, 1), (0, 1)]]
    scene.mesh.add_face([a, b, c])
    scene.mesh.add_face([a, c, d])
    edge = next(e for e in scene.mesh.edges if len(e.faces) == 2)
    edge.soft = soft
    path = tmp_path / "planar.step"
    step.save_step(scene, path)
    assert path.read_text().count("= ADVANCED_FACE(") == expected
    assert len(scene.mesh.faces) == 2
    assert step.prepare_step(path)["exact"] == 1

def test_oblique_cylinder_recovery_preserves_ids_and_reopens(tmp_path):
    from built_core import Vec3
    from built_core.history import History
    from built_core.protocol import commands

    scene = Scene()
    mesh = scene.mesh
    for outer, _, _ in ruled_band()["faces"]:
        mesh.add_face([Vec3(x, y, z + (0.25*x if z else -0.1*y)) for x,y,z in outer])
    keys = [f.key for f in mesh.faces]
    history = History(scene)
    params = dict(faces=keys, tolerance=0.001)
    plan = commands.run(history, "recover_exact_geometry", **params)
    assert "mitre" in plan["method"]
    assert plan["input_volume_m3"] < plan["output_volume_m3"]
    assert commands.run(history, "recover_exact_geometry", preview=False, **params)["ok"]
    assert [f.key for f in mesh.faces] == keys
    path = tmp_path / "mitre.step"
    step.save_step(scene, path)
    assert "= ELLIPSE(" in path.read_text()
    assert step.prepare_step(path)["exact"] == 1
    history.undo()
    assert [f.key for f in mesh.faces] == keys

@pytest.mark.parametrize("name,entity", [("extrusion", "SURFACE_OF_LINEAR_EXTRUSION"), ("revolution", "SURFACE_OF_REVOLUTION")])
def test_swept_surface_roundtrip(tmp_path, name, entity):
    path = Path(__file__).parent / "fixtures" / "step" / (name + ".step")
    doc = step.prepare_step(path)
    assert doc["exact"] == 1 and not doc["problems"]
    scene = Scene()
    step.apply_step(scene, doc)
    out = tmp_path / path.name
    step.save_step(scene, out)
    assert "= " + entity + "(" in out.read_text()
    assert step.prepare_step(out)["exact"] == 1

def test_mapped_rigid_instances_share_one_shape(tmp_path):
    src = Path(__file__).parent / "fixtures" / "step" / "capped-cylinder.step"
    text = src.read_text().replace("#73 = SHAPE_DEFINITION_REPRESENTATION(#71,#63);", """#73 = SHAPE_DEFINITION_REPRESENTATION(#71,#80);
#75 = REPRESENTATION_MAP(#9,#63);
#76 = MAPPED_ITEM('prototype',#75,#9);
#77 = MAPPED_ITEM('prototype',#75,#30);
#80 = SHAPE_REPRESENTATION('',(#76,#77),#5);""")
    path = tmp_path / "mapped.step"
    path.write_text(text)
    doc = step.prepare_step(path)
    assert doc["exact"] == 1 and not doc["problems"]
    root, = doc["groups"]
    a, b = root.children
    assert a.mesh is b.mesh
    assert sorted(g.xform.copyDataTo()[11] for g in (a,b)) == [0., 0.025]


def test_multiple_bodies_keep_one_product_after_native_save(tmp_path):
    from built_core import Vec3
    from formats import igz

    scene=Scene()
    for x in (0,2):
        scene.mesh.add_face([Vec3(x,0,0),Vec3(x+1,0,0),Vec3(x,1,0)])
    out=tmp_path / "two-bodies.step"
    step.save_step(scene,out)
    fresh=Scene()
    step.apply_step(fresh,step.prepare_step(out))
    native=tmp_path / "two-bodies.built"
    igz.save_scene(fresh,native)
    reopened=Scene()
    igz.load_into(reopened,native)
    step.save_step(reopened,out)
    assert out.read_text().count("= PRODUCT_DEFINITION(") == 1
    assert out.read_text().count("= SHAPE_DEFINITION_REPRESENTATION(") == 1
    assert step.prepare_step(out)["exact"] == 2


@pytest.mark.parametrize("name", ["assembly_two_boxes", "box_inch", "shelled_box", "cone", "cylinder", "sphere", "torus"])
def test_public_ap214_fixture_reopens(tmp_path, name):
    path = Path(__file__).parent / "fixtures" / "step" / "stepmesh" / (name + ".step")
    data = step.prepare_step(path)
    assert data["exact"] == 1 and not data["problems"]
    scene = Scene()
    step.apply_step(scene, data)
    output = tmp_path / "roundtrip.step"
    step.save_step(scene, output)
    result = step.prepare_step(output)
    assert result["exact"] == 1 and not result["problems"]
    if name == "torus":
        mesh = result["groups"][0].mesh
        assert mesh._store.native_exact and len(mesh.faces) == 1
        uses, = mesh._store.face_boundary_uses(mesh.faces[0].key)
        edges = [int(e) for e in mesh._store.ids("edge")]
        assert len(edges) == 2
        assert sorted(uses) == sorted((e, sense) for e in edges for sense in (False, True))
    if name == "assembly_two_boxes":
        first, second = result["groups"][0].children
        assert first.mesh is second.mesh
        assert first.xform.copyDataTo() != second.xform.copyDataTo()


def test_recovery_multiple_group_meshes_undo_together():
    from built_core import Vec3, command_protocol
    from built_core.group import Group
    from built_core.history import CompoundCommand, History

    scene = Scene()
    history = History(scene)
    commands = []
    for x in (0, 1):
        group = Group(name=str(x))
        scene.groups.append(group)
        mesh = group.mesh
        face = mesh.add_face([Vec3(0,0,0),Vec3(1,0,0),Vec3(x,1,0)])
        report, command = command_protocol._plan_recovery(mesh, [face.key], 0.0001)
        assert report["faces"] == 1
        commands.append(command)
    result = command_protocol.execute(history, CompoundCommand(commands))
    assert result["ok"]
    assert all(g.mesh._store.has_exact_bindings for g in scene.groups)
    history.undo()
    assert all(not g.mesh._store.has_exact_bindings for g in scene.groups)


def test_selected_step_delivery_keeps_the_cylindrical_wall(tmp_path):
    from built_core import deliveries

    path = Path(__file__).parent / "fixtures" / "step" / "capped-cylinder.step"
    scene = Scene()
    step.apply_step(scene, step.prepare_step(path))
    group, = scene.groups
    scene.edit_group = group
    scene.selection = {group.mesh.faces[0]}
    out = tmp_path / "wall.step"
    step.save_step(deliveries.scoped(scene, {"scope": "selection", "format": "step"}), out)
    assert "= CYLINDRICAL_SURFACE(" in out.read_text()
    assert step.prepare_step(out)["exact"] == 1


def test_planar_pcurve_with_wrong_radius_is_rejected(tmp_path):
    source = Path(__file__).parent / "fixtures" / "step" / "stepmesh" / "cylinder.step"
    authored = "#49 = CIRCLE('',#50,5.);"
    broken = tmp_path / "displaced-cap-rim.step"
    text = source.read_text()
    assert text.count(authored) == 1
    broken.write_text(text.replace(authored, "#49 = CIRCLE('',#50,4.);"))
    with pytest.raises(ValueError, match="planar conic pcurve disagrees"):
        step.prepare_step(broken)


def test_background_open_and_insert_keep_source_recovery_counts():
    from built_core import DocumentOpen

    path = Path(__file__).parent / "data" / "skp_arcs_2021.skp"
    command = DocumentOpen(path, insert=True).wait()
    _, _, history, _ = DocumentOpen(path).wait()
    assert command._skp_recovery_counts == history._skp_recovery_counts == (48, 0)


def test_step_import_feedback_has_one_result_and_optional_details(native_host, monkeypatch, tmp_path, caplog):
    import re
    import built_render
    from built_render import chrome_dialogs as dialogs

    win = native_host.window
    monkeypatch.setattr(dialogs, "async_host", lambda _: win)
    monkeypatch.setenv("BUILT_LOG_DIR", str(tmp_path / "logs"))
    fixtures = Path(__file__).parent / "fixtures" / "step"
    assert built_render.import_file(win, fixtures / "stepmesh/torus.step", "step")
    rows = [r for r in dialogs.history(win) if "torus.step" in r.title]
    assert [(r.title, r.tone) for r in rows] == [("Imported torus.step · 1 exact body", "success")]

    damaged = tmp_path / "partial.step"
    data = (fixtures / "mixed-parts.step").read_text()
    damaged.write_text(re.sub(r"(ORIENTED_EDGE\([^\n]+),\.T\.\);", r"\1,.F.);", data, count=1))
    assert not built_render.import_file(win, damaged, "step")
    sheet = dialogs.current_sheet(win)
    assert sheet.default == sheet.cancel == "cancel"
    assert [key for key, _, _, role in sheet.buttons if role == 1] == ["cancel"]
    assert "1 of 2 bodies" in sheet.text and "damaged closed shell" in sheet.text
    assert "ValueError" not in sheet.text and "BREP_WITH_VOIDS #" in caplog.text
    assert "BREP_WITH_VOIDS #187" in (tmp_path / "logs/built-import.log").read_text()
    assert dialogs.sheet_buttons(sheet)[-1] == ("details", "Details", "", 6)
    dialogs.answer(win, "details")
    assert "BREP_WITH_VOIDS #" in dialogs.sheet_buttons(sheet)[-1][2]
    dialogs.answer(win, "details")
    assert not sheet.details_open and dialogs.current_sheet(win) is sheet
    dialogs.answer(win, "import")
    partial, = [r for r in dialogs.history(win) if "partial.step" in r.title]
    assert (partial.tone, partial.action_label) == ("warning", "Show")
    assert partial.title == "Imported partial.step · 1 faceted body · 1 rejected body"
    partial.action()
    assert not dialogs.current_sheet(win).details_open
    dialogs.answer(win, "ok")

    invalid = tmp_path / "unreadable.step"
    invalid.write_bytes(b"not a STEP document")
    assert not built_render.import_file(win, invalid, "step")
    failure, = [r for r in dialogs.history(win) if "unreadable.step" in r.title]
    assert (failure.tone, failure.action_label) == ("warning", "Show")
    failure.action()
    assert "ValueError" not in dialogs.current_sheet(win).text
    dialogs.answer(win, "details")
    assert dialogs.sheet_buttons(dialogs.current_sheet(win))[-1][2]
    dialogs.answer(win, "ok")


def test_fragmented_planar_sheet_shares_support_but_keeps_hard_topology(tmp_path):
    from built_core import Vec3

    scene = Scene()
    for i in range(8):
        scene.mesh.add_face([Vec3(i, 0, 0), Vec3(i+1, 0, 0), Vec3(i+1, 1, 0), Vec3(i, 1, 0)])
    path = tmp_path / "strip.step"
    step.save_step(scene, path)
    text = path.read_text()
    assert text.count("= ADVANCED_FACE(") == 8
    assert text.count("= PLANE(") == 1
    assert text.count("= EDGE_CURVE(") == 25
    assert step.prepare_step(path)["exact"] == 1


@pytest.mark.parametrize("bead", [False, True])
def test_knee_assembly_shares_bolt_definition(tmp_path, bead):
    from tests.test_live_junction import knee_fixture
    scene, _, _ = knee_fixture(bead=bead)
    path = tmp_path / "knee.step"
    step.save_step(scene, path)
    text = path.read_text()
    products = [line for line in text.splitlines() if "= PRODUCT(" in line]
    assert sum("ISO 4017" in line for line in products) == 1
    assert any("End plate" in line for line in products)
    assert any("seam" in line for line in products)
    assert "NEXT_ASSEMBLY_USAGE_OCCURRENCE(" in text
