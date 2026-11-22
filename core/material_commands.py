# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""History commands the Library needs on the material registry that the
Materials panel never had: add a material without painting it (Duplicate),
and delete one the model wears (its faces and containers go back to the
default material). Rename and edit reuse ``RestampMaterialCommand`` /
``EditTexturedMaterialCommand``; these sit beside them, one step each."""
from __future__ import annotations

from core.history import Command, RestampMaterialCommand, _dirty_group_chunks


class AddMaterialCommand(Command):
    """Put ``material`` in the registry (nothing wears it yet)."""

    doc_undo = True     # the registry is in the document (slice 2)

    def __init__(self, material) -> None:
        self.material = material

    def do(self, scene) -> None:
        if self.material.name in scene.materials:
            raise ValueError(f"there is already a material called {self.material.name!r}")
        scene.materials[self.material.name] = self.material
        scene.version += 1

    def undo(self, scene) -> None:
        scene.materials.pop(self.material.name, None)
        scene.version += 1


class DeleteMaterialCommand(RestampMaterialCommand):
    """Remove the registry material ``name``: every face (front or back) and
    every container painted with it loses the paint and shows the default
    material. Undo is the restamp's: entry, faces and containers exactly as
    they were."""

    def __init__(self, name) -> None:
        from core.materials import Material
        super().__init__(name, Material(name))

    def do(self, scene) -> None:
        if self._old_faces is None:
            self._old_material = scene.materials.get(self._name)
            if self._old_material is None:
                raise KeyError(f"no material called {self._name!r}")
            self._old_faces = [(f, {k: f.attrs.get(k) for k in self._KEYS})
                               for f in self._targets(scene, capture_backs=True)]
            self._old_groups = [(g, g.material) for g in self._painted_groups(scene)]
        scene.materials.pop(self._name, None)
        for face, _ in self._old_faces:
            for key in self._KEYS:
                face.attrs.pop(key, None)
        for face, old in self._old_backs:
            rest = {k: v for k, v in old.items() if k not in self._KEYS}
            if rest:
                face.attrs["back"] = rest
            else:
                face.attrs.pop("back", None)
        for g, _old in self._old_groups:
            g.material = None
        self._count_meshes(scene)
        _dirty_group_chunks(scene)
        scene.version += 1


# ---- one kind of resource: every paint is a named material -------------------

#: ``Material.pbr`` channels (contract: docs/skp-pbr-format.md, "Mapping to
#: Material.pbr"). Maps are ``{"path", "flip_y"?}`` dicts.
PBR_VALUES = ("metallic", "roughness", "normal_scale", "ao_strength")
PBR_MAPS = ("metallic_map", "roughness_map", "normal_map", "ao_map")
PBR_KEYS = PBR_VALUES + PBR_MAPS


def colour_hex(rgb) -> str:
    return "#%02X%02X%02X" % tuple(int(round(max(0.0, min(1.0, float(c))) * 255)) for c in rgb[:3])


def colour_name(rgb) -> str:
    """The name a raw colour gets as a material: ``Colour #A0703C``."""
    return f"Colour {colour_hex(rgb)}"


def _same_colour(a, b) -> bool:
    return a is not None and b is not None and all(abs(float(x) - float(y)) < 0.5 / 255 for x, y in zip(a[:3], b[:3]))


def _bare_texture(texture):
    if not texture:
        return None
    tex = dict(texture)
    tex.pop("uvw", None)
    return tex


def find_material(materials, color=None, texture=None, opacity=None):
    """The name of a registry material with this plain recipe (no PBR of
    its own), or ``None``: what a raw colour or image reuses."""
    tex = _bare_texture(texture)
    for name, mat in (materials or {}).items():
        if mat.pbr is not None or mat.opacity != opacity:
            continue
        if tex is not None:
            if mat.texture and _bare_texture(mat.texture) == tex:
                return name
        elif not mat.texture and _same_colour(mat.color, color):
            return name
    return None


def fresh_name(materials, base: str) -> str:
    name, n = base, 2
    while name in (materials or {}):
        name, n = f"{base} ({n})", n + 1
    return name


def material_for_paint(scene, color=None, texture=None, opacity=None):
    """No anonymous paint: the Material a raw colour (or an image picked
    without a name) paints as. An existing material with the same recipe
    is reused; otherwise a fresh one, named after the colour or the image
    — not registered here: the paint stroke's tag command registers it in
    the same undo step, so merely picking a colour adds nothing."""
    from pathlib import Path
    from core.materials import Material
    materials = getattr(scene, "materials", None) or {}
    found = find_material(materials, color=color, texture=texture, opacity=opacity)
    if found is not None:
        return materials[found]
    tex = _bare_texture(texture)
    if tex is not None:
        base = Path(str(tex.get("path", "Image"))).stem.strip() or "Image"
        return Material(fresh_name(materials, base), texture=tex, opacity=opacity)
    rgb = tuple(float(c) for c in (color or (0.8, 0.8, 0.8))[:3])
    return Material(fresh_name(materials, colour_name(rgb)), color=rgb, opacity=opacity)


def unnamed_colours(scene) -> dict:
    """Paint from older files with no identity: faces (front or back) and
    containers carrying a colour but no ``mat`` and no image. ``{hex:
    {"color", "faces": [(face, side)], "groups": [group]}}`` per distinct
    colour (opacity kept apart). Read only; files are never rewritten on
    open."""
    from core.group import iter_placements
    from core.purge import _document_groups, iter_meshes
    out: dict = {}

    def slot(rgb, opacity):
        key = colour_hex(rgb) + ("" if opacity is None else f"@{float(opacity):.2f}")
        return out.setdefault(key, {"color": tuple(float(c) for c in rgb[:3]), "opacity": opacity,
                                    "faces": [], "groups": []})

    def anonymous(attrs):
        return (isinstance(attrs, dict) and attrs.get("color") is not None and not attrs.get("mat")
                and not attrs.get("texture"))

    for mesh in iter_meshes(scene):
        for f in mesh.faces:
            if anonymous(f.attrs):
                slot(f.attrs["color"], f.attrs.get("opacity"))["faces"].append((f, "front"))
            back = f.attrs.get("back")
            if anonymous(back):
                slot(back["color"], back.get("opacity"))["faces"].append((f, "back"))
    for top in _document_groups(scene):
        for g, _m in iter_placements(top):
            if anonymous(getattr(g, "material", None)):
                paint = g.material
                slot(paint["color"], paint.get("opacity"))["groups"].append(g)
    return out


class NameColoursCommand(Command):
    """"Name these": each distinct unnamed colour becomes a registry
    material (reusing one with the same recipe) and every face side and
    container that carried it takes its name. Nothing else changes; undo
    puts names and registry back."""

    def __init__(self) -> None:
        self._plan = None          # [(name, Material | None, [(face, side)], [group])]
        self._added: list = []

    def _survey(self, scene) -> None:
        from core.materials import Material
        materials = dict(getattr(scene, "materials", None) or {})
        self._plan = []
        for entry in unnamed_colours(scene).values():
            found = find_material(materials, color=entry["color"], opacity=entry["opacity"])
            mat = None
            if found is None:
                found = fresh_name(materials, colour_name(entry["color"]))
                mat = Material(found, color=entry["color"], opacity=entry["opacity"])
                materials[found] = mat
            self._plan.append((found, mat, entry["faces"], entry["groups"]))

    def count(self, scene) -> int:
        if self._plan is None:
            self._survey(scene)
        return len(self._plan)

    def do(self, scene) -> None:
        if self._plan is None:
            self._survey(scene)
        self._added = []
        for name, mat, faces, groups in self._plan:
            if mat is not None and name not in scene.materials:
                scene.materials[name] = mat
                self._added.append(name)
            for f, side in faces:
                if side == "front":
                    f.attrs["mat"] = name
                else:
                    f.attrs["back"] = {**f.attrs["back"], "mat": name}
            for g in groups:
                g.material = {**g.material, "mat": name}
        scene.version += 1

    def undo(self, scene) -> None:
        for name, _mat, faces, groups in self._plan or []:
            for f, side in faces:
                if side == "front":
                    f.attrs.pop("mat", None)
                elif isinstance(f.attrs.get("back"), dict):
                    f.attrs["back"] = {k: v for k, v in f.attrs["back"].items() if k != "mat"}
            for g in groups:
                if isinstance(g.material, dict):
                    g.material = {k: v for k, v in g.material.items() if k != "mat"}
        for name in self._added:
            scene.materials.pop(name, None)
        self._added = []
        scene.version += 1
