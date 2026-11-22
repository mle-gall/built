# SPDX-License-Identifier: GPL-3.0-or-later
"""Snapshot commands for keyed face edits. Shared topology helpers may refuse
an unsupported state. Keep that refusal inside the store transaction."""
from contextlib import contextmanager

from core.history import SnapshotCompound, SnapshotMutation


@contextmanager
def resident_edit(mesh):
    state = mesh.__dict__
    previous = state.get("_resident_edit", False)
    state["_resident_edit"] = True
    try:
        yield
    finally:
        if previous:
            state["_resident_edit"] = previous
        else:
            state.pop("_resident_edit", None)


class _ResidentEdit:
    @property
    def change(self):
        """The net forward edit, including healing, evaluated only when read."""
        return self.after.changes_since(self.before) if self.after is not None else None

    def do(self, scene):
        mesh = self._target(scene) if isinstance(self, SnapshotMutation) else scene.mesh
        with resident_edit(mesh):
            try:
                super().do(scene)
            except Exception:
                if self.before is not None:
                    mesh.restore_state(self.before)
                raise

    def apply(self, scene):
        """Apply directly and return the complete StoreChange. History uses do/undo."""
        self.do(scene)
        return self.change


class StoreFaceCompound(_ResidentEdit, SnapshotCompound):
    pass


class StoreFaceMutation(_ResidentEdit, SnapshotMutation):
    pass
