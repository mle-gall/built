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
