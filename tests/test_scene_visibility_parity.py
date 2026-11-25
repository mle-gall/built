# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The scene's visibility rules, pinned for their move to Rust: whatever
replaces them must answer as ``core/scene.py``'s Python did at ``61fe6326``
(a by-call port through PyO3 was tried and was about twice as slow; see
``docs/model-port.md``): for every loose face and edge, every group and
nested placement, a dimension and a guide of seeded scenes — hidden,
visible, selectable, locked, snappable, hidden as a similar component, the
tag's state — at the model, inside an open group (on a tag, in a folder
switched off, painted), inside a component with Hide Similar on, with the
navigator's allowed set, with hidden objects and geometry shown, and with
the loose geometry locked.

Regenerate only on purpose, against the reference implementation:
``python tests/test_scene_visibility_parity.py --write``."""
from __future__ import annotations

import gzip
import json
import random
import sys
from pathlib import Path

GOLDEN = Path(__file__).parent / "fixtures" / "scene_visibility_golden.json.gz"


def _scene(rng):
    from ingetrazo_core import Matrix4 as M, Vec3 as V
    from core import layers as L
    from core.dimension import Dimension
    from core.group import Group
    from core.guide import Guide
    from core.mesh import Mesh
    from core.scene import Scene
    scene = Scene()
    tags = ["A", "B", "C"]
    for t in tags:
        scene.layers.append(L.Layer(t, visible=rng.random() < 0.7, locked=rng.random() < 0.25))
    if rng.random() < 0.6:
        f = __import__("ingetrazo_core").tag_edits.add_folder(scene, "F")
        __import__("ingetrazo_core").tag_edits.move_to_folder(scene, scene.layers[1].name, f)
        scene.folder_visibility = {f: rng.random() < 0.5}
    pick = lambda: rng.choice([None, None] + tags + ["ghost"])  # noqa: E731

    def fill(mesh, at):
        for i in range(rng.randint(1, 3)):
            a = V(at + i * 3, 0, 0)
            f = mesh.add_face([a, a + V(1, 0, 0), a + V(1, 1, 0), a + V(0, 1, 0)])
            t = pick()
            if t:
                f.attrs["layer"] = t
            if rng.random() < 0.25:
                f.attrs["hidden"] = True
        for e in list(mesh.edges):
            r = rng.random()
            if r < 0.2:
                e.layer = pick()
            elif r < 0.35:
                e.hidden = True
        return mesh

    fill(scene.loose_mesh, 0)
    proto = fill(Mesh(), 100)
    for i in range(rng.randint(2, 5)):
        shared = rng.random() < 0.6
        g = Group(proto if shared else fill(Mesh(), 200 + 20 * i), name=f"g{i}")
        g.uid = f"u{i}"
        if shared or rng.random() < 0.5:
            x = M()
            x.translate(10.0 * i, 5.0, 0.0)
            g.xform = x
        g.layer = pick()
        g.hidden = rng.random() < 0.2
        g.locked = rng.random() < 0.2
        if rng.random() < 0.3:
            g.material = "Brick"
        if rng.random() < 0.5:
            c = Group(proto if rng.random() < 0.5 else fill(Mesh(), 400), name=f"c{i}")
            c.uid = f"u{i}c"
            y = M()
            y.translate(1.0, 1.0, 0.0)
            c.xform = y
            c.layer = pick()
            c.hidden = rng.random() < 0.2
            c.locked = rng.random() < 0.2
            g.children.append(c)
        scene.groups.append(g)
    scene.dimensions.append(Dimension(V(0, 0, 0), V(1, 0, 0), V(0, 1, 0)))
    scene.guides.append(Guide(V(1, 2, 3)))
    scene.show_hidden_objects = rng.random() < 0.3
    scene.show_hidden_geometry = rng.random() < 0.3
    scene.hide_similar_components = rng.random() < 0.6
    if rng.random() < 0.25:
        scene._navigator_allowed = {g.uid for g in scene.groups[::2]}
    if rng.random() < 0.15:
        scene.plugin_data = {"project_tree_loose_locks": [[]]}   # the model's loose geometry
    return scene


def _rows(scene):
    ents = list(scene.loose_mesh.faces) + list(scene.loose_mesh.edges)
    if scene.mesh is not scene.loose_mesh:
        ents += list(scene.mesh.faces) + list(scene.mesh.edges)
    for g in scene.groups:
        ents.append(g)
        ents.extend(g.children)
    ents += list(scene.dimensions) + list(scene.guides)
    out = []
    for e in ents:
        out.append([bool(scene.entity_hidden(e)), bool(scene.entity_visible(e)),
                    bool(scene.entity_selectable(e)), bool(scene.entity_locked(e)),
                    bool(scene.edge_snappable(e)), bool(scene.group_snappable(e)),
                    bool(scene.similar_hidden(e)), list(scene._layer_state(e)),
                    scene._open_group_layer(e), bool(scene._object_hidden(e)),
                    bool(scene._face_hidden(e))])
    out.append(scene.open_group_paint())
    return out


def answers():
    import itertools
    from core import layers as L
    counter = itertools.count()
    real = L.new_uid
    L.new_uid = lambda: f"k{next(counter)}"
    try:
        rng = random.Random(20261125)
        out = []
        for k in range(60):
            scene = _scene(rng)
            rec = [_rows(scene)]
            opened = [g for g in scene.groups if not g.locked]
            if opened:
                g = rng.choice(opened)
                scene.begin_group_edit(g)
                rec.append(_rows(scene))
                if g.children and rng.random() < 0.7:
                    scene.begin_group_edit(g.children[0])
                    rec.append(_rows(scene))
                scene.end_group_edit()
                rec.append(_rows(scene))
            out.append(rec)
        return out
    finally:
        L.new_uid = real


def test_the_visibility_rules_answer_as_the_python_did():
    want = json.loads(gzip.decompress(GOLDEN.read_bytes()))
    got = json.loads(json.dumps(answers()))
    assert len(got) == len(want)
    for i, (g, w) in enumerate(zip(got, want)):
        assert len(g) == len(w), i
        for j, (a, b) in enumerate(zip(g, w)):
            bad = [r for r, (x, y) in enumerate(zip(a, b)) if x != y]
            assert len(a) == len(b) and not bad, (i, j, bad[:5], a[bad[0]], b[bad[0]])


if __name__ == "__main__" and "--write" in sys.argv:
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    GOLDEN.parent.mkdir(exist_ok=True)
    data = json.dumps(answers(), separators=(",", ":")).encode()
    GOLDEN.write_bytes(gzip.compress(data, mtime=0))
    rows = [r for s in json.loads(data) for part in s for r in part[:-1]]
    print(GOLDEN, len(data), len(rows), [sum(bool(r[k]) for r in rows) for k in range(7)])
