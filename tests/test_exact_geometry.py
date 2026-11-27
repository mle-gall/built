# SPDX-License-Identifier: GPL-3.0-or-later
"""Exact support evaluation and the whole-interval trim admission boundary."""
import json
import math
from collections import Counter

import pytest

from built_core.command_protocol import ExactGeometry


def frame(z=0):
    return dict(origin=[0, 0, z], axis=[0, 0, 1], reference=[1, 0, 0])


def cylinder():
    t = math.tau
    curves = [dict(type="circle", frame=frame(z), radius=1) for z in (0, 2)]
    curves.append(dict(type="line", origin=[1, 0, 0], direction=[0, 0, 1]))
    edges = [
        dict(id=10+k, curve=k, vertices=ends, interval=[0, stop])
        for k, (ends, stop) in enumerate([([0, 0], t), ([1, 1], t), ([0, 1], 2)])
    ]
    vertices = [[0, 0], [t, 0], [t, 2], [0, 2]]
    ring = [dict(edge=edge, reversed=reverse, tolerance=0.005,
                 uv=[vertices[i], vertices[(i+1) % 4]])
            for i, (edge, reverse) in enumerate([(10, False), (12, False),
                                                  (11, True), (12, True)])]
    return dict(curves=curves, surfaces=[dict(type="cylinder", frame=frame(), radius=1)],
                topology=dict(tolerance=0.01, vertices={0: [1, 0, 0], 1: [1, 0, 2]},
                              edges=edges, faces=[dict(id=20, surface=0, reversed=False,
                                                       loops=[ring])]))


def test_closed_cylinder_edge_and_two_seam_uses():
    geometry = ExactGeometry(json.dumps(cylinder()))
    mesh = json.loads(geometry.tessellate())
    assert set(mesh["face_ids"]) == {20}
    incidence = Counter()
    for a, b, c in mesh["triangles"]:
        for i, j in [(a, b), (b, c), (c, a)]:
            incidence[tuple(sorted([i, j]))] += 1
        tri = [mesh["points"][i] for i in (a, b, c)]
        center = [sum(p[k] for p in tri)/3 for k in range(3)]
        assert abs(math.hypot(*center[:2]) - 1) <= 0.01
    for (a, b), count in incidence.items():
        if count == 1:
            z = mesh["points"][a][2]
            assert mesh["points"][b][2] == z and z in (0, 2)
        else:
            assert count == 2


def test_reject_coincident_endpoints_with_wrong_period_count():
    rows = cylinder()
    rows["topology"]["edges"][0]["interval"][1] *= 2
    with pytest.raises(ValueError, match="pcurve disagree"):
        ExactGeometry(json.dumps(rows))


def test_spline_quarter_circle_and_generator():
    axis = dict(degree=2, knots=[-1, 1], multiplicities=[3, 3], periodic=False)
    spline = dict(type="spline", axis=axis,
                  controls=[[1, 0, 0, 1], [1, 1, 0, math.sqrt(0.5)], [0, 1, 0, 1]])
    extrusion = dict(type="extrusion", generator=0, vector=[0, 0, 2])
    pool = dict(curves=[spline], surfaces=[extrusion])
    geometry = ExactGeometry(json.dumps(pool))
    assert geometry.surface_point(0, 0, 3) == pytest.approx([math.sqrt(0.5)] * 2 + [6])
    with pytest.raises(ValueError, match="outside"):
        geometry.curve_point(0, -2)
    spline["controls"][0][-1] = 0
    with pytest.raises(ValueError, match="weight"):
        ExactGeometry(json.dumps(pool))


def test_materialized_patch_is_one_history_step():
    from built_core.history import History
    from built_core.protocol import commands
    from built_core.scene import Scene

    scene = Scene()
    history = History(scene)
    result = commands.run(history, "materialize_exact_geometry", geometry=cylinder())
    assert result["ok"]
    ids = list(scene.mesh._store.ids("face"))
    assert len(ids) > 10
    history.undo()
    assert len(scene.mesh.faces) == 0
    history.redo()
    assert list(scene.mesh._store.ids("face")) == ids


def test_exact_plane_roundtrip_undo_and_critical_rows(tmp_path):
    import zipfile
    from built_core.history import History
    from built_core.protocol import commands
    from built_core.scene import Scene
    from formats import igz

    scene = Scene()
    h = History(scene)
    commands.run(h, "add_face", outer=[[0, 0, 0], [2, 0, 0], [0, 2, 0]])
    key = scene.mesh.faces[0].key
    assert commands.run(h, "bind_exact_plane", face=key, **frame())["ok"]
    path = tmp_path / "plane.built"

    def write(model):
        igz.save_scene(model, path)
        with zipfile.ZipFile(path) as z:
            return json.loads(z.read("document.json"))

    def check_bound(manifest):
        assert manifest["requires"] == ["built.exact_geometry.v1"]
        pool = manifest["ext"]["built.exact_geometry"]["surfaces"]
        assert pool == [{"type": "plane", "frame": frame()}]
        arrays = manifest["scene"]["geom"]["arrays"]
        assert arrays["face_surface_id"][:2] == ["<u8", [1]]
        assert arrays["face_surface_ref"][:2] == ["<u4", [1]]

    check_bound(write(scene))
    h.undo()
    assert write(scene)["requires"] == []
    h.redo()
    check_bound(write(scene))
    reopened = Scene()
    igz.load_into(reopened, path)
    check_bound(write(reopened))


def test_damaged_surface_reference_is_rejected(tmp_path):
    import struct
    import zipfile
    from built_core.history import History
    from built_core.protocol import commands
    from built_core.scene import Scene
    from formats import igz

    scene = Scene()
    h = History(scene)
    commands.run(h, "add_face", outer=[[0, 0, 0], [2, 0, 0], [0, 2, 0]])
    commands.run(h, "bind_exact_plane", face=scene.mesh.faces[0].key, **frame())
    path = tmp_path / "good.built"
    igz.save_scene(scene, path)
    bad = tmp_path / "bad.built"
    with zipfile.ZipFile(path) as src, zipfile.ZipFile(bad, "w") as dst:
        geom = json.loads(src.read("document.json"))["scene"]["geom"]
        for info in src.infolist():
            data = bytearray(src.read(info))
            if info.filename == geom["member"]:
                struct.pack_into("<I", data, geom["arrays"]["face_surface_ref"][2], 99)
            dst.writestr(info, data)
    target = Scene()
    target_history = History(target)
    commands.run(target_history, "add_edge", a=[9, 0, 0], b=[10, 0, 0])
    prior = target.mesh.capture_state()
    with pytest.raises(ValueError, match="missing plane pool row"):
        igz.load_into(target, bad)
    assert target.mesh.capture_state() == prior


def test_spherical_poles_have_single_model_vertices():
    half = math.pi / 2
    # The meridian runs from the south pole to the north through +X.
    meridian = dict(type="circle", radius=1, frame=dict(
        origin=[0, 0, 0], reference=[1, 0, 0], axis=[0, -1, 0]))
    uv = [(0, -half), (math.tau, -half), (math.tau, half), (0, half)]
    ring = [dict(**binding, uv=[uv[i], uv[(i+1) % 4]], tolerance=0.01)
            for i, binding in enumerate([dict(pole=0), dict(edge=0),
                                          dict(pole=1), dict(edge=0, reversed=True)])]
    model = dict(curves=[meridian], surfaces=[dict(type="sphere", radius=1, frame=frame())],
                 topology=dict(tolerance=0.04, vertices={0: [0, 0, -1], 1: [0, 0, 1]},
                               edges=[dict(id=0, curve=0, vertices=[0, 1], interval=[-half, half])],
                               faces=[dict(id=0, surface=0, reversed=False, loops=[ring])]))
    mesh = json.loads(ExactGeometry(json.dumps(model)).tessellate())
    uses = Counter(tuple(sorted(e)) for a, b, c in mesh["triangles"]
                   for e in [(a, b), (b, c), (c, a)])
    assert set(uses.values()) == {2}


def test_plane_clipboard_archive_and_cross_document_paste():
    from types import SimpleNamespace
    from built_core import Vec3
    from built_core.history import History
    from built_core.scene import Scene
    from formats import clip_payload
    from tools.paste import PasteTool

    rows = list(frame().values())
    square = [Vec3(0, 0, 0), Vec3(2, 0, 0), Vec3(2, 2, 0), Vec3(0, 2, 0)]
    payload = dict(faces=[(square, [], {})], edges=[], groups=[], ref=Vec3(),
                   face_planes={"0": rows})
    clip = clip_payload.decode(clip_payload.encode(payload))
    assert clip["face_planes"]["0"] == rows
    scene = Scene()
    h = History(scene)
    vp = SimpleNamespace(scene=scene, history=h, clipboard=clip, update=lambda: None)
    assert PasteTool.in_place(vp)
    assert scene.mesh._store.exact_plane(scene.mesh.faces[0].key) == rows
    h.undo()
    assert len(scene.mesh.faces) == 0
    h.redo()
    assert scene.mesh._store.exact_plane(scene.mesh.faces[0].key) == rows


def test_closed_cylinder_caps_share_every_rim_segment():
    rows = cylinder()
    for i, z in enumerate((0, 2)):
        rows["surfaces"].append(dict(type="plane", frame=frame(z)))
        cap = dict(edge=10+i, projected=True, reversed=i == 0, uv=[[1, 0], [1, 0]],
                   tolerance=0.005)
        rows["topology"]["faces"].append(dict(id=21+i, surface=i+1, reversed=i == 0,
                                              loops=[[cap]]))
    mesh = json.loads(ExactGeometry(json.dumps(rows)).tessellate())
    directed = Counter((i, j) for a, b, c in mesh["triangles"]
                       for i, j in ((a, b), (b, c), (c, a)))
    assert set(mesh["face_ids"]) == {20, 21, 22}
    assert all(n == directed[b, a] == 1 for (a, b), n in directed.items())


def test_boundary_returns_distinct_seam_branches():
    g = ExactGeometry(json.dumps(cylinder()))
    ring, = json.loads(g.face_boundary(20))
    assert [u["edge"] for u in ring] == [10, 12, 11, 12]
    assert ring[1]["reversed"] != ring[3]["reversed"]
    assert ring[1]["uv"][0][0] == math.tau and ring[3]["uv"][0][0] == 0


def test_failed_materialization_restores_existing_mesh():
    from built_core.history import History
    from built_core.protocol import commands
    from built_core.scene import Scene

    scene = Scene()
    h = History(scene)
    commands.run(h, "add_face", outer=[[10, 10, 0], [11, 10, 0], [10, 11, 0]])
    before = scene.mesh.capture_state()
    rows = cylinder()
    scale = 1e-10
    rows["surfaces"][0]["radius"] *= scale
    for c in rows["curves"][:2]:
        c["radius"] *= scale
    rows["curves"][2]["origin"][0] *= scale
    for v in rows["topology"]["vertices"].values():
        v[0] *= scale
    result = commands.run(h, "materialize_exact_geometry", geometry=rows)
    assert not result["ok"] and "weld collapses" in str(result["error"])
    assert scene.mesh.capture_state() == before


def test_group_only_clipboard_serializes_exact_definition():
    from built_core import Matrix4, Vec3
    from built_core.group import Group
    from built_core.mesh import Mesh
    from formats import clip_payload

    mesh = Mesh()
    f = mesh.add_face([Vec3(0, 0, 0), Vec3(2, 0, 0), Vec3(0, 2, 0)])
    rows = list(frame().values())
    mesh._store.bind_exact_plane(f.key, rows)
    placed = Group(mesh)
    placed.xform = Matrix4()
    clip = clip_payload.decode(clip_payload.encode(dict(groups=[placed], ref=Vec3())))
    copy, = clip["groups"]
    assert copy.mesh._store.exact_plane(copy.mesh.faces[0].key) == rows


@pytest.mark.parametrize(("surface", "uv", "expected"), [
    (dict(type="cylinder", radius=3), (math.pi / 2, 1), (0, 3, 1)),
    (dict(type="cone", radius=1, half_angle=math.pi / 6), (0, 2), (2, 0, math.sqrt(3))),
    (dict(type="sphere", radius=2), (math.pi / 2, math.pi / 6), (0, math.sqrt(3), 1)),
    (dict(type="torus", major=3, minor=1), (math.pi / 2, math.pi), (0, 2, 0)),
    (dict(type="plane"), (3, 8), (3, 8, 0)),
])
def test_elementary_support_parameters(surface, uv, expected):
    pool = ExactGeometry(json.dumps(dict(curves=[], surfaces=[dict(surface, frame=frame())])))
    assert pool.surface_point(0, *uv) == pytest.approx(expected)


def test_rational_tensor_and_revolution():
    axis = dict(degree=1, knots=[0, 1], multiplicities=[2, 2], periodic=False)
    net = dict(type="spline", u=axis, v=axis, rows=2, cols=2,
               controls=[[0, 0, 0, 1], [0, 1, 0, 1], [1, 0, 0, 1], [1, 1, 1, 2]])
    line = dict(type="line", origin=[2, 0, 0], direction=[0, 0, 1])
    rev = dict(type="revolution", generator=0, origin=[0, 0, 0], axis=[0, 0, 1])
    pool = ExactGeometry(json.dumps(dict(curves=[line], surfaces=[net, rev])))
    assert pool.surface_point(0, 0.5, 0.5) == pytest.approx([0.6, 0.6, 0.4])
    assert pool.surface_point(1, 3, math.pi/2) == pytest.approx([0, 2, 3])


def test_planar_annulus_has_no_triangles_across_the_hole():
    rows = dict(curves=[], surfaces=[dict(type="plane", frame=frame())])
    loops, edges, vertices = [], [], {}
    for i, (major, minor) in enumerate(((4, 3), (1, 0.5))):
        rows["curves"].append(dict(type="ellipse", major=major, minor=minor, frame=frame()))
        vertices[i] = [major, 0, 0]
        edges.append(dict(id=i, curve=i, vertices=[i, i], interval=[0, math.tau]))
        loops.append([dict(edge=i, projected=True, reversed=bool(i),
                           uv=[[major, 0], [major, 0]], tolerance=0.01)])
    rows["topology"] = dict(tolerance=0.02, vertices=vertices, edges=edges,
                            faces=[dict(id=1, surface=0, reversed=False, loops=loops)])
    mesh = json.loads(ExactGeometry(json.dumps(rows)).tessellate())
    area = 0
    for tri in mesh["triangles"]:
        a, b, c = [mesh["points"][i] for i in tri]
        x, y = [(a[k]+b[k]+c[k])/3 for k in (0, 1)]
        assert x*x+4*y*y >= 0.98
        area += ((b[0]-a[0])*(c[1]-a[1])-(b[1]-a[1])*(c[0]-a[0]))/2
    assert area == pytest.approx(math.pi*(12-0.5), rel=0.005)


def test_exact_plane_lost_on_partial_nonplanar_edit_and_restored_by_undo():
    from built_core.history import History
    from built_core.protocol import commands
    from built_core.scene import Scene

    scene = Scene()
    h = History(scene)
    commands.run(h, "add_face", outer=[[0, 0, 0], [2, 0, 0], [2, 2, 0], [0, 2, 0]])
    key = scene.mesh.faces[0].key
    assert commands.run(h, "bind_exact_plane", face=key, **frame())["ok"]
    v = scene.mesh.vertices[0].key
    assert commands.run(h, "move_vertices", vertices=[v], delta=[0, 0, 1])["ok"]
    assert all(scene.mesh._store.exact_plane(f.key) is None for f in scene.mesh.faces)
    h.undo()
    assert scene.mesh._store.exact_plane(key) == list(frame().values())


def test_angular_tolerance_refines_shared_logical_edge_samples():
    rows = cylinder()
    rows["topology"]["tolerance"] = 0.5
    rough = json.loads(ExactGeometry(json.dumps(rows)).tessellate())
    rows["topology"]["normal_tolerance"] = 0.07
    fine = json.loads(ExactGeometry(json.dumps(rows)).tessellate())
    assert len(fine["edge_samples"]["10"]) > len(rough["edge_samples"]["10"])
    edge = fine["edge_samples"]["10"]
    assert edge[0] == edge[-1]
    triangles = {tuple(sorted(e)) for a, b, c in fine["triangles"] for e in ((a, b), (b, c), (c, a))}
    assert all(tuple(sorted(e)) in triangles for e in zip(edge, edge[1:]))


def test_mirrored_nonuniform_instance_preserves_definition_plane():
    from built_core import Vec3
    from built_core.mesh import Mesh

    mesh = Mesh()
    face = mesh.add_face([Vec3(0, 0, 0), Vec3(2, 0, 0), Vec3(0, 2, 0)])
    rows = list(frame().values())
    store = mesh._store
    store.bind_exact_plane(face.key, rows)
    matrix = [-2, 0, 0, 0, 1, 3, 0, 0, 0, 0, 1, 0, 4, 5, 6, 1]
    placed = store._in_instance_frame(matrix, [0, 0, 0])
    assert placed.exact_plane(face.key) == [[4, 5, 6], [0, 0, -1], [-1, 0, 0]]
    assert store.exact_plane(face.key) == rows


def test_faceted_sibling_member_survives_an_exact_document_rewrite(tmp_path):
    import zipfile
    from built_core import Matrix4, Vec3
    from built_core.group import Group
    from built_core.scene import Scene
    from formats import igz

    scene = Scene()
    for z in (0, 3):
        g = Group(name=f"Panel {z}")
        g.xform = Matrix4()
        face = g.mesh.add_face([Vec3(0, 0, z), Vec3(2, 0, z), Vec3(0, 2, z)])
        if z == 0:
            g.mesh._store.bind_exact_plane(face.key, list(frame().values()))
        scene.groups.append(g)
    first, second = [tmp_path / (n + ".built") for n in ("before", "after")]
    igz.save_scene(scene, first)
    back = Scene()
    igz.load_into(back, first)
    igz.save_scene(back, second)

    def faceted_member(path):
        with zipfile.ZipFile(path) as z:
            d = json.loads(z.read("document.json"))
            blocks = [p["geom"] for p in d["scene"]["protos"]]
            ordinary, = [b for b in blocks if "face_surface_ref" not in b["arrays"]]
            return ordinary["arrays"], z.read(ordinary["member"])

    assert faceted_member(first) == faceted_member(second)


@pytest.mark.parametrize("operation", ["reverse_faces", "erase", "erase_stroke", "divide_edges",
                                        "move_vertices", "rotate_selection", "scale_selection",
                                        "push_pull"])
def test_cylinder_facets_remain_valid_after_tool_and_undo(operation):
    from built_core.history import History
    from built_core.protocol import commands
    from built_core.scene import Scene

    scene = Scene()
    h = History(scene)
    assert commands.run(h, "materialize_exact_geometry", geometry=cylinder())["ok"]
    mesh = scene.mesh
    before = mesh.capture_state()
    face, edge = mesh.faces[0].key, mesh.edges[0].key
    vertices = [v.key for v in mesh.vertices]
    args = {
        "reverse_faces": dict(faces=[face]),
        "erase": dict(faces=[face]),
        "erase_stroke": dict(edges=[edge], hide=True),
        "divide_edges": dict(edges=[edge], n=2),
        "move_vertices": dict(vertices=vertices, delta=[1, 2, 3]),
        "rotate_selection": dict(vertices=vertices, axis=[0, 0, 1], center=[0, 0, 0], angles=[40]),
        "scale_selection": dict(vertices=vertices, anchor=[0, 0, 0], factors=[1, 2, 1]),
        "push_pull": dict(face=face, distance=0.1),
    }[operation]
    result = commands.run(h, operation, **args)
    assert result["ok"], result["error"]
    mesh._store.check()
    h.undo()
    assert mesh.capture_state() == before
    mesh._store.check()


def test_materialization_refuses_welding_two_disjoint_shells_together():
    from copy import deepcopy
    from built_core.history import History
    from built_core.protocol import commands
    from built_core.scene import Scene

    scene = Scene()
    h = History(scene)
    shape = cylinder()
    topo = shape["topology"]
    face = deepcopy(topo["faces"][0])
    face["id"] += 100
    for use in face["loops"][0]:
        use["edge"] += 100
    topo["faces"].append(face)
    for e in list(topo["edges"]):
        topo["edges"].append(dict(e, id=e["id"]+100, vertices=[v+100 for v in e["vertices"]]))
    topo["vertices"].update({100+k: v for k, v in list(topo["vertices"].items())})
    before = scene.mesh.capture_state()
    result = commands.run(h, "materialize_exact_geometry", geometry=shape)
    assert not result["ok"] and "distinct model samples" in str(result["error"])
    assert scene.mesh.capture_state() == before


def test_link_reload_observes_a_changed_support_in_an_unchanged_block(tmp_path):
    from built_core import Vec3
    from built_core.group import Group
    from built_core.link_meshes import LinkMeshes
    from built_core.scene import Scene
    from formats import igz

    scene = Scene()
    face = scene.mesh.add_face([Vec3(), Vec3(2, 0, 0), Vec3(0, 2, 0)])
    rows = list(frame().values())
    scene.mesh._store.bind_exact_plane(face.key, rows)
    path = tmp_path / "sheet.built"
    igz.save_scene(scene, path)
    first = Scene()
    igz.load_into(first, path, link_meshes=LinkMeshes(path, {}))
    previous = Group(first.mesh)
    rows[0] = [1, 0, 0]
    scene.mesh._store.bind_exact_plane(face.key, rows)
    igz.save_scene(scene, path)
    second = Scene()
    reader = LinkMeshes(path, {}, previous)
    igz.load_into(second, path, link_meshes=reader)
    assert second.mesh._store.exact_plane(second.mesh.faces[0].key) == rows
