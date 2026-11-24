# SPDX-License-Identifier: GPL-3.0-or-later
"""glTF 2.0 and GLB import. Geometry is staged on the document worker."""
from ingetrazo_core import (
    gltf_build_mesh as build_mesh,
    gltf_guess_scale as _guess_scale,
    gltf_load as load_glb,
    gltf_node_transforms as _node_transforms,
    gltf_parse_container as parse_container,
    gltf_read_triangles as read_triangles,
)
