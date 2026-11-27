# SPDX-License-Identifier: GPL-3.0-or-later
"""Live History and format-2 compatibility through the public protocol."""
import json
import zipfile

import pytest

from built_core import Matrix4, Vec3
from built_core.group import Group
from built_core.history import History
from built_core.protocol import commands, queries
from built_core.scene import Scene
from formats import igz
from tests.test_persistent_ids import all_ids, face_id, manifest


def setup_groups(n):
    scene = Scene()
    groups = [Group(name=f"Part {i}") for i in range(n)]
    for group in groups:
        group.xform = Matrix4()
    scene.groups.extend(groups)
    return scene, History(scene), groups


def box(owner, i=1, **params):
    return dict(kind="test_box", uid=f"{i:016x}", owner=owner.uid, **params)


def apply(history, verb, **params):
    reply = commands.run(history, verb, **params)
    assert reply["ok"], reply
    return reply


def test_role_return_and_history():
    scene, h, (g,) = setup_groups(1)
    row = box(g)
    original = g.mesh
    apply(h, "live_set", row=row)
    closed = g.mesh
    ids = all_ids(closed)
    apply(h, "live_set", row=dict(row, open_top=True, size=[2, 3, 4]))
    opened = g.mesh
    assert len(opened.faces) == 5
    assert set(all_ids(opened)["face"]) < set(ids["face"])
    apply(h, "undo")
    assert g.mesh is closed and all_ids(g.mesh) == ids
    apply(h, "redo")
    assert g.mesh is opened and queries.live_rows(scene)[0]["open_top"]
    apply(h, "live_set", row=row)
    assert all_ids(g.mesh) == ids
    for _ in range(3):
        apply(h, "undo")
    assert g.mesh is original and queries.live_rows(scene) == []


def test_order_cycle_move_failure():
    scene, h, (anchor, a, b) = setup_groups(3)
    first = box(a, inputs=[dict(placement=anchor.uid)])
    apply(h, "live_set", row=first)
    face = face_id(a.mesh, a.mesh.faces[0])
    second = box(b, 2, inputs=[dict(path=[a.uid], id=face)])
    apply(h, "live_set", row=second)
    old = [g.mesh for g in (a, b)]
    apply(h, "move_placements", placements=[anchor.uid], delta=[2, 0, 0])
    moved = [g.mesh for g in (a, b)]
    assert all(x is not y for x, y in zip(old, moved))
    assert min(v.position_tuple()[0] for v in b.mesh.vertices) == 2.5
    apply(h, "undo")
    assert [a.mesh, b.mesh] == old
    apply(h, "redo")
    assert [a.mesh, b.mesh] == moved
    revision = h.revision
    result = commands.run(h, "live_set", row=dict(first, inputs=[dict(row=second["uid"])]))
    assert not result["ok"] and h.revision == revision
    apply(h, "live_set", row=dict(first, size=[0, 1, 1]))
    assert a.mesh is moved[0]
    assert queries.live_rows(scene)[0]["state"] == "invalid_size"


def rewrite(path, dest, modify):
    with zipfile.ZipFile(path) as source, zipfile.ZipFile(dest, "w") as output:
        for name in source.namelist():
            data = source.read(name)
            if name == "document.json":
                doc = json.loads(data)
                modify(doc)
                data = json.dumps(doc).encode()
            output.writestr(name, data)


def test_compatible_file_and_stale_choices(tmp_path):
    scene, h, (g,) = setup_groups(1)
    row = box(g)
    apply(h, "live_set", row=row)
    ids = all_ids(g.mesh)
    path = tmp_path / "live.built"
    igz.save_scene(scene, path)
    doc = manifest(path)
    assert not doc["requires"] and doc["ext"]["built.live"]["v"] == 1
    back = Scene()
    igz.load_into(back, path)
    assert queries.live_rows(back)[0]["state"] == "ok"
    assert all_ids(back.groups[0].mesh) == ids

    # An outside reader changes the geometry while preserving optional metadata.
    back.groups[0].mesh.add_edge(Vec3(3, 4, 5), Vec3(4, 5, 6))
    outside = tmp_path / "outside.built"
    igz.save_scene(back, outside)
    stale = Scene()
    igz.load_into(stale, outside)
    assert queries.live_rows(stale)[0]["state"] == "stale"
    g = stale.groups[0]
    found = g.mesh
    h = History(stale)
    apply(h, "live_reevaluate", uid=row["uid"])
    assert len(g.mesh.vertices) == 8
    apply(h, "undo")
    assert g.mesh is found and queries.live_rows(stale)[0]["state"] == "stale"
    apply(h, "live_release", uid=row["uid"])
    assert g.mesh is found and queries.live_rows(stale) == []

    baked = tmp_path / "baked.built"
    rewrite(path, baked, lambda doc: doc.pop("ext"))
    plain = Scene()
    igz.load_into(plain, baked)
    assert queries.live_rows(plain) == []
    assert all_ids(plain.groups[0].mesh) == ids


@pytest.mark.slow
@pytest.mark.perf
def test_dirty_closure_on_synthetic_assembly(tmp_path):
    """One edited input drives ten of a thousand recipes on the ride fixture."""
    import statistics
    import time
    from built_core import element_ids
    from tests.test_big_model_budgets import big_model

    scene = Scene()
    igz.load_into(scene, big_model(tmp_path / "assembly.built"))
    h = History(scene)
    owners = list(scene.groups_by_uid().values())[-1001:-1]
    vertex = scene.mesh.vertices[0]
    element = element_ids.id_text("vertex", scene.mesh._store.id_of("vertex", vertex.key))
    for i, owner in enumerate(owners):
        inputs = [dict(path=[], id=element)] if i < 10 else []
        apply(h, "live_set", row=box(owner, i+1, inputs=inputs))
    before = [g.mesh for g in owners]
    times = []
    for _ in range(9):
        start = time.perf_counter_ns()
        apply(h, "move_vertices", vertices=[element], delta=[0, 0, .01])
        times.append((time.perf_counter_ns()-start)/1e6)
    assert sum(g.mesh is not m for g, m in zip(owners, before)) == 10
    print(f"1,000 rows, 10 dirty, median History commit {statistics.median(times):.3f} ms")



def test_axis_datum_and_slot_binding():
    scene, h, (a, b) = setup_groups(2)
    source = box(a, size=[2, 4, 6])
    apply(h, "live_set", row=source)
    attached = box(b, 2, inputs=[dict(slot="host")],
                   slots={"host": dict(sweep=source["uid"], at="end", point="centre")})
    apply(h, "live_set", row=attached)
    assert min(v.position_tuple() for v in b.mesh.vertices) == (2., 2., 3.)
    apply(h, "live_set", row=dict(source, size=[3, 4, 6]))
    assert min(v.position_tuple() for v in b.mesh.vertices) == (3., 2., 3.)
    previous = b.mesh
    apply(h, "live_set", row=dict(attached, slots={"host": dict(slot="host")}))
    assert b.mesh is previous and queries.live_rows(scene)[1]["state"] == "ref_lost"


def test_reader_drops_placement_marker_and_document_reuse(tmp_path):
    scene, h, (g,) = setup_groups(1)
    row = box(g)
    apply(h, "live_set", row=row)
    g.ext = {}
    path = tmp_path / "no-marker.built"
    igz.save_scene(scene, path)
    igz.load_into(scene, path)
    assert queries.live_rows(scene)[0]["state"] == "stale"
    plain = tmp_path / "plain.built"
    rewrite(path, plain, lambda doc: doc.pop("ext"))
    igz.load_into(scene, plain)
    assert queries.live_rows(scene) == [] and len(scene.groups[0].mesh.faces) == 6


def test_copy_release_and_opaque_inverse():
    from built_core.history import Command

    scene, h, (anchor, g) = setup_groups(2)
    source = box(g, inputs=[dict(placement=anchor.uid)])
    apply(h, "live_set", row=source)
    apply(h, "copy_placements", placements=[g.uid], delta=[4, 0, 0])
    copied = scene.groups[-1]
    original, duplicate = queries.live_rows(scene)
    assert duplicate["uid"] != original["uid"]
    assert copied.ext["built.live"]["row"] == duplicate["uid"]
    apply(h, "live_release", uid=duplicate["uid"])
    baked = copied.mesh

    class MoveAnchor(Command):
        def do(self, scene):
            moved = Matrix4()
            moved.translate(1, 0, 0)
            anchor.xform = moved

        def undo(self, scene):
            anchor.xform = Matrix4()

    before = g.mesh
    h.execute(MoveAnchor())
    after = g.mesh
    assert after is not before
    apply(h, "undo")
    assert g.mesh is before
    apply(h, "redo")
    assert g.mesh is after and copied.mesh is baked
