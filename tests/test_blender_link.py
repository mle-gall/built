# SPDX-License-Identifier: GPL-3.0-or-later
"""The Blender link, Built's side: the loopback socket and its pairing
token, chunked frames, what the change feed makes Built send again, the
GLB chunks, applying each kind of Blender change as one undo step,
conflicts, and the Render drawer's loop driven by the add-on's controller
(Blender stood in for by a fake host)."""
from __future__ import annotations

import json
import math
import struct
import sys
import time
import types
from pathlib import Path

import numpy as np
import pytest
from tests.shell_isolation import restore_shell_patches  # noqa: F401

from built_core import Matrix4, Vec3 as V
from built_core import mesh as M
from built_core import render_blender as rb
from built_core.group import Group
from built_core.history import History, InsertGroupCommand, SetPluginDataCommand
from built_core.protocol import commands as C
from built_core.scene import Scene

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "integrations" / "blender"))

from built_link import client as addon_client  # noqa: E402
from built_link import controller as addon_controller  # noqa: E402
from built_link import wire as addon_wire  # noqa: E402

PLUGIN = "render_blender"


def _wait(pred, timeout=5.0, step=None):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if step is not None:
            step()
        got = pred()
        if got:
            return got
        time.sleep(0.01)
    raise AssertionError("condition not met in time")


def _scene():
    sc = Scene()
    h = History(sc)
    sc.mesh.add_face([V(0, 0, 0), V(2, 0, 0), V(2, 2, 0), V(0, 2, 0)])
    g = M.Mesh()
    g.add_face([V(0, 0, 0), V(1, 0, 0), V(1, 1, 0)])
    seat = Group(g, name="Seat")
    m = Matrix4()
    m.translate(V(2, 0, 0))
    seat.xform = m
    h.execute(InsertGroupCommand(seat))
    leg_mesh = M.Mesh()
    leg_mesh.add_face([V(5, 0, 0), V(6, 0, 0), V(6, 1, 0)])
    leg = Group(leg_mesh, name="Leg")
    h.execute(InsertGroupCommand(leg))
    return sc, h, seat, leg


def _matrix(g):
    return np.array(g.xform.data()).reshape(4, 4).T if g.xform is not None else np.eye(4)


def _png(w, h, rgb):
    import zlib
    raw = b"".join(b"\x00" + bytes(rgb) * w for _ in range(h))

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))


def _flat(m):
    return [float(v) for v in np.asarray(m).T.flatten()]


# ---------------------------------------------------------------- socket and frames

def test_python_and_rust_frames_are_the_same_format():
    big = bytes(range(256)) * 5000
    data = rb.link_encode({"type": "chunk", "uid": "a"}, big, msg=3, chunk=4096)
    (header, payload), = addon_wire.Assembler().feed(data)
    assert header == {"type": "chunk", "uid": "a"} and payload == big
    back = rb.link_decode(addon_wire.encode({"type": "changes", "items": [1]}, big, msg=9, chunk=4096))
    assert back[0]["items"] == [1] and back[0]["_payload"] == big


def _client(port, token, **hello):
    c = addon_client.Client()
    c.start(port, token, {"file": "bench.blend", **hello})
    return c


def _events(server, kind, timeout=5.0):
    seen = []

    def got():
        seen.extend(server.poll())
        return [e for e in seen if e["type"] == kind]
    return _wait(got, timeout)


def test_the_pairing_token_links_and_a_wrong_one_is_refused():
    server = rb.LinkServer()
    port = server.start(0)
    assert server.pairing == f"built:{port}:{server.token}"
    assert addon_wire.pairing(server.pairing) == (port, server.token)
    bad = _client(port, "WRONG-TOKEN")
    _wait(lambda: bad.state == "refused")
    assert "pairing code" in bad.message
    assert _events(server, "_refused")[0]["reason"] == "token"
    assert not server.linked
    good = _client(port, server.token)
    _wait(lambda: good.state == "linked")
    linked = _events(server, "_linked")[0]
    assert linked["file"] == "bench.blend" and "token" not in linked
    good.stop()
    server.stop()


def test_a_blend_linked_to_another_model_is_refused_unless_relinked():
    server = rb.LinkServer()
    port = server.start(0)
    server.set_document("doc-1", "bench.igz")
    other = _client(port, server.token, document="doc-2")
    _wait(lambda: other.state == "refused")
    assert "another Built model" in other.message
    other.start(port, server.token, {"document": "doc-2"}, relink=True)
    _wait(lambda: other.state == "linked")
    assert other.welcome["document"] == "doc-1" and other.welcome["name"] == "bench.igz"
    other.stop()
    server.stop()


def test_large_messages_arrive_whole_and_blender_closing_is_seen():
    server = rb.LinkServer()
    port = server.start(0)
    c = _client(port, server.token)
    _wait(lambda: c.state == "linked")
    _events(server, "_linked")
    big = np.random.default_rng(1).integers(0, 255, 5 * rb.LINK_CHUNK + 17, dtype=np.uint8).tobytes()
    assert server.send({"type": "chunk", "items": [{"uid": "u"}]}, big)
    (header, payload), = _wait(lambda: [m for m in c.drain() if m[0]["type"] == "chunk"])
    assert payload == big and header["items"] == [{"uid": "u"}]
    assert c.send({"type": "changes", "items": []}, b"x" * 10)
    msg = _events(server, "changes")[0]
    assert msg["_payload"] == b"x" * 10
    c.stop()                                    # Blender quits
    _events(server, "_closed")
    assert not server.linked
    again = _client(port, server.token)         # and comes back
    _wait(lambda: again.state == "linked")
    again.stop()
    server.stop()


def test_the_add_on_retries_until_built_listens():
    server = rb.LinkServer()
    port = server.start(0)
    token = server.token
    server.stop()
    c = _client(port, token)
    _wait(lambda: c.state == "waiting")
    server2 = rb.LinkServer(token)
    server2.start(port)
    _wait(lambda: c.state == "linked", timeout=8)
    c.stop()
    server2.stop()


# ---------------------------------------------------------------- what Built sends

def test_the_tracker_sends_everything_once_then_what_the_feed_names():
    sc, h, seat, leg = _scene()
    t = rb.LinkTracker()
    t.reset({})
    first = t.plan(sc)
    assert sorted(a["uid"] for a in first) == sorted([seat.uid, leg.uid, "loose"])
    assert all(a["op"] == "chunk" for a in first) and not t.pending()
    rev = h.revision

    def observe():
        nonlocal rev
        got = h.feed.since(rev)
        for step in got["steps"]:
            t.observe(step)
        rev = h.revision
    # Moving a placement with a matrix: its matrix travels alone.
    assert C.run(h, "move_placements", placements=[seat.uid], delta=[1, 0, 0])["ok"]
    observe()
    assert [(a["op"], a["uid"]) for a in t.plan(sc)] == [("transform", seat.uid)]
    # Renaming changes what Blender shows: the chunk goes again.
    C.run(h, "rename_placement", placement=leg.uid, name="Front leg")
    observe()
    assert [(a["op"], a["uid"]) for a in t.plan(sc)] == [("chunk", leg.uid)]
    # Hidden or deleted: removed from Blender.
    C.run(h, "hide", placements=[leg.uid])
    observe()
    assert [(a["op"], a["uid"]) for a in t.plan(sc)] == [("remove", leg.uid)]
    C.run(h, "delete_placements", placements=[seat.uid])
    observe()
    assert ("remove", seat.uid) in [(a["op"], a["uid"]) for a in t.plan(sc)]
    # A reconnecting Blender that already holds the same chunk gets nothing.
    t2 = rb.LinkTracker()
    t2.reset({})
    plan = t2.plan(sc)
    t3 = rb.LinkTracker()
    t3.reset({a["uid"]: a["fp"] for a in plan if a["op"] == "chunk"})
    assert [a["op"] for a in t3.plan(sc)] == []


def test_geometry_edits_resend_only_the_chunk_they_touch():
    sc, h, seat, leg = _scene()
    t = rb.LinkTracker()
    t.reset({})
    t.plan(sc)
    rev = h.revision

    def observe():
        nonlocal rev
        for step in h.feed.since(rev)["steps"]:
            t.observe(step)
        rev = h.revision
    assert C.run(h, "add_face", outer=[[0, 5, 0], [1, 5, 0], [1, 6, 0]])["ok"]
    observe()
    assert [(a["op"], a["uid"]) for a in t.plan(sc)] == [("chunk", "loose")]
    from built_core.materials import Material
    C.run(h, "paint_placements", placements=[leg.uid], paint={"color": [0.6, 0.4, 0.2], "mat": "Oak"},
          material=Material("Oak", color=(0.6, 0.4, 0.2)))
    observe()
    assert [(a["op"], a["uid"]) for a in t.plan(sc)] == [("chunk", leg.uid)]


def _glb_json(path):
    data = Path(path).read_bytes()
    n = struct.unpack("<I", data[12:16])[0]
    return json.loads(data[20:20 + n])


def test_chunks_are_local_glb_with_uids_and_library_material_names(tmp_path):
    sc, h, seat, leg = _scene()
    from built_core.materials import Material
    C.run(h, "paint_placements", placements=[seat.uid], paint={"color": [0.6, 0.4, 0.2], "mat": "Oak"},
          material=Material("Oak", color=(0.6, 0.4, 0.2)))
    out = rb.export_chunks(sc, [seat.uid, "loose", "missing"], tmp_path / "c.glb")
    assert sorted(out) == sorted([seat.uid, "loose"])
    doc = _glb_json(tmp_path / "c.glb")
    roots = [doc["nodes"][i] for i in doc["scenes"][0]["nodes"]]
    assert {r["extras"]["built_uid"] for r in roots} == {seat.uid, "loose"}
    assert all("matrix" not in r for r in roots), "roots are in local coordinates"
    tagged = [m["extras"]["built_material"] for m in doc.get("materials", []) if "built_material" in m.get("extras", {})]
    assert tagged == ["Oak"], doc.get("materials")       # the default paint has no library name


# ---------------------------------------------------------------- applying Blender's changes

def test_a_transform_applies_as_one_undo_step():
    sc, h, seat, leg = _scene()
    before = _matrix(seat)
    c, s = math.cos(0.4), math.sin(0.4)
    new = np.array([[c, -s, 0, 3], [s, c, 0, 1], [0, 0, 1, 0.5], [0, 0, 0, 1]]) @ np.diag([1, 1.5, 1, 1])
    cmd, applied, failed = rb.link_apply(sc, [{"key": "transform:" + seat.uid, "value": {"matrix": _flat(new)}}], PLUGIN)
    assert applied == ["transform:" + seat.uid] and not failed
    depth = len(h.undo_stack)
    h.execute(cmd)
    assert len(h.undo_stack) == depth + 1
    assert np.allclose(_matrix(seat), new, atol=1e-9)
    h.undo()
    assert np.allclose(_matrix(seat), before)


def test_locked_mirrored_or_missing_placements_are_refused_per_item():
    sc, h, seat, leg = _scene()
    C.run(h, "lock", placements=[leg.uid])
    mirror = _flat(np.diag([-1, 1, 1, 1]))
    cmd, applied, failed = rb.link_apply(sc, [
        {"key": "transform:" + leg.uid, "value": {"matrix": _flat(np.eye(4))}},
        {"key": "transform:" + seat.uid, "value": {"matrix": mirror}},
        {"key": "transform:nope", "value": {"matrix": _flat(np.eye(4))}}], PLUGIN)
    assert cmd is None and applied == []
    assert "locked" in failed["transform:" + leg.uid]
    assert "mirrored" in failed["transform:" + seat.uid]
    assert "no longer" in failed["transform:nope"]


def test_a_material_takes_colour_roughness_metallic_and_an_image(tmp_path):
    sc, h, seat, leg = _scene()
    C.run(h, "create_material", color=[0.5, 0.3, 0.1], name="Oak")
    v = rb.link_values(sc, PLUGIN, ["material:Oak"])["material:Oak"]
    assert v == {"color": [0.5, 0.3, 0.1], "roughness": 1.0, "metallic": 0.0, "texture": None} or \
        v["color"] == pytest.approx([0.5, 0.3, 0.1])
    cmd, applied, _ = rb.link_apply(sc, [{"key": "material:Oak", "value": {
        "color": [0.6, 0.4, 0.2], "roughness": 0.35, "metallic": 0.1, "texture": None}}], PLUGIN)
    h.execute(cmd)
    m = sc.materials.get("Oak")
    assert tuple(m.color) == pytest.approx((0.6, 0.4, 0.2)) and m.pbr["roughness"] == pytest.approx(0.35)
    after = rb.link_values(sc, PLUGIN, ["material:Oak"])["material:Oak"]
    assert after["roughness"] == pytest.approx(0.35) and after["metallic"] == pytest.approx(0.1)
    # An image from Blender, written by the drawer as a file.
    png = tmp_path / "oiled.png"
    png.write_bytes(_png(4, 4, (120, 80, 40)))
    cmd, applied, _ = rb.link_apply(sc, [{"key": "material:Oak", "value": {
        "color": None, "roughness": 0.35, "metallic": 0.1, "texture": {"name": "oiled.png", "crc": "x"}}}],
        PLUGIN, {"material:Oak": png})
    h.execute(cmd)
    tex = rb.link_values(sc, PLUGIN, ["material:Oak"])["material:Oak"]["texture"]
    assert tex["name"] == "oiled.png" and tex["path"] == str(png)
    h.undo()
    h.undo()
    m = sc.materials.get("Oak")
    assert tuple(m.color) == pytest.approx((0.5, 0.3, 0.1)) and not m.pbr


def test_lights_from_blender_join_the_render_lights():
    sc, h, seat, leg = _scene()
    h.execute(SetPluginDataCommand(PLUGIN, {"ambience": "night", "lights": [
        {"kind": "point", "pos": [0, 0, 2], "power": 400, "id": "l1", "name": "Bulb"}]}))
    assert rb.link_values(sc, PLUGIN, ["light:l1"])["light:l1"]["power"] == 400
    cmd, applied, failed = rb.link_apply(sc, [
        {"key": "light:l1", "value": {"kind": "point", "pos": [0, 0, 2.5], "power": 600.0, "color": [1, 0.9, 0.8]}},
        {"key": "light:new7", "value": {"kind": "spot", "pos": [1, 1, 3], "dir": [0, 0, -1], "power": 900.0,
                                        "angle": 40.0, "color": [1, 1, 1], "name": "Studio key"}},
        {"key": "light:area", "value": {"kind": "area", "pos": [0, 0, 0]}}], PLUGIN)
    assert sorted(applied) == ["light:l1", "light:new7"] and "point and spot" in failed["light:area"]
    h.execute(cmd)
    data = sc.plugin_data[PLUGIN]
    assert data["ambience"] == "night"                       # the rest is kept
    lights = {lt["id"]: lt for lt in data["lights"]}
    assert lights["l1"]["power"] == 600 and lights["l1"]["pos"][2] == 2.5 and lights["l1"]["kelvin"] is None
    assert lights["new7"]["kind"] == "spot" and lights["new7"]["angle"] == 40
    h.undo()
    assert [lt["id"] for lt in sc.plugin_data[PLUGIN]["lights"]] == ["l1"]


def test_cameras_become_scenes_or_move_an_existing_one():
    sc, h, seat, leg = _scene()
    front = {"eye": [0.0, -8.0, 1.6], "target": [0.0, 0.0, 1.0], "up": [0.0, 0.0, 1.0], "fov_deg": 40.0,
             "perspective": True}
    cmd, applied, _ = rb.link_apply(sc, [{"key": "camera:Front", "value": front}], PLUGIN)
    h.execute(cmd)
    assert [v.name for v in sc.saved_views] == ["Front"]
    got = rb.link_values(sc, PLUGIN, ["camera:Front"])["camera:Front"]
    assert got["eye"] == pytest.approx(front["eye"], abs=1e-6) and got["fov_deg"] == pytest.approx(40)
    moved = {**front, "eye": [2.0, -8.0, 1.6]}
    cmd, applied, _ = rb.link_apply(sc, [{"key": "camera:Front", "value": moved}], PLUGIN)
    h.execute(cmd)
    assert len(sc.saved_views) == 1
    assert rb.link_values(sc, PLUGIN, ["camera:Front"])["camera:Front"]["eye"] == pytest.approx(moved["eye"], abs=1e-6)
    h.undo()
    assert rb.link_values(sc, PLUGIN, ["camera:Front"])["camera:Front"]["eye"] == pytest.approx(front["eye"], abs=1e-6)
    h.undo()
    assert sc.saved_views == [] or len(sc.saved_views) == 0


def test_a_mixed_batch_is_one_undo_step():
    sc, h, seat, leg = _scene()
    C.run(h, "create_material", color=[0.5, 0.3, 0.1], name="Oak")
    depth = len(h.undo_stack)
    cmd, applied, failed = rb.link_apply(sc, [
        {"key": "transform:" + seat.uid, "value": {"matrix": _flat(np.eye(4))}},
        {"key": "material:Oak", "value": {"roughness": 0.2}},
        {"key": "light:a", "value": {"kind": "point", "pos": [0, 0, 3], "power": 100.0}},
        {"key": "camera:Top", "value": {"eye": [0, 0, 10], "target": [0, 0, 0], "up": [0, 1, 0], "fov_deg": 30}},
    ], PLUGIN)
    assert len(applied) == 4 and not failed
    h.execute(cmd)
    assert len(h.undo_stack) == depth + 1
    h.undo()
    assert np.allclose(_matrix(seat)[:3, 3], [2, 0, 0]) and not sc.materials.get("Oak").pbr
    assert not (sc.plugin_data or {}).get(PLUGIN, {}).get("lights") and len(sc.saved_views) == 0


def test_conflicts_when_both_apps_changed_the_same_placement():
    sc, h, seat, leg = _scene()
    key = "transform:" + seat.uid
    base = rb.link_values(sc, PLUGIN, [key])[key]               # what Blender received
    review = rb.LinkReview()
    proposal = {"key": key, "kind": "transform", "label": "Seat", "base": base,
                "value": {"matrix": _flat(np.array([[1, 0, 0, 1.1], [0, 1, 0, 0], [0, 0, 1, 0], [0, 0, 0, 1]]))}}
    review.receive([proposal], rb.link_values(sc, PLUGIN, [key]))
    assert review.conflicts == 0
    (row,) = review.rows()
    assert row["label"] == "Seat · Position X" and row["built"] == "2.00 m" and row["blender"] == "1.10 m"
    C.run(h, "move_placements", placements=[seat.uid], delta=[0.5, 0, 0])   # Built moves it too
    review.revalidate(rb.link_values(sc, PLUGIN, review.keys()))
    assert review.conflicts == 1 and review.rows()[0]["built"] == "2.50 m"


# ---------------------------------------------------------------- the drawer, end to end

class _Timer:
    def __init__(self):
        self.active = False

    def start(self):
        self.active = True

    def stop(self):
        self.active = False

    def isActive(self):
        return self.active


class _App:
    key = PLUGIN

    def __init__(self, sc, h):
        self.scene, self.notes = sc, []
        self.viewport = types.SimpleNamespace(history=h, update=lambda: None)
        self.window = types.SimpleNamespace(notify=lambda tone, text, actions=None: self.notes.append(text),
                                            _current_path="/models/bench.igz")

    def document_data(self, default=None):
        data = self.scene.plugin_data or {}
        return json.loads(json.dumps(data[PLUGIN])) if PLUGIN in data else default

    def set_document_data(self, value):
        self.viewport.history.execute(SetPluginDataCommand(PLUGIN, value))


def _drawer(sc, h):
    from plugins._blender_panel import BlenderLink
    app = _App(sc, h)
    panel = types.SimpleNamespace(app=app, found=None, invalidate=lambda: None,
                                  host=types.SimpleNamespace(timers=types.SimpleNamespace(
                                      timer=lambda fn, ms, repeat: _Timer())))
    return BlenderLink(panel), app


@pytest.fixture
def linked(monkeypatch, tmp_path):
    monkeypatch.setattr(rb, "LINK_PORT", 0)
    sc, h, seat, leg = _scene()
    link, app = _drawer(sc, h)
    sys.path.insert(0, str(Path(__file__).parent))
    from test_blender_addon import FakeHost
    host = FakeHost()
    ctl = addon_controller.Controller(host)
    ctl.link(link.start())

    def pump():
        link.tick()
        ctl.tick(budget=0.05)
    _wait(lambda: link.linked and ctl.state == "linked", step=pump)
    yield types.SimpleNamespace(sc=sc, h=h, seat=seat, leg=leg, link=link, app=app, host=host, ctl=ctl, pump=pump)
    ctl.unlink()
    link.close()


def test_open_in_blender_sends_the_model_then_only_changes(linked):
    t = linked
    _wait(lambda: len(t.host.imported) >= 1 and t.link.progress is None and not t.link.queue, step=t.pump)
    assert {f"transform:{u}" for u in (t.seat.uid, t.leg.uid, "loose")} <= set(t.host.values)
    assert t.link.state()[0] == "Linked · bench.blend"
    assert t.app.scene.plugin_data[PLUGIN]["link_id"]                       # the document's link id
    sent = len(t.host.imported)
    C.run(t.h, "move_placements", placements=[t.seat.uid], delta=[1, 0, 0])
    _wait(lambda: t.host.values[f"transform:{t.seat.uid}"]["matrix"][12] == pytest.approx(3.0), step=t.pump)
    assert len(t.host.imported) == sent                                     # no re-export for a move


def test_blender_changes_wait_for_review_and_apply_as_one_step(linked):
    t = linked
    _wait(lambda: f"transform:{t.seat.uid}" in t.ctl.sync.local, step=t.pump)
    t.host.values[f"transform:{t.seat.uid}"] = {"matrix": _flat(np.array(
        [[1, 0, 0, 2.5], [0, 1, 0, 0], [0, 0, 1, 0], [0, 0, 0, 1]]))}
    t.host.values["light:k1"] = {"kind": "point", "pos": [0, 0, 3], "dir": [0, 0, -1], "color": [1, 1, 1],
                                 "power": 700.0, "angle": 60.0, "name": "Studio key", "on": True}
    assert t.ctl.send() == 2
    _wait(lambda: len(t.link.review) == 2, step=t.pump)
    assert t.link.state()[0] == "2 changes from Blender"
    rows = {r["key"]: r for r in t.link.rows()}
    assert rows[f"transform:{t.seat.uid}"]["cells"] == ["Seat · Position X", "2.00 m", "2.50 m"]
    assert [a[0] for a in rows["light:k1"]["actions"]] == ["apply", "ignore"]
    depth = len(t.h.undo_stack)
    t.link.event("link_apply_all", "invoke", None)
    assert len(t.h.undo_stack) == depth + 1 and len(t.link.review) == 0
    assert _matrix(t.seat)[0, 3] == pytest.approx(2.5)
    assert t.sc.plugin_data[PLUGIN]["lights"][0]["name"] == "Studio key"
    # Blender hears back and does not send the same change again.
    _wait(lambda: f"transform:{t.seat.uid}" not in t.ctl.sync.sent, step=t.pump)
    for _ in range(10):
        t.pump()
    assert t.ctl.changes()[0] == []


def test_a_conflict_offers_keep_built_or_take_blender(linked):
    t = linked
    key = f"transform:{t.seat.uid}"
    _wait(lambda: key in t.ctl.sync.local, step=t.pump)
    t.host.values[key] = {"matrix": _flat(np.array([[1, 0, 0, 1.1], [0, 1, 0, 0], [0, 0, 1, 0], [0, 0, 0, 1]]))}
    C.run(t.h, "move_placements", placements=[t.seat.uid], delta=[-0.75, 0, 0])   # Built: 1.25
    for _ in range(5):
        t.pump()
    assert t.host.values[key]["matrix"][12] == pytest.approx(1.1)          # Blender's edit kept
    assert t.ctl.send() == 1
    _wait(lambda: len(t.link.review) == 1, step=t.pump)
    (row,) = t.link.rows()
    assert row["flag"] and [a[0] for a in row["actions"]] == ["take", "keep"]
    assert row["cells"] == ["Seat · Position X", "1.25 m", "1.10 m"]
    assert t.link.state() == ("Linked · 1 conflict", False)
    t.link.event("link_changes", "row_action", {"row": key, "action": "keep"})
    _wait(lambda: t.host.values[key]["matrix"][12] == pytest.approx(1.25), step=t.pump)
    assert _matrix(t.seat)[0, 3] == pytest.approx(1.25)


def test_ignore_leaves_built_unchanged(linked):
    t = linked
    key = f"transform:{t.seat.uid}"
    _wait(lambda: key in t.ctl.sync.local, step=t.pump)
    t.host.values[key] = {"matrix": _flat(np.eye(4))}
    t.ctl.send()
    _wait(lambda: len(t.link.review) == 1, step=t.pump)
    depth = len(t.h.undo_stack)
    t.link.event("link_changes", "row_action", {"row": key, "action": "ignore"})
    assert len(t.h.undo_stack) == depth and _matrix(t.seat)[0, 3] == pytest.approx(2)
    _wait(lambda: key not in t.ctl.sync.sent, step=t.pump)
    assert t.ctl.changes()[0] == []                                        # not proposed again


def test_blender_quitting_and_coming_back_resends_only_what_changed(linked):
    t = linked
    _wait(lambda: t.link.progress is None and not t.link.queue and t.host.imported, step=t.pump)
    t.ctl.unlink()
    _wait(lambda: not t.link.linked, step=t.link.tick)
    assert t.link.state()[0] == "Waiting for Blender"
    C.run(t.h, "rename_placement", placement=t.leg.uid, name="Back leg")
    t.host.imported.clear()
    ctl = addon_controller.Controller(t.host)
    ctl.link(t.link.server.pairing)

    def pump():
        t.link.tick()
        ctl.tick(budget=0.05)
    _wait(lambda: t.link.linked and t.host.imported, step=pump)
    assert [uids for _, uids in t.host.imported] == [[t.leg.uid]]
    ctl.unlink()


def test_install_add_on_writes_an_installable_zip(tmp_path, monkeypatch):
    import zipfile
    out = rb.write_addon_zip(tmp_path / "built_link.zip")
    with zipfile.ZipFile(out) as z:
        names = z.namelist()
        assert z.testzip() is None
        assert "built_link/__init__.py" in names and "built_link/launch.py" in names
        assert all(n.startswith("built_link/") and n.endswith((".py", ".md")) for n in names)
        assert "bl_info" in z.read("built_link/__init__.py").decode()
