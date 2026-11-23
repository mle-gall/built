# SPDX-License-Identifier: GPL-3.0-or-later
"""Registry paint reference until document-suite verification."""
from __future__ import annotations
from core.mesh import element_identity
from core.history_commands import MeshCommand, _dirty_group_chunks

class RestampMaterialCommand(MeshCommand):
    """Edit a registry material and RESTAMP every face that wears it.

    "Change the concrete everywhere": the registry entry is replaced by
    ``new_material`` (same name) and each face across the loose mesh and
    every group whose ``attrs["mat"]`` carries that name receives the new
    recipe — colour/texture/opacity — in one undoable step. Keys absent
    from the new recipe are removed (a material edited from textured to
    plain colour drops its texture). Groups and components painted WHOLE
    wear it too, on the container (issue #155, @fafecm: the loose face
    changed colour, the painted box beside it stayed the old one). Undo
    restores the registry entry, each face's previous values and each
    container's previous paint exactly.

    A ``new_material`` with ANOTHER name renames as it restamps: the old
    entry leaves the registry, the new one takes its place and every face
    and container now says the new name. Only the model changes — a
    library colour renamed here stays in the library as it was."""

    def __init__(self, name, new_material) -> None:
        self._name = name
        self._new = new_material
        self._old_material = None
        self._old_faces: Optional[list] = None   # (face, {key: old value})
        self._old_backs = []
        self._old_groups: Optional[list] = None  # (container, old material)
        self._old_target = None   # a renamed-onto entry, put back by undo
        self._meshes: Optional[list] = None      # the ones restamped

    _KEYS = ("mat", "color", "texture", "opacity", "pbr")

    _include_backs = True

    def _affected_faces(self, scene):
        if self._old_faces is None:
            faces = self._targets(scene, capture_backs=self._include_backs)
            self._old_faces = [(face, {k: face.attrs.get(k) for k in self._KEYS})
                               for face in faces]
        return self._old_faces

    def geometry_meshes(self, scene):
        from core.purge import iter_meshes
        return list(iter_meshes(scene))

    def doc_after(self, scene, phase):
        self._count_meshes(scene)
        _dirty_group_chunks(scene)
        scene.version += 1

    def _targets(self, scene, *, capture_backs=False):
        # Nested placements included: walking scene.groups as a flat list
        # left every face INSIDE a component wearing the old recipe, so
        # "change the concrete everywhere" changed it almost nowhere in an
        # imported model. Sibling instances share one prototype mesh, hence
        # the identity dedupe.
        from core.group import iter_placements
        seen: set = set()
        meshes = [scene.loose_mesh]
        for g in scene.groups:
            for pg, _m in iter_placements(g):
                if id(pg.mesh) not in seen:
                    seen.add(id(pg.mesh))
                    meshes.append(pg.mesh)
        for mesh in meshes:
            if mesh is None:
                continue
            for f in mesh.faces:
                if capture_backs:
                    back = f.attrs.get("back")
                    if isinstance(back, dict) and back.get("mat") == self._name:
                        self._old_backs.append((f, back))
                if f.attrs.get("mat") == self._name:
                    yield f

    def _count_meshes(self, scene) -> None:
        """Bump the serial of the meshes whose faces were restamped: an
        attrs swap bumps none by itself, and the viewport's placement caches
        would keep drawing every closed component in the old recipe."""
        if self._meshes is None:
            from core.group import iter_placements
            worn = {element_identity(f) for f, _old in self._old_faces or []}
            worn.update(element_identity(f) for f, _ in self._old_backs)
            found: dict = {}
            meshes = [scene.loose_mesh] + [pg.mesh for g in scene.groups
                                           for pg, _m in iter_placements(g)]
            for mesh in meshes:
                if (mesh is not None and id(mesh) not in found
                        and any(element_identity(f) in worn for f in mesh.faces)):
                    found[id(mesh)] = mesh
            self._meshes = list(found.values())
        for mesh in self._meshes:
            mesh._mut_serial += 1

    def _painted_groups(self, scene):
        """Every placement painted as a whole with this material, nested
        ones included. Each placement carries its own paint, so no dedupe
        by prototype here."""
        from core.group import iter_placements
        seen: set = set()
        for g in scene.groups:
            for pg, _m in iter_placements(g):
                paint = getattr(pg, "material", None)
                if (id(pg) not in seen and isinstance(paint, dict)
                        and paint.get("mat") == self._name):
                    seen.add(id(pg))
                    yield pg

    def do(self, scene) -> None:
        stamp = self._new.face_attrs()
        if self._old_groups is None:
            self._old_material = scene.materials.get(self._name)
            self._affected_faces(scene)
            self._old_groups = [(g, g.material)
                                for g in self._painted_groups(scene)]
            self._old_target = scene.materials.get(self._new.name)
        if self._new.name != self._name:
            scene.materials.pop(self._name, None)
        scene.materials[self._new.name] = self._new
        keep_mapping = (self._old_material is not None
                        and self._old_material.texture == self._new.texture)

        def apply(attrs):
            for key in self._KEYS:
                if key == "texture" and keep_mapping:
                    continue
                if key in stamp:
                    attrs[key] = stamp[key]
                else:
                    attrs.pop(key, None)

        for face, _ in self._old_faces:
            apply(face.attrs)
        for face, old in self._old_backs:
            back = dict(old)
            apply(back)
            face.attrs["back"] = back
        for g, _old in self._old_groups or []:
            g.material = dict(stamp)
        self._count_meshes(scene)
        _dirty_group_chunks(scene)
        scene.version += 1

    def undo(self, scene) -> None:
        if self._new.name != self._name:
            if self._old_target is None:
                scene.materials.pop(self._new.name, None)
            else:
                scene.materials[self._new.name] = self._old_target
        if self._old_material is None:
            scene.materials.pop(self._name, None)
        else:
            scene.materials[self._name] = self._old_material
        for f, old in self._old_faces or []:
            for k in self._KEYS:
                if old[k] is None:
                    f.attrs.pop(k, None)
                else:
                    f.attrs[k] = old[k]
        for face, old in self._old_backs:
            face.attrs["back"] = old
        for g, old in self._old_groups or []:
            g.material = old
        self._count_meshes(scene)
        _dirty_group_chunks(scene)
        scene.version += 1


class EditTexturedMaterialCommand(RestampMaterialCommand):
    """Edit a TEXTURED registry material — its colour, its tile size —
    and carry the edit to every face, group and component that wears it.

    A YouTube review of 0.6.1 (#1308): «when you edit an already applied
    material, say change the colour, it does not affect the applied
    materials in the model, almost as if it creates another material
    definition». It did: the Materials panel's Apply changed the texture
    to paint with next and the faces SELECTED, so the model kept the old
    look, and with one face selected the same name came out in two
    colours. Unlike a restamp, the edit is applied to each face's own
    texture: a texture placed by hand (``uvw``), continued round a curved
    surface or turned on another wall keeps its place; only the colour —
    and the tile size and turn, when ``size`` is given — change.

    ``tint`` is an RGB (0-1) or ``None`` to remove the colour; ``size`` is
    ``(sw, sh, rot)`` or ``None`` to leave each face's own. Undo is the
    restamp's: the registry entry, each face's and each container's paint
    exactly as they were."""

    _include_backs = False

    def __init__(self, name, tint, tint_mode: int = 0, size=None) -> None:
        super().__init__(name, None)
        self._tint = tuple(tint) if tint is not None else None
        self._tint_mode = int(tint_mode or 0)
        self._size = tuple(size) if size is not None else None

    def edited(self, tex):
        """*tex* with this edit: the colour (from its untinted image) and,
        when given, the size and turn."""
        if not tex:
            return tex
        from core.texture import tinted_texture, untinted_texture
        out = dict(tex)
        if self._size is not None:
            out["sw"], out["sh"], out["rot"] = (float(v) for v in self._size)
        if self._tint is None:
            return untinted_texture(out)
        return tinted_texture(out, self._tint, self._tint_mode)

    def do(self, scene) -> None:
        from core.materials import Material
        old = scene.materials.get(self._name)
        if old is None or not old.texture:
            return
        if self._old_groups is None:
            self._old_material = old
            self._affected_faces(scene)
            self._old_groups = [(g, g.material)
                                for g in self._painted_groups(scene)]
            self._new = Material(self._name, color=old.color,
                                 texture=self.edited(old.texture),
                                 opacity=old.opacity, finish=old.finish, pbr=old.pbr)
        scene.materials[self._name] = self._new
        for f, before in self._old_faces:
            if before["texture"]:
                f.attrs["texture"] = self.edited(before["texture"])
        for g, before in self._old_groups or []:
            if isinstance(before, dict) and before.get("texture"):
                g.material = {**before,
                              "texture": self.edited(before["texture"])}
        self._count_meshes(scene)
        _dirty_group_chunks(scene)
        scene.version += 1

    def undo(self, scene) -> None:
        if self._old_faces is None:
            return
        # The name never changes: the restamp's undo, without its renaming.
        scene.materials[self._name] = self._old_material
        for f, old in self._old_faces:
            for k in self._KEYS:
                if old[k] is None:
                    f.attrs.pop(k, None)
                else:
                    f.attrs[k] = old[k]
        for g, old in self._old_groups or []:
            g.material = old
        self._count_meshes(scene)
        _dirty_group_chunks(scene)
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
        if self._old_groups is None:
            self._old_material = scene.materials.get(self._name)
            if self._old_material is None:
                raise KeyError(f"no material called {self._name!r}")
            self._affected_faces(scene)
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
