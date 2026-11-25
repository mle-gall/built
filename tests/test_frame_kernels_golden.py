# SPDX-License-Identifier: GPL-3.0-or-later
"""The section cut's vertices and the selected hidden edges, held to the
bytes the Python modules they come from produced
(``tests/fixtures/frame_kernels_golden.json``, recorded from
``views/wgpu_sections.py`` and ``views/wgpu_lines.py`` before they were
deleted): the arithmetic moved to Rust, the numbers did not."""
import hashlib
import json
from pathlib import Path
from types import SimpleNamespace

import numpy as np
import pytest

GOLDEN = Path(__file__).parent / "fixtures" / "frame_kernels_golden.json"

PLANES = {"low": ((0.0, 0.0, 0.3), (0, 0, 1)), "across": ((0.5, 0.5, 0.5), (1, 0, 0)),
          "oblique": ((0.5, 0.5, 0.4), (0.3, -0.5, 0.8))}
FILLS = ({"pattern": "solid"}, {"pattern": "hatch", "spacing": 0.4}, {"pattern": "cross", "spacing": 0.25},
         {"pattern": "none"})
CAMERAS = ({}, {"yaw": 37.5, "pitch": 12.0, "distance": 80.0}, {"yaw": -120.0, "pitch": 55.0, "fov_deg": 60.0})


def digest(array) -> str:
    return hashlib.sha256(np.ascontiguousarray(array).tobytes()).hexdigest()


def city():
    from scripts import bench_models
    return bench_models.build("S")


def soften(scene):
    """Soft and hidden edges on every mesh, by position in its edge list."""
    for group in scene.groups:
        for index, edge in enumerate(group.mesh.edges):
            edge.soft = index % 3 == 0
            edge.hidden = index % 7 == 0


def section_cases(geometry, scene, packed):
    """``{case: (bytes, digest)}`` of ``geometry`` over planes, fills, face
    modes, cameras and heights."""
    from built_core import Vec3
    from built_core.camera import OrbitCamera
    from built_core.section import SectionPlane
    from built_core.style import Style
    low, high = scene.bounds()
    low, high = low.toTuple(), high.toTuple()
    out = {}
    for name, (at, normal) in PLANES.items():
        point = Vec3(*(lo + (hi - lo) * t for lo, hi, t in zip(low, high, at)))
        plane = SectionPlane(point, Vec3(*normal), active=True)
        scene.section_planes = [plane]
        view = SimpleNamespace(packed=packed)
        for fill in FILLS:
            plane.fill = dict(fill)
            for mode in ("shaded", "xray"):
                style = Style()
                style.face_mode = mode
                for number, pose in enumerate(CAMERAS):
                    camera = OrbitCamera()
                    for field, value in pose.items():
                        setattr(camera, field, value)
                    for height in (600, 1440):
                        data = geometry(view, scene, style, camera, height)
                        key = f"{name}/{fill['pattern']}/{mode}/camera{number}/{height}"
                        out[key] = [len(data), digest(data)]
    scene.section_planes = []
    return out


def kernel_cases(scene, packed):
    """The chords of each plane, and what the fill kernels make of them:
    the rings (``section_loops``), the fill's triangles, the hatch's
    segments at two spacings, one way and both."""
    import built_core
    from built_core import Vec3
    from built_core.camera import OrbitCamera
    from built_core.section import SectionPlane
    from built_core.style import Style
    from views.wgpu_sections import geometry
    low, high = scene.bounds()
    low, high = low.toTuple(), high.toTuple()
    out = {}
    for name, (at, normal) in PLANES.items():
        point = Vec3(*(lo + (hi - lo) * t for lo, hi, t in zip(low, high, at)))
        plane = SectionPlane(point, Vec3(*normal), active=True)
        scene.section_planes = [plane]
        view = SimpleNamespace(packed=packed)
        geometry(view, scene, Style(), OrbitCamera(), 600)
        chords = view._section_geometry_cache[1]
        rings = built_core.section_loops(chords)
        fill = built_core.section_fill_triangles(chords, plane.normal)
        out[name] = {"chords": [len(chords), digest(chords)],
                     "rings": [len(rings), digest(np.concatenate(rings)) if rings else ""],
                     "fill": [len(fill), digest(fill)]}
        for spacing in (0.25, 1.5):
            for cross in (False, True):
                hatch = built_core.section_hatch_segments(chords, plane.normal, spacing, cross)
                out[name][f"hatch/{spacing}/{cross}"] = [len(hatch), digest(hatch)]
    scene.section_planes = []
    return out


def hidden_view(scene):
    from built_core import pack_scene
    from tests.controller_surface import ControllerSurface
    from tests.headless_gpu import HeadlessView
    source = ControllerSurface()
    source.scene = scene
    source.resize(800, 600)
    view = HeadlessView()
    view.resize(800, 600)
    view.scene = scene
    view.source_viewport = source
    view.camera = source.camera
    view.packed = pack_scene(scene)
    view.uid_rows = {}
    for row, group in enumerate(view.packed.groups):
        if group is not None:
            view.uid_rows.setdefault(view.packed.row_key(group), []).append(row)
    return view, source


def hidden_cases(selected_hidden, scene):
    from built_core.group import Group
    view, source = hidden_view(scene)
    try:
        scene.show_hidden_geometry = True
        groups = [g for g in scene.groups if isinstance(g, Group)]
        out = {}
        for name, chosen in (("all", groups), ("every third", groups[::3]), ("one", groups[:1])):
            scene.select(chosen)
            view.__dict__.pop("_hidden_lines", None)
            _, segments = selected_hidden(view)
            out[name] = [len(segments), digest(segments)]
        return out
    finally:
        view.close()
        source.close()


@pytest.fixture(scope="module")
def golden():
    return json.loads(GOLDEN.read_text())


def test_section_vertices_are_the_recorded_bytes(golden):
    from built_core import pack_scene
    from views.wgpu_sections import geometry
    scene = city()
    got = section_cases(geometry, scene, pack_scene(scene))
    assert got.keys() == golden["sections"].keys()
    different = [case for case in got if got[case] != golden["sections"][case]]
    assert not different, different[:6]
    assert sum(size for size, _ in got.values()) > 1_000_000, "the cases draw something"


def test_selected_hidden_edges_are_the_recorded_bytes(golden):
    from views.wgpu_lines import selected_hidden
    scene = city()
    soften(scene)
    got = hidden_cases(selected_hidden, scene)
    assert got == golden["hidden"]
    assert got["all"][0] > 1000, "the cases draw something"


def test_fill_kernels_are_the_recorded_bytes(golden):
    from built_core import pack_scene
    scene = city()
    got = kernel_cases(scene, pack_scene(scene))
    assert got == golden["kernels"]
    assert all(case["fill"][0] > 100 for case in got.values()), "the cases fill something"
