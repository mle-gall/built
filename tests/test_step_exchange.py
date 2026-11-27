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
