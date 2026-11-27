# SPDX-License-Identifier: GPL-3.0-or-later
"""Live History and format-2 compatibility through the public protocol."""
import json
import zipfile

from built_core import Matrix4
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
    back.groups[0].mesh.add_vertex((3, 4, 5))
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
