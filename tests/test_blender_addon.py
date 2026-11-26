# SPDX-License-Identifier: GPL-3.0-or-later
"""The Built Link add-on for Blender, without Blender: its wire format,
what counts as a change, how Built's messages drive the scene (a fake
host stands in for bpy), and that it registers against a mocked bpy."""
from __future__ import annotations

import importlib
import math
import sys
import types
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "integrations" / "blender"))

from built_link import controller as C  # noqa: E402
from built_link import convert, sync, wire  # noqa: E402

IDENTITY = [1.0, 0, 0, 0, 0, 1.0, 0, 0, 0, 0, 1.0, 0, 0, 0, 0, 1.0]


def moved(x, y=0.0, z=0.0):
    return [1.0, 0, 0, 0, 0, 1.0, 0, 0, 0, 0, 1.0, 0, x, y, z, 1.0]


# ---------------------------------------------------------------- wire

def test_frames_round_trip_and_large_payloads_are_split_and_joined():
    big = bytes(range(256)) * 9000          # 2.3 MB: three frames at 1 MiB
    data = wire.encode({"type": "chunk", "items": [{"uid": "a"}]}, big, msg=5) + \
        wire.encode({"type": "transform", "uid": "b"}, b"", msg=6)
    asm = wire.Assembler()
    out = []
    for i in range(0, len(data), 7777):     # arbitrary segment boundaries
        out.extend(asm.feed(data[i:i + 7777]))
    assert [h["type"] for h, _ in out] == ["chunk", "transform"]
    assert out[0][1] == big and out[0][0]["items"] == [{"uid": "a"}]
    assert "msg" not in out[0][0] and asm.pending() == 0


def test_malformed_frames_raise():
    with pytest.raises(wire.WireError):
        wire.Assembler().feed(b"\x00\x00\x00\x05\x00\x00\x00\x00[1,2]")
    with pytest.raises(wire.WireError):
        wire.Assembler().feed(b"\xff\xff\xff\xff\x00\x00\x00\x00")
    parts = wire.encode({"type": "chunk"}, b"x" * 3000, msg=2, chunk=1000)
    second = parts[len(parts) // 3:]          # starts mid-message
    with pytest.raises(wire.WireError):
        wire.Assembler().feed(second)


def test_pairing_codes():
    assert wire.pairing(" built:47815:abcd-efgh ") == (47815, "ABCD-EFGH")
    for bad in ("", "47815:ABCD", "built:port:ABCD", "built:0:ABCD", "built:47815:"):
        with pytest.raises(ValueError):
            wire.pairing(bad)


# ---------------------------------------------------------------- sync

def test_blender_float_noise_is_not_a_change():
    s = sync.Sync()
    s.accepted("transform:u1", {"matrix": moved(1.25)}, {"matrix": moved(1.25 + 3e-7)})
    assert s.proposals({"transform:u1": {"matrix": moved(1.25 + 1e-7)}}) == []


def test_a_change_is_sent_once_with_the_base_built_sent():
    s = sync.Sync()
    s.accepted("transform:u1", {"matrix": moved(1.25)}, {"matrix": moved(1.25)})
    records = {"transform:u1": {"matrix": moved(1.10)}}
    items = s.proposals(records, labels={"transform:u1": "Bench seat"})
    assert items == [{"key": "transform:u1", "kind": "transform", "label": "Bench seat",
                      "base": {"matrix": moved(1.25)}, "value": {"matrix": moved(1.10)}}]
    s.mark_sent(items, records)
    assert s.proposals(records) == []
    assert s.proposals({"transform:u1": {"matrix": moved(1.0)}})  # moved again


def test_only_the_fields_blender_changed_replace_builts_value():
    s = sync.Sync()
    built = {"color": [0.5, 0.3, 0.1], "roughness": 0.8, "metallic": 0.0, "texture": None}
    # Blender reads the colour back slightly differently (no change).
    s.accepted("material:Oak", built, {**built, "color": [0.50001, 0.3, 0.1]})
    record = {**built, "color": [0.50001, 0.3, 0.1], "roughness": 0.35}
    (item,) = s.proposals(record and {"material:Oak": record})
    assert item["value"] == {**built, "roughness": 0.35}


def test_kinds_filter_and_new_items_have_no_base():
    s = sync.Sync()
    records = {"light:a1": {"kind": "point", "power": 700.0},
               "camera:Front": {"eye": [0, -5, 1.6]}}
    items = s.proposals(records, kinds=("light",))
    assert [i["key"] for i in items] == ["light:a1"] and items[0]["base"] is None


def test_built_values_wait_behind_a_local_change_and_resolve():
    s = sync.Sync()
    s.accepted("transform:u1", {"matrix": moved(1.0)}, {"matrix": moved(1.0)})
    local = {"matrix": moved(2.0)}
    assert not s.should_apply("transform:u1", local)
    s.hold("transform:u1", {"matrix": moved(3.0)})
    (item,) = s.proposals({"transform:u1": local})
    assert item["base"] == {"matrix": moved(1.0)}       # Built will see the conflict
    s.resolved("transform:u1", "ignored", None, local)
    assert s.base["transform:u1"] == {"matrix": moved(3.0)} and not s.remote
    assert s.proposals({"transform:u1": local}) == []


# ---------------------------------------------------------------- convert

def test_matrices_travel_column_major():
    rows = [[1, 0, 0, 4], [0, 1, 0, 5], [0, 0, 1, 6], [0, 0, 0, 1]]
    assert convert.flat(rows)[12:15] == [4.0, 5.0, 6.0]
    assert convert.rows(convert.flat(rows)) == [[float(v) for v in r] for r in rows]


def test_lights_and_cameras_read_back_what_built_sent():
    value = {"kind": "spot", "pos": [1.0, 2.0, 3.0], "dir": [0.0, 0.6, -0.8], "color": [1.0, 0.9, 0.8],
             "power": 1500.0, "angle": 50.0, "name": "Lamp", "on": True}
    pose = convert.light_pose(value)
    back = convert.light_value("SPOT", pose, value["color"], value["power"], math.radians(50), "Lamp")
    assert sync.same(back, value)
    area = convert.light_value("AREA", pose, value["color"], 100.0, 0.0, "Panel")
    assert area["unsupported"] == "Area lights stay in Blender"
    cam = {"eye": [0.0, -8.0, 1.6], "target": [0.0, 0.0, 1.0], "up": [0.0, 0.07479, 0.99720],
           "fov_deg": 40.0, "perspective": True}
    m, d = convert.camera_pose(cam)
    back = convert.camera_value(m, d, 40.0, True)
    assert sync.same(back["eye"], cam["eye"]) and sync.same(back["target"], cam["target"])
    assert abs(sum(a * b for a, b in zip(back["up"], cam["up"])) - 1) < 1e-3


def test_vertical_field_of_view():
    h = math.radians(60)
    assert convert.vertical_fov("VERTICAL", h, math.radians(40), 1920, 1080) == pytest.approx(40)
    v = convert.vertical_fov("AUTO", h, 0, 1920, 1080)
    assert v == pytest.approx(math.degrees(2 * math.atan(math.tan(h / 2) * 1080 / 1920)))


# ---------------------------------------------------------------- controller

class FakeClient:
    def __init__(self):
        self.state, self.message, self.sent, self.inbox = "linked", "", [], []
        self.hello, self.port, self.token = {}, None, None

    def drain(self, limit=64):
        out, self.inbox = self.inbox[:limit], self.inbox[limit:]
        return out

    def send(self, header, payload=b""):
        self.sent.append((header, payload))
        return True

    def start(self, port, token, hello=None, relink=False):
        self.port, self.token, self.hello, self.relink = port, token, hello, relink

    def stop(self):
        self.state = "idle"


class FakeHost:
    """The scene as plain values: what Blender would hold."""

    def __init__(self):
        self.values, self.imported, self.removed, self.doc = {}, [], [], ("", "")
        self.images, self.names, self.fps = {}, {}, {}

    def document(self):
        return self.doc[0]

    def set_document(self, d, n):
        self.doc = (d, n)

    def file_name(self):
        return "bench.blend"

    def file_path(self):
        return "/tmp/bench.blend"

    def version(self):
        return "5.2"

    def have(self):
        return dict(self.fps)

    def import_glb(self, payload, items):
        self.imported.append((len(payload), [i["uid"] for i in items]))
        for i in items:
            self.values["transform:" + i["uid"]] = {"matrix": list(i["matrix"])}
            self.names["transform:" + i["uid"]] = i.get("name") or i["uid"]
            self.fps[i["uid"]] = i.get("fp", "")

    def remove(self, uid):
        self.removed.append(uid)
        self.fps.pop(uid, None)
        self.values.pop("transform:" + uid, None)

    def record(self, key):
        v = self.values.get(key)
        return dict(v) if v is not None else None

    def records(self):
        return ({k: dict(v) for k, v in self.values.items()},
                {k: self.names.get(k, k.split(":", 1)[1].title()) for k in self.values})

    def apply(self, key, value):
        self.values[key] = dict(value)

    def keys(self, prefix):
        return [k for k in self.values if k.startswith(prefix)]

    def delete(self, key):
        self.values.pop(key, None)

    def assign_ids(self):
        pass

    def texture_bytes(self, key):
        return self.images.get(key)


def _linked():
    host, client = FakeHost(), FakeClient()
    c = C.Controller(host, client)
    c.handle({"type": "_linked", "document": "doc-1", "name": "bench.igz"})
    return c, host, client


def test_the_model_arrives_in_chunks_then_updates_and_removals():
    c, host, client = _linked()
    assert host.doc == ("doc-1", "bench.igz") and c.status() == "Linked to bench.igz"
    client.inbox = [({"type": "begin", "count": 3}, b""),
                    ({"type": "chunk", "items": [{"uid": "a", "name": "Seat", "matrix": moved(1), "fp": "f1"},
                                                 {"uid": "b", "name": "Leg", "matrix": IDENTITY, "fp": "f2"}]}, b"GLB1"),
                    ({"type": "chunk", "items": [{"uid": "loose", "name": "Loose", "matrix": IDENTITY, "fp": "f3"}]},
                     b"GLB2")]
    c.tick(budget=1)
    assert host.imported == [(4, ["a", "b"]), (4, ["loose"])]
    assert c.status().startswith("Receiving the model · 3 / 3")
    client.inbox = [({"type": "end"}, b""), ({"type": "transform", "uid": "a", "matrix": moved(2)}, b""),
                    ({"type": "remove", "uid": "b"}, b"")]
    c.tick(budget=1)
    assert host.values["transform:a"] == {"matrix": moved(2)} and host.removed == ["b"]
    assert c.progress is None and c.changes()[0] == []     # Built's own updates are not changes


def test_blender_changes_go_out_as_one_batch_with_image_bytes():
    c, host, client = _linked()
    oak = {"color": None, "roughness": 0.8, "metallic": 0.0, "texture": {"name": "oak.png", "crc": "aa"}}
    c.handle({"type": "material", "name": "Oak", "value": oak})
    c.handle({"type": "chunk", "items": [{"uid": "a", "name": "Seat", "matrix": moved(1), "fp": "f"}]}, b"G")
    host.values["material:Oak"] = {**oak, "texture": {"name": "oiled.png", "crc": "bb"}}
    host.images["material:Oak"] = b"\x89PNG-bytes"
    host.values["transform:a"] = {"matrix": moved(1.1)}
    host.values["light:new1"] = {"kind": "point", "pos": [0, 0, 3], "power": 700.0}
    assert c.send() == 3
    (header, payload), = client.sent
    assert header["type"] == "changes" and payload == b"\x89PNG-bytes"
    keys = {i["key"]: i for i in header["items"]}
    assert keys["material:Oak"]["image"] == {"offset": 0, "size": len(payload), "name": "oiled.png"}
    assert keys["light:new1"]["base"] is None
    assert keys["transform:a"]["base"] == {"matrix": moved(1)}
    assert c.send() == 0                                    # nothing new


def test_resolutions_settle_each_key():
    c, host, client = _linked()
    c.handle({"type": "chunk", "items": [{"uid": "a", "name": "Seat", "matrix": moved(1), "fp": "f"}]}, b"G")
    host.values["transform:a"] = {"matrix": moved(1.5)}
    c.send()
    # Built applied it: its value is now Blender's, nothing to send again.
    c.handle({"type": "resolved", "key": "transform:a", "resolution": "applied", "value": {"matrix": moved(1.5)}})
    assert c.changes()[0] == []
    # A later edit, then Built keeps its own: Blender takes Built's value back.
    host.values["transform:a"] = {"matrix": moved(4)}
    c.send()
    c.handle({"type": "resolved", "key": "transform:a", "resolution": "kept_built", "value": {"matrix": moved(1.5)}})
    assert host.values["transform:a"] == {"matrix": moved(1.5)} and c.changes()[0] == []


def test_a_built_update_waits_behind_a_local_edit():
    c, host, client = _linked()
    c.handle({"type": "chunk", "items": [{"uid": "a", "name": "Seat", "matrix": moved(1), "fp": "f"}]}, b"G")
    host.values["transform:a"] = {"matrix": moved(1.1)}          # edited in Blender
    c.handle({"type": "transform", "uid": "a", "matrix": moved(1.25)})  # edited in Built
    assert host.values["transform:a"] == {"matrix": moved(1.1)}   # Blender's edit kept
    (item,), _ = c.changes()
    assert item["base"] == {"matrix": moved(1)}                   # Built sees the conflict


def test_lights_and_cameras_from_built_replace_the_managed_ones():
    c, host, client = _linked()
    c.handle({"type": "lights", "items": [{"id": "l1", "kind": "point", "pos": [0, 0, 2], "power": 400.0},
                                          {"id": "l2", "kind": "spot", "pos": [1, 0, 2], "power": 900.0}]})
    c.handle({"type": "cameras", "items": [{"name": "Front", "eye": [0, -5, 1.6], "target": [0, 0, 1]}]})
    assert host.values["light:l1"]["power"] == 400.0 and "id" not in host.values["light:l1"]
    assert host.values["camera:Front"] == {"eye": [0, -5, 1.6], "target": [0, 0, 1]}
    c.handle({"type": "lights", "items": [{"id": "l2", "kind": "spot", "pos": [1, 0, 2], "power": 900.0}]})
    assert "light:l1" not in host.values and "light:l2" in host.values


def test_refusal_and_closing_are_reported():
    host, client = FakeHost(), FakeClient()
    c = C.Controller(host, client)
    c.handle({"type": "_refused", "reason": "document", "message": "This .blend is linked to another Built model."})
    assert c.refused_reason == "document" and "another Built model" in c.notice
    c.handle({"type": "_closed", "reason": "Built closed the link"})
    assert c.progress is None and c.notice == "Built closed the link"
    c.link("built:47815:ABCD-EFGH")
    assert client.port == 47815 and client.token == "ABCD-EFGH"
    assert client.hello["file"] == "bench.blend" and client.hello["path"] == "/tmp/bench.blend"


# ---------------------------------------------------------------- bpy mocked

class _Props(types.ModuleType):
    def __getattr__(self, name):
        return lambda **kw: ("prop", name, kw)


def _fake_bpy():
    bpy = types.ModuleType("bpy")
    registered = []

    class Base:
        pass

    bpy.types = types.SimpleNamespace(Operator=Base, Panel=Base, AddonPreferences=Base,
                                      WindowManager=type("WindowManager", (), {}))
    bpy.props = _Props("bpy.props")
    bpy.utils = types.SimpleNamespace(register_class=registered.append,
                                      unregister_class=registered.remove)
    timers = []
    bpy.app = types.SimpleNamespace(timers=types.SimpleNamespace(
        is_registered=lambda f: f in timers, register=lambda f, **kw: timers.append(f),
        unregister=timers.remove), version_string="5.2.0")
    bpy.context = types.SimpleNamespace(preferences=types.SimpleNamespace(addons={}))
    mathutils = types.ModuleType("mathutils")
    mathutils.Matrix = list
    return bpy, mathutils, registered


def test_the_addon_registers_and_unregisters_with_bpy_mocked(monkeypatch):
    bpy, mathutils, registered = _fake_bpy()
    monkeypatch.setitem(sys.modules, "bpy", bpy)
    monkeypatch.setitem(sys.modules, "mathutils", mathutils)
    for name in ("built_link.ui", "built_link.blender_scene"):
        sys.modules.pop(name, None)
    import built_link
    ui = importlib.import_module("built_link.ui")
    assert built_link.bl_info["name"] == "Built Link" and built_link.bl_info["blender"] >= (4, 2, 0)
    built_link.register()
    names = [c.__name__ for c in registered]
    assert {"BUILT_OT_link", "BUILT_OT_send", "BUILT_OT_unlink", "VIEW3D_PT_built_link"} <= set(names)
    assert ui.VIEW3D_PT_built_link.bl_category == "Built"
    assert hasattr(bpy.types.WindowManager, "built_send_transforms")
    built_link.unregister()
    assert registered == [] and not hasattr(bpy.types.WindowManager, "built_send_transforms")
    for name in ("built_link.ui", "built_link.blender_scene"):
        sys.modules.pop(name, None)


def test_every_addon_module_compiles():
    import py_compile
    for path in (ROOT / "integrations" / "blender" / "built_link").glob("*.py"):
        py_compile.compile(str(path), doraise=True)


def test_open_in_blender_links_at_once_with_the_code_built_passes(monkeypatch):
    bpy, mathutils, registered = _fake_bpy()
    timers = []
    bpy.app.timers.register = lambda f, **kw: timers.append(f)
    monkeypatch.setitem(sys.modules, "bpy", bpy)
    monkeypatch.setitem(sys.modules, "mathutils", mathutils)
    for name in ("built_link.ui", "built_link.blender_scene"):
        sys.modules.pop(name, None)
    ui = importlib.import_module("built_link.ui")
    calls = []
    monkeypatch.setattr(ui, "link", lambda pairing, relink=False: calls.append(pairing))
    monkeypatch.setattr(ui, "register", lambda: None)
    monkeypatch.setenv("BUILT_LINK", "built:47815:ABCD-EFGH")
    source = (ROOT / "integrations/blender/built_link/launch.py").read_text()
    exec(compile(source, "launch.py", "exec"), {"__file__": str(ROOT / "integrations/blender/built_link/launch.py"),
                                               "__name__": "__main__"})
    (start,) = timers
    assert start() is None and calls == ["built:47815:ABCD-EFGH"]
    for name in ("built_link.ui", "built_link.blender_scene"):
        sys.modules.pop(name, None)
