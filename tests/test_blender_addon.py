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
        self.images, self.names, self.fps, self.adopted = {}, {}, {}, {}

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

    def image_bytes(self, key, field):
        return self.images.get((key, field))

    def adopt(self, name, built_name):
        self.adopted[name] = built_name
        value = self.values.pop("newmaterial:" + name)
        self.values["material:" + built_name] = {k: v for k, v in value.items() if k not in ("name", "assign")}

    def unsupported_materials(self):
        return []


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
    host.images[("material:Oak", "texture")] = b"\x89PNG-bytes"
    host.values["transform:a"] = {"matrix": moved(1.1)}
    host.values["light:new1"] = {"kind": "point", "pos": [0, 0, 3], "power": 700.0}
    assert c.send() == 3
    (header, payload), = client.sent
    assert header["type"] == "changes" and payload == b"\x89PNG-bytes"
    keys = {i["key"]: i for i in header["items"]}
    assert keys["material:Oak"]["images"] == {"texture": {"offset": 0, "size": len(payload), "name": "oiled.png"}}
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


# ---------------------------------------------------------------- simple PBR (mocked node graphs)

class _Sock:
    def __init__(self, name, value=None):
        self.name, self.default_value, self.links = name, value, []

    @property
    def is_linked(self):
        return bool(self.links)


class _Inputs(list):
    def get(self, name):
        return next((s for s in self if s.name == name), None)

    def __getitem__(self, k):
        return self.get(k) if isinstance(k, str) else list.__getitem__(self, k)


class _Node:
    def __init__(self, type_, inputs=(), **attrs):
        self.type = type_
        self.inputs = _Inputs(_Sock(n, v) for n, v in inputs)
        self.__dict__.update(attrs)


def _link(src, out, dst):
    dst.links.append(types.SimpleNamespace(from_node=src, from_socket=types.SimpleNamespace(name=out)))


def _image(name):
    return types.SimpleNamespace(name=name, filepath=f"//{name}")


def _graph(**overrides):
    bsdf = _Node("BSDF_PRINCIPLED", [("Base Color", (0.6, 0.4, 0.2, 1.0)), ("Metallic", 0.0), ("Roughness", 0.35),
                                     ("Normal", None), ("Alpha", 1.0), ("Emission Color", (1.0, 1.0, 1.0, 1.0)),
                                     ("Emission Strength", 0.0), ("Coat Weight", 0.0)])
    out = _Node("OUTPUT_MATERIAL", [("Surface", None), ("Volume", None), ("Displacement", None)],
                is_active_output=True)
    _link(bsdf, "BSDF", out.inputs["Surface"])
    nodes = [out, bsdf]
    mat = types.SimpleNamespace(name="Oiled oak", use_nodes=True, node_tree=types.SimpleNamespace(nodes=nodes))
    return mat, bsdf, out, nodes


def _record(image):
    return {"name": image.name, "crc": "c-" + image.name}


def test_a_constant_principled_material_is_simple_pbr():
    from built_link import pbr
    mat, *_ = _graph()
    value, images, reason = pbr.extract(mat, _record)
    assert reason is None and images == {}
    assert value["color"] == pytest.approx([0.6, 0.4, 0.2]) and value["roughness"] == 0.35
    assert value["emission"] == [0, 0, 0] and value["emission_strength"] == 0     # strength 0: none


def test_images_mapping_normal_map_alpha_and_emission_are_extracted():
    from built_link import pbr
    mat, bsdf, out, nodes = _graph()
    uv = _Node("UVMAP")
    mapping = _Node("MAPPING", [("Vector", None), ("Location", (0, 0, 0)), ("Rotation", (0, 0, math.radians(90))),
                                ("Scale", (2.0, 4.0, 1.0))])
    _link(uv, "UV", mapping.inputs["Vector"])
    base = _Node("TEX_IMAGE", [("Vector", None)], image=_image("oak.png"), projection="FLAT")
    _link(mapping, "Vector", base.inputs["Vector"])
    _link(base, "Color", bsdf.inputs["Base Color"])
    _link(base, "Alpha", bsdf.inputs["Alpha"])
    rough = _Node("TEX_IMAGE", [("Vector", None)], image=_image("oak_r.png"), projection="FLAT")
    _link(rough, "Color", bsdf.inputs["Roughness"])
    nimg = _Node("TEX_IMAGE", [("Vector", None)], image=_image("oak_n.png"), projection="FLAT")
    nmap = _Node("NORMAL_MAP", [("Strength", 0.7), ("Color", None)], space="TANGENT")
    _link(nimg, "Color", nmap.inputs["Color"])
    _link(nmap, "Normal", bsdf.inputs["Normal"])
    bsdf.inputs["Emission Color"].default_value = (1.0, 0.5, 0.0, 1.0)
    bsdf.inputs["Emission Strength"].default_value = 3.0
    nodes.extend([uv, mapping, base, rough, nimg, nmap])
    value, images, reason = pbr.extract(mat, _record)
    assert reason is None
    assert value["color"] is None and value["texture"] == {"name": "oak.png", "crc": "c-oak.png"}
    assert value["tile"] == [0.5, 0.25] and value["rot"] == pytest.approx(90)
    assert value["roughness_map"]["name"] == "oak_r.png" and value["roughness"] == 1.0
    assert value["normal_map"]["name"] == "oak_n.png" and value["normal_scale"] == 0.7
    assert value["opacity"] == 1.0
    assert value["emission"] == [1.0, 0.5, 0.0] and value["emission_strength"] == 3.0
    assert set(images) == {"texture", "roughness_map", "normal_map"}


def test_occlusion_from_the_gltf_output_group():
    from built_link import pbr
    mat, bsdf, out, nodes = _graph()
    ao = _Node("TEX_IMAGE", [("Vector", None)], image=_image("ao.png"), projection="FLAT")
    group = _Node("GROUP", [("Occlusion", None)], node_tree=types.SimpleNamespace(name="glTF Material Output"))
    _link(ao, "Color", group.inputs["Occlusion"])
    nodes.extend([ao, group])
    value, images, reason = pbr.extract(mat, _record)
    assert reason is None and value["ao_map"]["name"] == "ao.png" and "ao_map" in images


@pytest.mark.parametrize("case", ["mix", "noise", "group", "coat", "math_roughness", "alpha_map", "object_coords"])
def test_anything_else_stays_in_blender(case):
    from built_link import pbr
    mat, bsdf, out, nodes = _graph()
    if case == "mix":
        mix = _Node("MIX_SHADER", [("Fac", 0.5), ("Shader", None)])
        out.inputs["Surface"].links = []
        _link(mix, "Shader", out.inputs["Surface"])
        _link(bsdf, "BSDF", mix.inputs["Shader"])
    elif case == "noise":
        _link(_Node("TEX_NOISE"), "Color", bsdf.inputs["Base Color"])
    elif case == "group":
        _link(_Node("GROUP", node_tree=types.SimpleNamespace(name="Wood")), "Color", bsdf.inputs["Base Color"])
    elif case == "coat":
        _link(_Node("VALUE"), "Value", bsdf.inputs["Coat Weight"])
    elif case == "math_roughness":
        img = _Node("TEX_IMAGE", [("Vector", None)], image=_image("r.png"), projection="FLAT")
        sep = _Node("SEPARATE_COLOR", [("Color", None)])
        _link(img, "Color", sep.inputs["Color"])
        _link(sep, "Green", bsdf.inputs["Roughness"])
    elif case == "alpha_map":
        _link(_Node("TEX_IMAGE", [("Vector", None)], image=_image("a.png"), projection="FLAT"), "Color",
              bsdf.inputs["Alpha"])
    elif case == "object_coords":
        img = _Node("TEX_IMAGE", [("Vector", None)], image=_image("b.png"), projection="FLAT")
        _link(_Node("TEX_COORD"), "Object", img.inputs["Vector"])
        _link(img, "Color", bsdf.inputs["Base Color"])
    value, images, reason = pbr.extract(mat, _record)
    assert reason, case


def test_lenient_reading_of_a_material_built_sent():
    from built_link import pbr
    mat, bsdf, out, nodes = _graph()
    img = _Node("TEX_IMAGE", [("Vector", None)], image=_image("mr.png"), projection="FLAT")
    sep = _Node("SEPARATE_COLOR", [("Color", None)])
    _link(img, "Color", sep.inputs["Color"])
    _link(sep, "Green", bsdf.inputs["Roughness"])
    value, _, reason = pbr.extract(mat, _record, lenient=True)
    assert reason and value["color"] == pytest.approx([0.6, 0.4, 0.2])


class _Polys(list):
    def foreach_get(self, attr, out):
        for i, p in enumerate(self):
            out[i] = p.material_index


def _host_with(objects, materials):
    """blender_scene.Host over a mocked bpy holding these objects."""
    bpy, mathutils, _ = _fake_bpy()
    coll = types.SimpleNamespace(all_objects=objects)
    bpy.data = types.SimpleNamespace(collections={"Built": coll}, materials={m.name: m for m in materials})
    bpy.path = types.SimpleNamespace(abspath=lambda p: p.lstrip("/"))
    return bpy, mathutils


class _Mat(dict):
    """A material: custom properties as a dict, hashable as bpy's are."""
    __hash__ = object.__hash__
    __eq__ = object.__eq__

    def __init__(self, mat):
        super().__init__()
        self.__dict__.update(mat.__dict__)


def test_new_materials_map_to_placements_and_slots(monkeypatch):
    new, *_ = _graph()
    new = _Mat(new)
    shader, sbsdf, sout, _ = _graph()
    shader = _Mat(shader)
    shader.name = "Glass shader"
    sout.inputs["Surface"].links = []
    _link(_Node("MIX_SHADER"), "Shader", sout.inputs["Surface"])
    oak = _Mat(types.SimpleNamespace(name="Oak", use_nodes=False, node_tree=None))
    oak["built_material"] = "Oak"

    def obj(uid, slots, indices, origin):
        o = {"built_uid": uid, "built_slots": "\x1f".join(origin)}
        holder = types.SimpleNamespace(type="MESH", material_slots=[types.SimpleNamespace(material=m) for m in slots],
                                       data=types.SimpleNamespace(polygons=_Polys(
                                           types.SimpleNamespace(material_index=i) for i in indices)))

        class O(dict):
            pass
        out = O(o)
        out.__dict__.update(holder.__dict__)
        return out
    whole = obj("seat", [new], [0, 0, 0], ["Oak"])
    slot = obj("leg", [oak, new], [0, 1, 1], ["Oak", "Steel"])
    added = obj("rail", [oak, new], [0, 1], ["Oak"])           # a slot Built never had
    kept = obj("panel", [shader], [0], ["Oak"])
    bpy, mathutils = _host_with([whole, slot, added, kept], [new, shader, oak])
    monkeypatch.setitem(sys.modules, "bpy", bpy)
    monkeypatch.setitem(sys.modules, "mathutils", mathutils)
    sys.modules.pop("built_link.blender_scene", None)
    import built_link
    monkeypatch.delattr(built_link, "blender_scene", raising=False)
    blender_scene = importlib.import_module("built_link.blender_scene")
    monkeypatch.setattr(blender_scene, "_image_crc", lambda image: "x")
    host = blender_scene.Host()
    found = host._new_materials()
    assert [a["uid"] for a in found[new]] == ["seat", "leg", "rail"]
    assert found[new][0] == {"uid": "seat", "from": "", "whole": True} or found[new][0]["whole"]
    assert found[new][1] == {"uid": "leg", "from": "Steel", "whole": False}
    assert found[new][2] == {"uid": "rail", "from": "", "whole": False}
    record = host._new_material(new, found[new])
    assert record["name"] == "Oiled oak" and record["assign_note"]
    assert host._new_material(shader, found[shader]) == {"unsupported": "Node setup not supported"}
    assert host.unsupported_materials() == [("Glass shader", "Node setup not supported")]
    sys.modules.pop("built_link.blender_scene", None)


def test_a_new_material_goes_out_and_becomes_a_built_material_once_applied():
    c, host, client = _linked()
    value = {"color": None, "texture": {"name": "oak.png", "crc": "aa"}, "roughness": 0.35, "name": "Oiled oak",
             "assign": [{"uid": "seat", "from": "Oak", "whole": True}]}
    host.values["newmaterial:Oiled oak"] = value
    host.images[("newmaterial:Oiled oak", "texture")] = b"PNGDATA"
    assert c.send() == 1
    (header, payload), = client.sent
    (item,) = header["items"]
    assert item["kind"] == "newmaterial" and item["base"] is None
    assert item["images"]["texture"]["size"] == 7 and payload == b"PNGDATA"
    built = {"color": None, "texture": {"name": "oak.png", "crc": "aa"}, "roughness": 0.35}
    c.handle({"type": "resolved", "key": "newmaterial:Oiled oak", "resolution": "applied",
              "value": {**built, "name": "Oiled oak (Blender)"}})
    assert host.adopted == {"Oiled oak": "Oiled oak (Blender)"}
    assert c.changes()[0] == []                     # nothing new to send
    host.values["material:Oiled oak (Blender)"]["roughness"] = 0.2
    (edit,), _ = c.changes()
    assert edit["key"] == "material:Oiled oak (Blender)" and edit["value"]["roughness"] == 0.2
