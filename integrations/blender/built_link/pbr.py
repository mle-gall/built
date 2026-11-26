# SPDX-License-Identifier: GPL-3.0-or-later
"""Blender materials as Built's PBR materials.

A *simple PBR* material is a Principled BSDF connected straight to the
Material Output's Surface, each input a constant or an Image Texture
(optionally through UV Map / Texture Coordinate and Mapping), normals
through a Normal Map node, occlusion from the glTF Material Output group
Blender's glTF importer makes. Anything else (node groups, procedural
textures, mix shaders, math...) stays in Blender.

``extract(material)`` returns ``(value, images, reason)``: ``value`` in
the link's material terms (the same fields Built sends), ``images`` the
Blender images by field, and ``reason`` why the material is not simple
(``value`` then holds what could be read, for materials Built already
knows). Written against the node API only, so tests mock it.
"""
import math

NOT_SUPPORTED = "Node setup not supported"
MAPS = ("normal_map", "roughness_map", "metallic_map", "ao_map", "emission_map")
OCCLUSION_GROUPS = ("glTF Material Output", "glTF Settings")


class Unsupported(Exception):
    pass


def _input(node, *names):
    for name in names:
        socket = node.inputs.get(name)
        if socket is not None:
            return socket
    return None


def _source(socket):
    """The node feeding ``socket`` and the output it uses."""
    link = socket.links[0]
    return link.from_node, getattr(link.from_socket, "name", "")


def _constant(socket):
    v = socket.default_value
    try:
        return [float(c) for c in v]
    except TypeError:
        return float(v)


def _uv(node):
    """An Image Texture's coordinates: None (the UVs), or a Mapping's
    (scale x, scale y, rotation degrees). Anything else is unsupported."""
    vec = node.inputs.get("Vector")
    if vec is None or not vec.is_linked:
        return None
    src, out = _source(vec)
    mapping = None
    if src.type == "MAPPING":
        for name in ("Location", "Rotation", "Scale"):
            s = src.inputs.get(name)
            if s is not None and s.is_linked:
                raise Unsupported(f"{name} of a Mapping is driven by nodes")
        scale = _constant(src.inputs["Scale"]) if src.inputs.get("Scale") is not None else [1.0, 1.0, 1.0]
        rot = _constant(src.inputs["Rotation"]) if src.inputs.get("Rotation") is not None else [0.0, 0.0, 0.0]
        mapping = (scale[0], scale[1], math.degrees(rot[2]))
        inner = src.inputs.get("Vector")
        if inner is None or not inner.is_linked:
            return mapping
        src, out = _source(inner)
    if src.type == "UVMAP" or (src.type == "TEX_COORD" and out == "UV"):
        return mapping
    raise Unsupported("Texture coordinates other than UVs")


def _image(socket, outputs=("Color",)):
    src, out = _source(socket)
    if src.type != "TEX_IMAGE":
        raise Unsupported(NOT_SUPPORTED)
    if src.image is None:
        raise Unsupported("An Image Texture without an image")
    if out not in outputs:
        raise Unsupported(f"The image's {out} output is not supported")
    if getattr(src, "projection", "FLAT") != "FLAT":
        raise Unsupported("Box or sphere projected images")
    return src, _uv(src)


def _output(tree):
    outputs = [n for n in tree.nodes if n.type == "OUTPUT_MATERIAL"]
    active = [n for n in outputs if getattr(n, "is_active_output", True)]
    return (active or outputs or [None])[0]


def _empty():
    return {"color": [0.8, 0.8, 0.8], "texture": None, "tile": None, "roughness": 0.5, "metallic": 0.0,
            "opacity": 1.0, "normal_map": None, "normal_scale": 1.0, "roughness_map": None,
            "metallic_map": None, "ao_map": None, "emission": [0.0, 0.0, 0.0], "emission_strength": 0.0,
            "emission_map": None}


def extract(material, image_record=lambda image: {"name": image.name}, lenient=False):
    """``(value, images, reason)`` for ``material`` (see the module).
    ``lenient``: read what can be read and skip the rest (a material Built
    sent, whose importer graph packs maps) instead of stopping at the
    first unsupported input."""
    value, images = _empty(), {}
    tree = getattr(material, "node_tree", None)
    if not getattr(material, "use_nodes", False) or tree is None:
        colour = list(getattr(material, "diffuse_color", (0.8, 0.8, 0.8, 1.0)))
        value.update(color=[float(c) for c in colour[:3]],
                     roughness=float(getattr(material, "roughness", 0.5)),
                     metallic=float(getattr(material, "metallic", 0.0)))
        return value, images, None
    reasons = []

    def attempt(fn):
        try:
            fn()
        except Unsupported as exc:
            if not lenient:
                raise
            reasons.append(str(exc))

    try:
        out = _output(tree)
        if out is None or not out.inputs["Surface"].is_linked:
            raise Unsupported("Nothing on the Material Output's Surface")
        bsdf, _ = _source(out.inputs["Surface"])
        if bsdf.type != "BSDF_PRINCIPLED":
            raise Unsupported(NOT_SUPPORTED)
        for name in ("Volume", "Displacement"):
            s = out.inputs.get(name)
            if s is not None and s.is_linked:
                raise Unsupported(f"{name} on the Material Output")
        carried = {"Base Color", "Metallic", "Roughness", "Normal", "Alpha", "Emission Color", "Emission",
                   "Emission Strength"}
        for socket in bsdf.inputs:
            if socket.is_linked and socket.name not in carried:
                raise Unsupported(f"{socket.name} is driven by nodes")

        def base():
            s = bsdf.inputs["Base Color"]
            if s.is_linked:
                node, uv = _image(s)
                images["texture"] = node.image
                value["texture"] = image_record(node.image)
                value["color"] = None
                if uv is not None:
                    sx, sy, rot = uv
                    value["tile"] = [1.0 / sx if sx else 1.0, 1.0 / sy if sy else 1.0]
                    if abs(rot) > 1e-6:
                        value["rot"] = rot
                else:
                    value["tile"] = [1.0, 1.0]
            else:
                value["color"] = _constant(s)[:3]

        def scalar(name, field, map_field):
            def run():
                s = bsdf.inputs[name]
                if s.is_linked:
                    node, _ = _image(s)
                    images[map_field] = node.image
                    value[map_field] = image_record(node.image)
                    value[field] = 1.0
                else:
                    value[field] = _constant(s)
            return run

        def alpha():
            s = bsdf.inputs.get("Alpha")
            if s is None:
                return
            if s.is_linked:
                node, out_name = _source(s)
                base_socket = bsdf.inputs["Base Color"]
                same_image = (node.type == "TEX_IMAGE" and out_name == "Alpha" and base_socket.is_linked
                              and _source(base_socket)[0] is node)
                if not same_image:
                    raise Unsupported("An alpha map other than the base image's")
                value["opacity"] = 1.0
            else:
                value["opacity"] = _constant(s)

        def normal():
            s = bsdf.inputs.get("Normal")
            if s is None or not s.is_linked:
                return
            node, _ = _source(s)
            if node.type != "NORMAL_MAP" or getattr(node, "space", "TANGENT") != "TANGENT":
                raise Unsupported("Normals other than a tangent Normal Map")
            strength = node.inputs.get("Strength")
            if strength is not None and strength.is_linked:
                raise Unsupported("Normal strength is driven by nodes")
            colour = node.inputs.get("Color")
            if colour is None or not colour.is_linked:
                return
            img, _ = _image(colour)
            images["normal_map"] = img.image
            value["normal_map"] = image_record(img.image)
            value["normal_scale"] = _constant(strength) if strength is not None else 1.0

        def emission():
            colour = _input(bsdf, "Emission Color", "Emission")
            strength = bsdf.inputs.get("Emission Strength")
            if strength is not None and strength.is_linked:
                raise Unsupported("Emission strength is driven by nodes")
            power = _constant(strength) if strength is not None else 1.0
            if colour is not None and colour.is_linked:
                img, _ = _image(colour)
                images["emission_map"] = img.image
                value["emission_map"] = image_record(img.image)
                rgb = [1.0, 1.0, 1.0]
            else:
                rgb = _constant(colour)[:3] if colour is not None else [0.0, 0.0, 0.0]
            if power <= 0 or not any(c > 0 for c in rgb):
                value["emission"], value["emission_strength"] = [0.0, 0.0, 0.0], 0.0
                value["emission_map"] = None
                images.pop("emission_map", None)
            else:
                value["emission"], value["emission_strength"] = [float(c) for c in rgb], float(power)

        def occlusion():
            for node in tree.nodes:
                if node.type == "GROUP" and getattr(getattr(node, "node_tree", None), "name", "") in OCCLUSION_GROUPS:
                    s = node.inputs.get("Occlusion")
                    if s is not None and s.is_linked:
                        img, _ = _image(s, outputs=("Color", "Alpha"))
                        images["ao_map"] = img.image
                        value["ao_map"] = image_record(img.image)

        for step in (base, scalar("Metallic", "metallic", "metallic_map"),
                     scalar("Roughness", "roughness", "roughness_map"), alpha, normal, emission, occlusion):
            attempt(step)
        if not lenient:
            _no_strays(tree, bsdf, out)
    except Unsupported as exc:
        return value, images, str(exc)
    return value, images, (reasons[0] if reasons else None)


ALLOWED = {"OUTPUT_MATERIAL", "BSDF_PRINCIPLED", "TEX_IMAGE", "MAPPING", "UVMAP", "TEX_COORD", "NORMAL_MAP"}


def _no_strays(tree, bsdf, out):
    """Every node that feeds the output is one the link understands."""
    seen, stack = set(), [out]
    while stack:
        node = stack.pop()
        if id(node) in seen:
            continue
        seen.add(id(node))
        if node.type not in ALLOWED and not (node.type == "GROUP" and getattr(
                getattr(node, "node_tree", None), "name", "") in OCCLUSION_GROUPS):
            raise Unsupported(NOT_SUPPORTED)
        for socket in node.inputs:
            if socket.is_linked:
                stack.append(socket.links[0].from_node)
