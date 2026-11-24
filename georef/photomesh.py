# SPDX-License-Identifier: GPL-3.0-or-later
"""Public model types for native georeferenced survey imports."""
from dataclasses import dataclass, field
from functools import partialmethod
from pathlib import Path

from ingetrazo_core import (
    atlas_bytes, find_anchor, load_atlas, load_odm_obj, parse_anchor, parse_mtl, plan_texture_sizes,
    photo_bounds, photo_geodetic, photo_height, photo_index, photo_missing, photo_sample,
    photo_pack as pack_mesh, photo_unpack as unpack_mesh, vertical_origin,
)

VERTICAL_ODM, VERTICAL_DEM, VERTICAL_LOCAL = "odm", "dem", "local"
GEOMETRY_ENTRY = "survey/geometry.npz"
ATLAS_ENTRY = "survey/atlas-{:02d}.jpg"
FOOT_PERCENTILE = 1.0
DEFAULT_TEXTURE_BUDGET = 1024 ** 3


@dataclass(frozen=True)
class ODMAnchor:
    east: float
    north: float
    zone: int
    northern: bool

    @property
    def hemisphere(self):
        return "N" if self.northern else "S"


@dataclass
class PhotoMaterial:
    name: str
    texture: Path | None
    start: int
    count: int


@dataclass(eq=False)
class PhotoMesh:
    vertices: object
    uvs: object
    triangles: object
    materials: list
    source: Path | None = None
    anchor: ODMAnchor | None = None
    visible: bool = field(default=True, init=False)
    images: dict = field(default_factory=dict, init=False)
    vertical_reference: str = field(default=VERTICAL_ODM, init=False)
    layer: str | None = field(default=None, init=False)
    _index: object = field(default=None, init=False, repr=False)

    def invalidate_index(self):
        self._index = None

    @property
    def vertex_count(self):
        return self.vertices.shape[0]

    @property
    def triangle_count(self):
        return self.triangles.shape[0]

    @property
    def georeferenced(self):
        return self.anchor is not None

    missing_textures = property(photo_missing)
    bounds = partialmethod(photo_bounds)
    height_at = partialmethod(photo_height)
    _ensure_index = partialmethod(photo_index)


@dataclass(eq=False)
class PhotoMeshSampler:
    mesh: PhotoMesh
    datum: object

    def ensure_area(self, lat_s, lon_w, lat_n, lon_e):
        return None

    elevation_at_local = partialmethod(photo_sample)
    elevation_at = partialmethod(photo_geodetic)
