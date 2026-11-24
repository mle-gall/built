"""Camera-facing resident placements; geometry and document transforms stay fixed.

The measuring and turning are the renderer's (billboards.rs); this keeps
which placements are figures, their unturned matrices and the last camera."""
import numpy as np
from ingetrazo_render import billboard_stances, billboard_worlds, sprite_mapping


def fitted_maps(pos, corners, sizes, counts, maps):
    """Map a sprite's horizontal extent and height to 0..1, on both sides."""
    first = int(sizes[0]) if len(sizes) else 0
    mapping = sprite_mapping(np.asarray(pos, np.float64).reshape(-1, 3),
                             [int(c) for c in corners], first)
    if mapping is not None:
        maps[:] = list(mapping) * 2
    return maps


class Billboards:
    """Cache world-space anchors/normals once per scene edit, O(figures) per orbit."""
    def __init__(self, packed, candidates=None):
        self.packed = packed
        flags = packed.instances['info'][:, 2]
        if candidates is None:
            self.ids = np.flatnonzero(flags & 8)
        else:
            self.ids = candidates[(flags[candidates] & 8) != 0]
        self.base = packed.instances['world'][self.ids].copy()
        self.key = None
        self.entries = []
        if len(self.ids):
            codes = packed.instances['info'][self.ids, 0]
            ranges = [(int(packed.commands[c]['base']), int(packed.commands[c]['count'])) for c in codes]
            self.entries = billboard_stances(np.ascontiguousarray(packed.vertices['pos']), ranges,
                                             np.ascontiguousarray(self.base))
        self._mesh = [packed.groups[i].billboard == 'mesh' for i in self.ids]

    def restore(self):
        self.packed.instances['world'][self.ids] = self.base

    def update(self, camera, renderer=None, inference=None):
        if not len(self.ids):
            return
        eye = camera.eye()
        key = (*eye.toTuple(), *camera.target.toTuple(), camera.perspective)
        if key == self.key:
            return
        self.key = key
        matrices = billboard_worlds(np.ascontiguousarray(self.base), self.entries, self._mesh,
                                    eye.toTuple(), camera.target.toTuple(), bool(camera.perspective))
        self.packed.instances['world'][self.ids] = matrices
        if renderer is not None:
            renderer.set_instance_world(self.ids.tolist(), matrices.reshape(-1))
        if inference is not None:
            inference.update(self.ids, self.packed)
