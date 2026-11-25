# SPDX-License-Identifier: GPL-3.0-or-later
"""``Scene.clear()`` must leave a scene as the Python at ``53a4ceff`` left
it: seeded scenes with some of what it resets filled in (geometry, groups,
dimensions, labels, paths, guides, scenes, folders, materials, plugin
data, links, tags and their folders, the active tag, line types, flags,
the north angle, the cameras, units) or nothing at all, an open group
edit among them. Recorded per case: every field ``clear`` touches, the
version moved or not. ``tests/fixtures/scene_clear_golden.json``;
regenerate only on purpose: ``python tests/test_scene_clear_golden.py --write``."""
from __future__ import annotations

import json
import random
import sys
from pathlib import Path

import pytest

GOLDEN = Path(__file__).parent / "fixtures" / "scene_clear_golden.json"
CASES = 40
FIELDS = ("groups", "dimensions", "text_labels", "text_placement_anchors", "geo_paths",
          "geo_points", "guides", "image_planes", "saved_views", "scene_folders",
          "scene_folder_parents", "compositions", "custom_scales", "kept_components",
          "plugin_data", "link_registry", "active_layer", "layer_folders",
          "folder_parents", "folder_visibility", "layer_folder_uids", "layer_folder_names",
          "hatch_patterns", "line_types", "linetype_colors", "bricks", "georef", "tile_layer",
          "terrain", "back_face_color", "active_ifc", "section_planes", "show_section_planes",
          "show_section_cuts", "show_hidden_objects", "show_hidden_geometry", "show_axes",
          "north_deg", "camera_home", "views_home", "units")


def state(s):
    out = {name: repr(getattr(s, name, None)) for name in FIELDS}
    out["layers"] = [(ly.key, ly.name) for ly in s.layers]
    out["mesh"] = (len(s.mesh.faces), len(s.mesh.edges))
    out["materials"] = sorted(s.materials)
    out["selection"] = len(s.selection)
    out["style"] = type(s.display_style).__name__
    out["animation"] = repr(s.animation.to_dict())
    return out


def run(seed):
    from built_core import Vec3 as V
    from core.group import Group
    from core.layers import Layer
    from core.materials import Material
    from core.mesh import Mesh
    from core.scene import Scene
    rng = random.Random(seed)
    s = Scene()
    pick = rng.random
    if pick() < 0.5:
        f = s.mesh.add_face([V(0, 0, 0), V(1, 0, 0), V(1, 1, 0)])
        if pick() < 0.5:
            s.selection.add(f)
    if pick() < 0.4:
        g = Group(Mesh(), name="g")
        s.groups.append(g)
        if pick() < 0.3:
            s.begin_group_edit(g)
    if pick() < 0.3:
        s.layers.append(Layer("Wood"))
    if pick() < 0.2:
        s.active_layer = "Layer 0"
    if pick() < 0.3:
        s.layer_folders = ["F"]
    if pick() < 0.3:
        s.materials["Oak"] = Material("Oak", color=(0.5, 0.4, 0.3))
    if pick() < 0.3:
        s.plugin_data = {"x": 1}
    if pick() < 0.3:
        s.link_registry = {"/a.igz": {}}
    if pick() < 0.2:
        s.custom_scales.append("1:33")
    if pick() < 0.2:
        s.show_section_cuts = False
    if pick() < 0.2:
        s.show_hidden_objects = True
    if pick() < 0.2:
        s.north_deg = 12.0
    if pick() < 0.2:
        s.back_face_color = (0.1, 0.2, 0.3)
    if pick() < 0.3:
        s.camera_home = {"yaw": 1.0}
    if pick() < 0.3:
        s.units = {"length": "mm", "precision": 0}
    if pick() < 0.2:
        s.scene_folders = ["S"]
    if pick() < 0.2:
        s.line_types = {"dash": [1.0, -0.5]}
    v = s.version
    s.clear()
    out = state(s)
    out["moved"] = s.version != v
    out["edit_open"] = s.edit_group is not None
    return json.loads(json.dumps(out, default=repr))


@pytest.mark.parametrize("seed", range(CASES))
def test_clear_leaves_what_the_python_left(seed):
    assert run(seed) == json.loads(GOLDEN.read_text())[seed]


if __name__ == "__main__":
    if "--write" in sys.argv:
        GOLDEN.write_text(json.dumps([run(seed) for seed in range(CASES)], indent=0))
        print("wrote", GOLDEN)
