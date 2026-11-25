# SPDX-License-Identifier: GPL-3.0-or-later
"""``Scene.revision`` must move exactly when the Python walk at ``6977fb85``
moved it: seeded scenes, seeded edits of every field the walk compared
(a placement's uid, name, tag, custom attributes edited in place and
assigned, paint, Hide, lock, billboard, component flag, axes, matrix, its
mesh's revision, children added and removed, kept definitions), the tags
and their folder tables, ``content_version``, and reads with nothing
changed in between. ``tests/fixtures/scene_revision_golden.json`` holds the
revision read after each step. Regenerate only on purpose:
``python tests/test_scene_revision_golden.py --write``."""
from __future__ import annotations

import json
import random
import sys
from pathlib import Path

import pytest

GOLDEN = Path(__file__).parent / "fixtures" / "scene_revision_golden.json"
CASES = 30


def run(seed):
    from ingetrazo_core import Matrix4, Vec3
    from core.group import Group
    from core.layers import Layer
    from core.mesh import Mesh
    from core.scene import Scene
    rng = random.Random(seed)
    s = Scene()
    groups = []

    def mesh():
        m = Mesh()
        m.add_face([Vec3(0, 0, 0), Vec3(1, 0, 0), Vec3(1, 1, 0)])
        return m
    for i in range(rng.randint(2, 6)):
        g = Group(mesh(), name=f"g{i}", uid=f"u{seed}-{i}")
        if rng.random() < 0.5:
            g.xform = Matrix4()
        s.groups.append(g)
        groups.append(g)
        if rng.random() < 0.5:
            c = Group(mesh(), name=f"c{i}", uid=f"c{seed}-{i}")
            c.xform = Matrix4()
            g.adopt([c])
            groups.append(c)
    out = [s.revision]

    def step(label):
        out.append([label, s.revision])
    for n in range(rng.randint(15, 30)):
        g = rng.choice(groups)
        op = rng.choice(["read", "read", "name", "uid", "layer", "ext", "ext_inplace", "ifc",
                         "material", "hidden", "locked", "billboard", "component", "axes",
                         "xform", "xform_inplace", "mesh", "child", "unchild", "kept", "tag",
                         "tag_inplace", "folder", "version", "text3d", "same_name"])
        if op == "name":
            g.name = f"n{n}"
        elif op == "same_name":
            g.name = g.name
        elif op == "uid":
            g.uid = f"x{seed}-{n}"
        elif op == "layer":
            g.layer = rng.choice([None, "A", "B"])
        elif op == "ext":
            g.ext = {"k": n, "l": [n, 1.5]}
        elif op == "ext_inplace":
            if not g.ext:
                g.ext = {"k": 0}
            g.ext["k"] = rng.choice([0, n, -0.0, 0.0])
        elif op == "ifc":
            g.ifc = rng.choice([None, {"class": "IfcWall"}, {"class": "IfcSlab"}])
        elif op == "material":
            g.material = rng.choice([None, {"color": [0.1, 0.2, 0.3]}, {"color": [0.1, 0.2, 0.4]}])
        elif op == "hidden":
            g.hidden = not g.hidden
        elif op == "locked":
            g.locked = not g.locked
        elif op == "billboard":
            g.billboard = rng.choice([False, True])
        elif op == "component":
            g.component = not g.component
        elif op == "axes":
            a = Matrix4()
            a.rotate(rng.choice([0.0, 30.0]), 0, 0, 1)
            g.axes = a
        elif op == "xform":
            m = Matrix4()
            m.translate(rng.choice([0.0, 1.0]), 0, 0)
            g.xform = m
        elif op == "xform_inplace" and g.xform is not None:
            g.xform.translate(0.5, 0, 0)
        elif op == "mesh":
            g.mesh.add_face([Vec3(5, 0, 0), Vec3(6, 0, 0), Vec3(6, 1, 0)])
        elif op == "child":
            c = Group(mesh(), name="new", uid=f"k{seed}-{n}")
            c.xform = Matrix4()
            g.adopt(list(g.children) + [c])
            groups.append(c)
        elif op == "unchild" and g.children:
            g.children = list(g.children)[:-1]
        elif op == "kept":
            k = Group(mesh(), name="kept", uid=f"kept{seed}-{n}")
            k.xform = Matrix4()
            s.kept_components.append(k)
        elif op == "tag":
            s.layers.append(Layer(f"T{n}"))
        elif op == "tag_inplace" and len(s.layers) > 1:
            s.layers[-1].visible = not s.layers[-1].visible
        elif op == "folder":
            s.folder_parents = {"F": rng.choice(["G", "H"])}
        elif op == "version":
            s.version += 1
        elif op == "text3d":
            g.text3d = rng.choice([None, {"text": "a"}])
        step(op)
    return out


def generate():
    return [run(seed) for seed in range(CASES)]


@pytest.mark.parametrize("seed", range(CASES))
def test_revision_moves_as_the_python_did(seed):
    want = json.loads(GOLDEN.read_text())[seed]
    assert run(seed) == want


if __name__ == "__main__":
    if "--write" in sys.argv:
        GOLDEN.write_text(json.dumps(generate()))
        print("wrote", GOLDEN)
