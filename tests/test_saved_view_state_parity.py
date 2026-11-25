# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""A saved view's capture and recall moved to Rust
(``model/saved_view_state.rs``) and must answer as ``core/saved_views.py``'s
Python did at ``fff1954a``: ``SavedView.capture`` of seeded scenes and
cameras (tags and folders shown and hidden, a style or none, active and
inactive section planes, the geographic layers present or not, shadows,
hidden objects, the hidden-things switches, lenses), ``take_camera``,
``recapture``, ``apply`` / ``apply_state`` of seeded views onto seeded
scenes (with and without ``put_style``, views from before each field),
``shown_state``, ``visibility_sig``, ``georef_state`` and
``apply_shadow_state``.

Regenerate only on purpose, against the reference implementation:
``python tests/test_saved_view_state_parity.py --write``."""
from __future__ import annotations

import gzip
import json
import math
import random
import sys
from pathlib import Path
from types import SimpleNamespace

GOLDEN = Path(__file__).parent / "fixtures" / "saved_view_state_golden.json.gz"


def _scene(rng):
    from built_core import Vec3 as V
    from core import layers as L
    from core.group import Group
    from core.mesh import Mesh
    from core.scene import Scene
    from built_core.section import SectionPlane
    scene = Scene()
    for i in range(rng.randint(0, 5)):
        scene.layers.append(L.Layer(f"T{i}", visible=rng.random() < 0.7))
    folders = []
    for i in range(rng.randint(0, 3)):
        folders.append(__import__("built_core").tag_edits.add_folder(scene, f"F{i}"))
    scene.folder_visibility = {f: rng.random() < 0.6 for f in L.folders_of(scene)}
    for i in range(rng.randint(0, 4)):
        g = Group(Mesh(), name=f"G{i}")
        g.uid = f"u{i}"
        g.hidden = rng.random() < 0.4
        if rng.random() < 0.4:
            c = Group(Mesh(), name=f"G{i}c")
            c.uid = f"u{i}c"
            c.hidden = rng.random() < 0.5
            g.children.append(c)
        scene.groups.append(g)
    for i in range(rng.randint(0, 3)):
        scene.section_planes.append(SectionPlane(V(0, 0, i), V(0, 0, 1), name=f"S{i}",
                                                uid=i + 1))
    if scene.section_planes and rng.random() < 0.6:
        scene.set_active_section(rng.choice(scene.section_planes))
    scene.show_section_planes = rng.random() < 0.7
    scene.show_section_cuts = rng.random() < 0.7
    scene.show_hidden_objects = rng.random() < 0.3
    scene.show_hidden_geometry = rng.random() < 0.3
    for attr in ("tile_layer", "terrain", "photo_mesh"):
        if rng.random() < 0.5:
            setattr(scene, attr, SimpleNamespace(visible=rng.random() < 0.5))
    if rng.random() < 0.2:
        scene.display_style = None
    elif rng.random() < 0.5:
        scene.display_style.edges = rng.random() < 0.5
        scene.display_style.name = rng.choice(("Default", "Hidden", "Plan"))
    if rng.random() < 0.15:
        scene.shadows = None
    elif scene.shadows is not None:
        scene.shadows.enabled = rng.random() < 0.5
        scene.shadows.hour = rng.randint(0, 23)
    if rng.random() < 0.3:
        scene._navigator_view_key = rng.choice(("plans", "v2"))
    if rng.random() < 0.3:
        scene._navigator_allowed = {g.uid for g in scene.groups[:1]}
    return scene


def _camera(rng):
    from built_core import Vec3 as V
    from core.camera import OrbitCamera
    cam = OrbitCamera()
    cam.target = V(rng.uniform(-9, 9), rng.uniform(-9, 9), rng.uniform(-2, 2))
    cam.distance = rng.uniform(1, 90)
    cam.yaw = rng.uniform(-3, 3)
    cam.pitch = rng.uniform(-1.4, 1.4)
    cam.fov_deg = rng.choice((35.0, 45.0, 60.0))
    cam.perspective = rng.random() < 0.7
    cam.two_point = rng.random() < 0.2
    if rng.random() < 0.4:
        cam.apply_lens({"shift_x": rng.uniform(-0.3, 0.3), "roll": rng.uniform(-10, 10)})
    return cam


def _jsonable(x):
    if isinstance(x, float):
        return repr(x) if not math.isfinite(x) else x
    if isinstance(x, (list, tuple)):
        return [type(x).__name__] + [_jsonable(v) for v in x]
    if isinstance(x, (set, frozenset)):
        return ["set"] + sorted(repr(v) for v in x)
    if isinstance(x, dict):
        return {"dict": [[repr(k), _jsonable(v)] for k, v in x.items()]}
    if x is None or isinstance(x, (bool, int, str)):
        return x
    return repr(x)


def _state(scene, camera):
    style = scene.display_style
    shadows = scene.shadows
    sec = scene.active_section()
    return _jsonable({
        "layers": [(ly.key, ly.visible) for ly in scene.layers],
        "folders": dict(scene.folder_visibility),
        "style": None if style is None else style.to_dict(),
        "section": None if sec is None else sec.name,
        "flags": (scene.show_section_planes, scene.show_section_cuts,
                  scene.show_hidden_objects, scene.show_hidden_geometry),
        "geo": [None if getattr(scene, a) is None else getattr(scene, a).visible
                for a in ("tile_layer", "terrain", "photo_mesh")],
        "shadows": None if shadows is None else shadows.to_dict(),
        "hidden": [g.hidden for g in scene.groups_by_uid().values()],
        "camera": (camera.target.toTuple(), camera.distance, camera.yaw, camera.pitch,
                   camera.fov_deg, camera.perspective, camera.two_point, camera.lens_dict()),
    })


def _view_fields(v):
    return _jsonable({f: getattr(v, f) for f in v._record_fields})


def answers():
    import itertools
    from core import layers as L
    counter = itertools.count()
    real = L.new_uid
    L.new_uid = lambda: f"k{next(counter)}"
    try:
        return _answers()
    finally:
        L.new_uid = real


def _answers():
    import built_core as N
    from built_core import saved_views as S
    out = {"capture": [], "apply": [], "shown": [], "sig": [], "geo": [], "shadow": []}
    rng = random.Random(20261125)
    views = []
    for k in range(80):
        scene, cam = _scene(rng), _camera(rng)
        style = None
        if rng.random() < 0.3:
            from built_core.style import Style
            style = Style(name="Own")
        v = S.SavedView.capture(f"V{k}", scene, cam, style)
        rec = [_view_fields(v), v.to_dict() and _jsonable(v.to_dict())]
        cam2 = _camera(rng)
        v.take_camera(cam2)
        rec.append(_view_fields(v))
        v.folder = rng.choice((None, "Walls"))
        v.recapture(_scene(rng), cam, None)
        rec.append(_view_fields(v))
        out["capture"].append(rec)
        views.append(v)
        out["shown"].append(_jsonable(N.saved_view_shown_state(scene)))
        out["sig"].append(_jsonable(N.saved_view_visibility_sig(scene)))
        out["geo"].append(_jsonable(N.saved_view_georef_state(scene)))
    for k in range(120):
        v = rng.choice(views)
        v = S.SavedView.from_dict(v.to_dict())
        for field in ("hidden_folders", "style", "section", "georef", "shadows",
                      "hidden_objects", "hidden_shown"):
            if rng.random() < 0.2:
                setattr(v, field, None)
        if v.section is not None and rng.random() < 0.3:
            v.section = {"active": rng.choice((None, 1, 2, 99))}
        if rng.random() < 0.2:
            v.hidden_layers = ["T0", "T2", "nope"]
        scene, cam = _scene(rng), _camera(rng)
        put = []
        if rng.random() < 0.3:
            v.apply(scene, cam, put.append)
        elif rng.random() < 0.5:
            v.apply(scene, cam)
        else:
            v.apply_state(scene)
        out["apply"].append([_state(scene, cam), [s.to_dict() for s in put]])
    for k in range(20):
        scene = _scene(rng)
        if scene.shadows is None:
            continue
        raw = {"enabled": rng.random() < 0.5, "hour": rng.choice((3, 30, "x")),
               "darkness": rng.uniform(0, 1)}
        held = scene.shadows
        N.saved_view_apply_shadows(scene, raw)
        out["shadow"].append([scene.shadows is held, _jsonable(scene.shadows.to_dict())])
    return out


def test_saved_view_capture_and_recall_answer_as_the_python_did():
    want = json.loads(gzip.decompress(GOLDEN.read_bytes()))
    got = json.loads(json.dumps(answers()))
    for key in want:
        assert len(got[key]) == len(want[key]), key
        bad = [i for i, (g, w) in enumerate(zip(got[key], want[key])) if g != w]
        assert not bad, (key, bad[:10], got[key][bad[0]] if bad else None,
                         want[key][bad[0]] if bad else None)


if __name__ == "__main__" and "--write" in sys.argv:
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    GOLDEN.parent.mkdir(exist_ok=True)
    data = json.dumps(answers(), separators=(",", ":")).encode()
    GOLDEN.write_bytes(gzip.compress(data, mtime=0))
    print(GOLDEN, len(data))
