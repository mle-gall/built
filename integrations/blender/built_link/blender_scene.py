# SPDX-License-Identifier: GPL-3.0-or-later
"""The Blender side of the link's host: where Built's model lives in the
.blend and how its objects read back as Built values.

Built's placements land in the "Built" collection; each top-level
placement is one root object carrying ``built_uid`` (and ``built_fp``, the
geometry fingerprint Built sent). Materials carry ``built_material``,
lights ``built_light`` and cameras ``built_camera``. Anything else in the
.blend is Blender's own and never touched.
"""
import math
import os
import tempfile
import uuid
import zlib

import bpy
from mathutils import Matrix

from . import convert, pbr

MODEL = "Built"
SEP = "\x1f"
LIGHTS = "Built lights"
CAMERAS = "Built cameras"


def _collection(name):
    coll = bpy.data.collections.get(name)
    if coll is None:
        coll = bpy.data.collections.new(name)
    scene = bpy.context.scene
    if coll.name not in scene.collection.children:
        scene.collection.children.link(coll)
    return coll


def _layer_collection(layer, coll):
    if layer.collection == coll:
        return layer
    for child in layer.children:
        found = _layer_collection(child, coll)
        if found is not None:
            return found
    return None


def _rows(obj):
    return [list(r) for r in obj.matrix_world]


def _subtree(obj):
    out = [obj]
    for child in obj.children:
        out.extend(_subtree(child))
    return out


def _bsdf(mat):
    if not mat.use_nodes or mat.node_tree is None:
        return None
    return next((n for n in mat.node_tree.nodes if n.type == "BSDF_PRINCIPLED"), None)


def _image_of(mat):
    bsdf = _bsdf(mat)
    if bsdf is None:
        return None
    socket = bsdf.inputs.get("Base Color")
    if socket is None or not socket.is_linked:
        return None
    node = socket.links[0].from_node
    return node.image if node.type == "TEX_IMAGE" else None


_crc_cache = {}


def _image_bytes(image):
    if image.packed_file is not None:
        return bytes(image.packed_file.data)
    path = bpy.path.abspath(image.filepath)
    if path and os.path.isfile(path):
        with open(path, "rb") as fh:
            return fh.read()
    return None


def _image_data(image):
    """The image's bytes; an image never saved is packed first (as PNG)."""
    data = _image_bytes(image)
    if data is None and image.packed_file is None:
        try:
            image.pack()
        except RuntimeError:
            return None
        data = _image_bytes(image)
    return data


def _material_indices(mesh):
    polys = mesh.polygons
    try:
        import array
        out = array.array("i", [0]) * len(polys)
        polys.foreach_get("material_index", out)
        return list(out)
    except (AttributeError, TypeError):
        return [p.material_index for p in polys]


def _image_crc(image):
    key = (image.name, image.filepath, image.packed_file is not None, image.is_dirty, tuple(image.size))
    if key not in _crc_cache:
        data = _image_bytes(image)
        _crc_cache[key] = f"{zlib.crc32(data) & 0xffffffff:08x}" if data else None
    return _crc_cache[key]


def _render_size():
    r = bpy.context.scene.render
    return r.resolution_x, r.resolution_y


class Host:
    """What the controller asks of Blender."""

    # ------------------------------------------------------------ identity

    def document(self):
        return bpy.context.scene.get("built_document", "")

    def set_document(self, document, name):
        scene = bpy.context.scene
        scene["built_document"] = document
        scene["built_name"] = name

    def file_name(self):
        return os.path.basename(bpy.data.filepath) or "Untitled.blend"

    def file_path(self):
        return bpy.data.filepath

    def version(self):
        return bpy.app.version_string

    def roots(self):
        coll = bpy.data.collections.get(MODEL)
        if coll is None:
            return {}
        return {o["built_uid"]: o for o in coll.all_objects if o.get("built_root") and "built_uid" in o}

    def have(self):
        return {uid: o.get("built_fp", "") for uid, o in self.roots().items()}

    # ------------------------------------------------------------ geometry

    def import_glb(self, payload, items):
        coll = _collection(MODEL)
        view_layer = bpy.context.view_layer
        active = view_layer.active_layer_collection
        layer = _layer_collection(view_layer.layer_collection, coll)
        fd, path = tempfile.mkstemp(suffix=".glb", prefix="built-link-")
        with os.fdopen(fd, "wb") as fh:
            fh.write(payload)
        before_objects = set(bpy.data.objects)
        before_materials = set(bpy.data.materials)
        try:
            if layer is not None:
                view_layer.active_layer_collection = layer
            bpy.ops.import_scene.gltf(filepath=path)
        finally:
            view_layer.active_layer_collection = active
            os.unlink(path)
        new = [o for o in bpy.data.objects if o not in before_objects]
        for o in new:
            if o.name not in coll.objects and not o.users_collection:
                coll.objects.link(o)
        self._share_materials(new, before_materials)
        for m in bpy.data.materials:
            if m not in before_materials and not m.get("built_material"):
                m["built_paint"] = True       # Built's own unnamed paint, not a new material
        for o in new:
            if o.type == "MESH":
                # Which Built material each slot started as: a slot that
                # later wears a Blender material maps back to those faces.
                o["built_slots"] = SEP.join((slot.material.get("built_material", "") if slot.material else "")
                                            for slot in o.material_slots)
        by_uid = {o.get("built_uid"): o for o in new if o.parent is None}
        old = self.roots()
        for it in items:
            uid = it["uid"]
            root = by_uid.get(uid)
            if uid in old and old[uid] is not root:
                self._delete_tree(old[uid])
            if root is None:
                continue
            root["built_root"] = True
            root["built_fp"] = it.get("fp", "")
            root.name = it.get("name") or root.name
            root.matrix_world = Matrix(convert.rows(it["matrix"]))

    def _share_materials(self, objects, before):
        known = {m.get("built_material"): m for m in before if m.get("built_material")}
        for o in objects:
            for slot in getattr(o, "material_slots", []):
                m = slot.material
                if m is not None and m not in before and m.get("built_material") in known:
                    slot.material = known[m["built_material"]]
        for m in [m for m in bpy.data.materials if m not in before]:
            name = m.get("built_material")
            if name in known and m.users == 0:
                bpy.data.materials.remove(m)
            elif name and name not in known:
                known[name] = m

    def _delete_tree(self, root):
        meshes = set()
        for o in reversed(_subtree(root)):
            if o.type == "MESH" and o.data is not None:
                meshes.add(o.data)
            bpy.data.objects.remove(o, do_unlink=True)
        for mesh in meshes:
            if mesh.users == 0:
                bpy.data.meshes.remove(mesh)

    def remove(self, uid):
        root = self.roots().get(uid)
        if root is not None:
            self._delete_tree(root)

    # ------------------------------------------------------------ records

    def assign_ids(self):
        """Give lights made in Blender an id of their own (not from a
        panel's draw: Blender forbids writes there)."""
        for o in bpy.context.scene.objects:
            if o.type == "LIGHT" and "built_light" not in o:
                o["built_light"] = uuid.uuid4().hex[:12]

    def keys(self, prefix):
        return [k for k in self._all_records()[0] if k.startswith(prefix)]

    def records(self):
        return self._all_records()

    def _all_records(self):
        records, labels = {}, {}
        for uid, o in self.roots().items():
            key = "transform:" + uid
            records[key] = {"matrix": convert.flat(_rows(o))}
            labels[key] = o.name
        for m in bpy.data.materials:
            name = m.get("built_material")
            if name:
                key = "material:" + name
                records[key] = self._material(m)
                labels[key] = name
        for m, assign in self._new_materials().items():
            key = "newmaterial:" + m.name
            records[key] = self._new_material(m, assign)
            labels[key] = m.name
        for o in bpy.context.scene.objects:
            if o.type == "LIGHT":
                key = "light:" + o.get("built_light", "new-" + o.name)
                records[key] = self._light(o)
                labels[key] = o.name
            elif o.type == "CAMERA":
                name = o.get("built_camera") or o.name
                key = "camera:" + name
                if key not in records:
                    records[key] = self._camera(o)
                    labels[key] = name
        return records, labels

    def record(self, key):
        kind, name = key.split(":", 1)
        if kind == "transform":
            o = self.roots().get(name)
            return {"matrix": convert.flat(_rows(o))} if o else None
        if kind == "material":
            m = self._find_material(name)
            return self._material(m) if m else None
        if kind == "newmaterial":
            m = bpy.data.materials.get(name)
            assign = self._new_materials().get(m) if m is not None else None
            return self._new_material(m, assign) if assign else None
        if kind == "light":
            o = self._find("built_light", name)
            return self._light(o) if o else None
        if kind == "camera":
            o = self._find("built_camera", name) or bpy.context.scene.objects.get(name)
            return self._camera(o) if o and o.type == "CAMERA" else None
        return None

    def _find(self, prop, value):
        return next((o for o in bpy.context.scene.objects if o.get(prop) == value), None)

    def _find_material(self, name):
        return next((m for m in bpy.data.materials if m.get("built_material") == name), None)

    def _image_record(self, image):
        name = os.path.basename(bpy.path.abspath(image.filepath)) if image.filepath else ""
        return {"name": name or image.name, "crc": _image_crc(image)}

    def _material(self, m):
        """A material Built sent, read back in the link's terms (what the
        importer's graph hides, such as packed maps, reads as nothing)."""
        value, _, _ = pbr.extract(m, self._image_record, lenient=True)
        value.pop("rot", None)
        return value

    def _new_materials(self):
        """Materials Built does not know on Built's objects, with where
        they sit: ``{material: [{"uid", "from", "whole"}]}``. ``from`` is
        the Built material the slot started as; ``whole`` when the
        material covers the object."""
        coll = bpy.data.collections.get(MODEL)
        found = {}
        if coll is None:
            return found
        for o in coll.all_objects:
            if o.type != "MESH" or "built_uid" not in o or not o.material_slots:
                continue
            slots = o.get("built_slots", "").split(SEP) if o.get("built_slots") else []
            indices = _material_indices(o.data)
            for i, slot in enumerate(o.material_slots):
                m = slot.material
                if m is None or m.get("built_material") or m.get("built_paint"):
                    continue
                used = sum(1 for x in indices if x == i)
                if not used and len(o.material_slots) > 1:
                    continue
                whole = used == len(indices) or len(o.material_slots) == 1
                found.setdefault(m, []).append({"uid": o["built_uid"], "from": slots[i] if i < len(slots) else "",
                                                "whole": whole})
        return found

    def _new_material(self, m, assign):
        value, _, reason = pbr.extract(m, self._image_record)
        if reason:
            return {"unsupported": reason}
        value["name"] = m.name
        value["assign"] = assign
        if any(not a["whole"] and not a["from"] for a in assign):
            value["assign_note"] = "per-face use goes on the whole placement"
        return value

    def unsupported_materials(self):
        """(name, reason) of the materials that stay in Blender."""
        out = []
        for m in self._new_materials():
            _, _, reason = pbr.extract(m, self._image_record)
            if reason:
                out.append((m.name, reason))
        return sorted(out)

    def adopt(self, name, built_name):
        """Blender's material ``name`` is now Built's ``built_name``."""
        m = bpy.data.materials.get(name)
        if m is not None:
            m["built_material"] = built_name

    def _light(self, o):
        d = o.data
        return convert.light_value(d.type, _rows(o), d.color, d.energy, getattr(d, "spot_size", 0.0),
                                   o.name, not o.hide_render)

    def _camera(self, o):
        d = o.data
        w, h = _render_size()
        fov = convert.vertical_fov(d.sensor_fit, d.angle_x, d.angle_y, w, h)
        return convert.camera_value(_rows(o), o.get("built_distance", 10.0), fov, d.type != "ORTHO")

    def image_bytes(self, key, field):
        kind, name = key.split(":", 1)
        m = self._find_material(name) if kind == "material" else bpy.data.materials.get(name)
        if m is None:
            return None
        _, images, _ = pbr.extract(m, self._image_record, lenient=True)
        image = images.get(field)
        return _image_data(image) if image is not None else None

    # ------------------------------------------------------------ apply Built's values

    def apply(self, key, value):
        kind, name = key.split(":", 1)
        if kind == "transform":
            o = self.roots().get(name)
            if o is not None:
                o.matrix_world = Matrix(convert.rows(value["matrix"]))
        elif kind == "material":
            m = self._find_material(name)
            if m is not None:
                self._set_material(m, value)
        elif kind == "light":
            self._set_light(name, value)
        elif kind == "camera":
            self._set_camera(name, value)

    def _set_material(self, m, value):
        bsdf = _bsdf(m)
        if bsdf is None:
            return
        base = bsdf.inputs["Base Color"]
        if value.get("color") is not None and not base.is_linked:
            base.default_value = (*value["color"][:3], 1.0)
        for k, socket in (("roughness", "Roughness"), ("metallic", "Metallic")):
            s = bsdf.inputs.get(socket)
            if s is not None and not s.is_linked and value.get(k) is not None:
                s.default_value = float(value[k])
        alpha = bsdf.inputs.get("Alpha")
        if alpha is not None and not alpha.is_linked and value.get("opacity") is not None:
            alpha.default_value = float(value["opacity"])
        glow = bsdf.inputs.get("Emission Color") or bsdf.inputs.get("Emission")
        power = bsdf.inputs.get("Emission Strength")
        if power is not None and not power.is_linked and value.get("emission_strength") is not None:
            power.default_value = float(value["emission_strength"])
            if glow is not None and not glow.is_linked and value["emission_strength"] > 0:
                glow.default_value = (*value.get("emission", (0.0, 0.0, 0.0))[:3], 1.0)
        normal = bsdf.inputs.get("Normal")
        if normal is not None and normal.is_linked and value.get("normal_scale") is not None:
            node = normal.links[0].from_node
            strength = node.inputs.get("Strength") if node.type == "NORMAL_MAP" else None
            if strength is not None and not strength.is_linked:
                strength.default_value = float(value["normal_scale"])
        tex = value.get("texture")
        image = _image_of(m)
        if tex and image is not None and tex.get("path") and os.path.isfile(tex["path"]) \
                and _image_crc(image) != tex.get("crc"):
            image.filepath = tex["path"]
            image.reload()

    def _set_light(self, ident, value):
        o = self._find("built_light", ident)
        kind = "SPOT" if value.get("kind") == "spot" else "POINT"
        if o is None or o.data.type != kind:
            if o is not None:
                bpy.data.objects.remove(o, do_unlink=True)
            data = bpy.data.lights.new(value.get("name") or "Built light", type=kind)
            o = bpy.data.objects.new(value.get("name") or "Built light", data)
            _collection(LIGHTS).objects.link(o)
            o["built_light"] = ident
        d = o.data
        d.color = value.get("color", (1.0, 1.0, 1.0))[:3]
        d.energy = float(value.get("power", 400.0))
        if kind == "SPOT":
            d.spot_size = math.radians(float(value.get("angle", 60.0)))
        if value.get("name"):
            o.name = value["name"]
        o.hide_render = not value.get("on", True)
        o.matrix_world = Matrix(convert.light_pose(value))

    def _set_camera(self, name, value):
        o = self._find("built_camera", name)
        if o is None:
            data = bpy.data.cameras.new(name)
            o = bpy.data.objects.new(name, data)
            _collection(CAMERAS).objects.link(o)
            o["built_camera"] = name
        pose, distance = convert.camera_pose(value)
        o["built_distance"] = distance
        o.data.type = "PERSP" if value.get("perspective", True) else "ORTHO"
        o.data.sensor_fit = "VERTICAL"
        o.data.angle_y = math.radians(float(value.get("fov_deg", 45.0)))
        o.matrix_world = Matrix(pose)

    def delete(self, key):
        kind, name = key.split(":", 1)
        prop = {"light": "built_light", "camera": "built_camera"}.get(kind)
        o = self._find(prop, name) if prop else None
        if o is not None:
            bpy.data.objects.remove(o, do_unlink=True)
