# SPDX-License-Identifier: GPL-3.0-or-later
"""Sketch rows (general-base §2.2, lane 2): one History step per Done, the
sweeps that use a sketch re-evaluated in that step, open profiles leave
their users as they were, rows travel through save and open, and a profile
drawn in the Sketch workspace sweeps exactly like the Library one."""
import json
import math

import pytest

from built_core.history import History
from built_core.protocol import commands, queries
from built_core.scene import Scene
from formats import igz
from tests.test_live_recipes import manifest
from tests.test_persistent_ids import all_ids


def apply(history, verb, **params):
    reply = commands.run(history, verb, **params)
    assert reply["ok"], reply
    return reply


def rect(w, h, first_id=1):
    p = [[0, 0], [w, 0], [w, h], [0, h]]
    return [dict(id=first_id + i, kind="line", a=p[i], b=p[(i + 1) % 4]) for i in range(4)]


def sketch(curves, uid="5ce7c40000000001", **extra):
    return {"uid": uid, "use": "profile", "name": "Plate", "curves": curves, **extra}


def rows_of(scene, kind):
    return [r for r in queries.live_rows(scene) if r["kind"] == kind]


def member(profile, points=((0, 0, 0), (2, 0, 0))):
    return dict(profile=profile, path=dict(points=[list(p) for p in points]))


def extent(g):
    pos = [v.position_tuple() for v in g.mesh.vertices]
    return [round(max(p[i] for p in pos) - min(p[i] for p in pos), 9) for i in range(3)]


def positions(g):
    return sorted(tuple(round(c, 9) for c in v.position_tuple()) for v in g.mesh.vertices)


def test_done_is_one_step_and_its_sweeps_follow_it():
    scene = Scene()
    h = History(scene)
    s = sketch(rect(200, 100))
    reply = apply(h, "sketch_commit", row=s)
    assert reply["uid"] == s["uid"]
    (row,) = rows_of(scene, "sketch")
    assert row["state"] == "ok" and "owner" not in row and row["curves"][0]["kind"] == "line"
    apply(h, "create_sweeps", sweeps=[member(dict(sketch=s["uid"]))])
    (g,) = scene.groups
    assert extent(g) == [2.0, 0.2, 0.1]
    ids = all_ids(g.mesh)
    before = g.mesh
    revision = h.revision
    apply(h, "sketch_commit", row=sketch(rect(300, 100)))
    assert h.revision == revision + 1
    assert extent(g) == [2.0, 0.3, 0.1]
    # Generated ids keep their roles across the sketch edit (curve ids are kept).
    assert all_ids(g.mesh) == ids
    apply(h, "undo")
    assert g.mesh is before
    assert rows_of(scene, "sketch")[0]["curves"][1]["a"] == [200, 0]
    apply(h, "redo")
    assert extent(g) == [2.0, 0.3, 0.1]


def test_an_open_sketch_leaves_its_users_as_they_were():
    scene = Scene()
    h = History(scene)
    s = sketch(rect(200, 100))
    apply(h, "sketch_commit", row=s)
    apply(h, "create_sweeps", sweeps=[member(dict(sketch=s["uid"]))])
    (g,) = scene.groups
    good = g.mesh
    apply(h, "sketch_commit", row=sketch(rect(400, 100)[:3]))
    assert rows_of(scene, "sketch")[0]["state"] == "profile_open"
    assert g.mesh is good
    assert rows_of(scene, "sweep")[0]["state"] == "ok"
    profile = queries.sketch_profile(scene, s["uid"])
    assert not profile["ok"]
    assert [p["kind"] for p in profile["problems"]] == ["open_end", "open_end"]
    # Closing it again re-evaluates the member.
    apply(h, "sketch_commit", row=sketch(rect(400, 100)))
    assert g.mesh is not good and extent(g) == [2.0, 0.4, 0.1]


def test_a_crossing_sketch_is_not_a_profile():
    scene = Scene()
    h = History(scene)
    bow = [dict(id=1, kind="line", a=[0, 0], b=[10, 10]), dict(id=2, kind="line", a=[10, 10], b=[10, 0]),
           dict(id=3, kind="line", a=[10, 0], b=[0, 10]), dict(id=4, kind="line", a=[0, 10], b=[0, 0])]
    apply(h, "sketch_commit", row=sketch(bow))
    assert rows_of(scene, "sketch")[0]["state"] == "profile_open"
    kinds = [p["kind"] for p in queries.sketch_profile(scene, "5ce7c40000000001")["problems"]]
    assert "crossing" in kinds


def test_bad_sketches_are_refused_without_a_step():
    scene = Scene()
    h = History(scene)
    revision = h.revision
    for bad in (
        sketch([dict(id=1, kind="line", a=[0, 0], b=[0, 0])]),
        sketch(rect(10, 10) + [dict(id=1, kind="circle", c=[0, 0], r=3)]),
        sketch([dict(id=1, kind="arc", c=[0, 0], r=-1, a0=0, a1=1)]),
    ):
        with pytest.raises(ValueError):
            commands.run(h, "sketch_commit", row=bad)
    assert h.revision == revision and rows_of(scene, "sketch") == []


def test_a_new_sketch_gets_a_uid_and_reports_its_attachment_points():
    scene = Scene()
    h = History(scene)
    s = sketch(rect(130, 200), points=[dict(name="shear_centre", at=[-20, 100])], attach="top_centre")
    del s["uid"]
    uid = apply(h, "sketch_commit", row=s)["uid"]
    assert len(uid) == 16
    p = queries.sketch_profile(scene, uid)
    assert p["ok"] and p["area"] == pytest.approx(26000)
    assert p["attach"]["top_centre"] == [65, 200]
    assert p["attach"]["shear_centre"] == [-20, 100]
    assert p["default"] == "top_centre"


def test_sketch_rows_survive_save_and_open(tmp_path):
    scene = Scene()
    h = History(scene)
    arc = dict(id=3, kind="arc", c=[50, 50], r=50, a0=0, a1=math.pi, segments=24)
    curves = rect(100, 50)
    curves[2] = arc
    s = sketch(curves, plane={"ref": {"path": [], "id": "f:0"}, "frame": "face", "offset": 0},
               dims=[dict(id=9, kind="length", a=dict(curve=1, at="mid"), value=100)],
               extrude=dict(extent="depth", depth=80), next_id=10)
    apply(h, "sketch_commit", row=s)
    apply(h, "create_sweeps", sweeps=[member(dict(sketch=s["uid"]))])
    path = tmp_path / "sketch.built"
    igz.save_scene(scene, path)
    saved = manifest(path)["ext"]["built.live"]["rows"]
    stored = next(r for r in saved if r["kind"] == "sketch")
    assert "size" not in stored and "open_top" not in stored and "owner" not in stored
    assert stored["curves"][2] == arc
    back = Scene()
    igz.load_into(back, path)
    (row,) = rows_of(back, "sketch")
    assert row["state"] == "ok"
    for key in ("plane", "dims", "extrude", "curves", "next_id"):
        assert row[key] == stored[key]
    assert queries.sketch_profile(back, s["uid"])["area"] == pytest.approx(5000 + 1250 * math.pi)
    assert rows_of(back, "sweep")[0]["state"] == "ok"
    # Done on the opened file re-evaluates its member in one step.
    apply(History(back), "sketch_commit", row=dict(s, curves=rect(150, 50)))
    assert extent(back.groups[0]) == [2.0, 0.15, 0.05]


def test_release_removes_the_sketch_and_its_members_keep_their_geometry():
    scene = Scene()
    h = History(scene)
    s = sketch(rect(200, 100))
    apply(h, "sketch_commit", row=s)
    apply(h, "create_sweeps", sweeps=[member(dict(sketch=s["uid"]))])
    (g,) = scene.groups
    kept = positions(g)
    apply(h, "live_release", uid=s["uid"])
    # The member keeps the profile as its own frozen copy (general-base §4.4).
    (copy,) = rows_of(scene, "sketch")
    assert copy["uid"] != s["uid"] and copy["frozen"] is True
    assert positions(g) == kept
    assert rows_of(scene, "sweep")[0]["state"] == "ok"
    apply(h, "undo")
    (row,) = rows_of(scene, "sketch")
    assert row["uid"] == s["uid"] and row["state"] == "ok"


def test_a_drawn_profile_sweeps_exactly_like_the_library_one(tmp_path, monkeypatch):
    """The Sketch workspace's 45 x 145 rectangle, as a model sketch row and as
    a saved Library file, gives the same member as the catalogue's timber."""
    from built_render.wgpu_library import ProfilesProvider
    from built_render import sketch_workspace

    path = ((0, 0, 0), (3, 1, 0.5))
    scene = Scene()
    h = History(scene)
    apply(h, "create_sweeps", sweeps=[member(dict(library="basic/timber"), path)])
    drawn = sketch(rect(45, 145), uid="5ce7c40000000002", name="Timber 45 x 145")
    apply(h, "sketch_commit", row=drawn)
    apply(h, "create_sweeps", sweeps=[member(dict(sketch=drawn["uid"]), path)])
    # Saved to the Library as the workspace writes it, then picked again.
    monkeypatch.setenv("BUILT_SETTINGS_DIR", str(tmp_path))
    directory = sketch_workspace.profiles_dir()
    assert directory == str(ProfilesProvider.saved_directory())
    body = {k: v for k, v in drawn.items() if k not in ("uid",)}
    (tmp_path / "out").mkdir()
    sketch_workspace.save_profile(json.dumps({"format": "built.profile", "v": 1, "name": drawn["name"],
                                              "use": "profile", "sketch": body}))
    sketch_workspace.save_profile(json.dumps({"format": "built.profile", "v": 1, "name": "Notch",
                                              "use": "section", "sketch": dict(body, use="section")}))
    # The Sweep presets picker lists these after the presets: the saved
    # drawing, and the model's own sketch; never a cutter section.
    from types import SimpleNamespace
    choices = ProfilesProvider().items(SimpleNamespace(scene=scene))
    # The saved drawing: from the older folder, or My library once migrated.
    local, = [i for i in choices if i["source"] in ("local", "mine")]
    assert local["name"] == "Timber 45 x 145"
    project, = [i for i in choices if i["source"] == "model"]
    assert project["key"] == "sketch:" + drawn["uid"]
    assert not [i for i in choices if i["name"] == "Notch"]
    apply(h, "create_sweeps", sweeps=[member(dict(drawing=local["sketch"]), path)])
    library, from_row, from_file = [g for g in scene.groups]
    assert positions(library) == positions(from_row) == positions(from_file)
    assert len(library.mesh.faces) == len(from_row.mesh.faces) == len(from_file.mesh.faces)
    assert {r["state"] for r in queries.live_rows(scene)} == {"ok"}
