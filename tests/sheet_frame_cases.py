# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""What a sheet frame offers and draws, case by case, for the goldens
recorded from the composer (scripts/sheet_frame_goldens.py) before it went
and the workspace tests that hold the native path to them
(tests/test_sheets_workspace.py): the scene each case sets up, what the
workspace answers (built_render.sheet_frames) and what the composer
answered, in one canonical, JSON-able form."""
from __future__ import annotations

import json
import re

CASES = ("snaps", "circles", "plan_cut_frame", "plan_cut_layer", "annotations", "dxf_class", "dxf_layer")


def _round(v, nd=5):
    if isinstance(v, float):
        return round(v, nd)
    if isinstance(v, (list, tuple)):
        return [_round(x, nd) for x in v]
    return v


def canon(value):
    """A case's answer as JSON reads it back."""
    return json.loads(json.dumps(value))


# ---- The scenes -----------------------------------------------------------------------------------
def plan_cut(scene, plan, by_layer: bool) -> None:
    """A plan cut through the fixture's boxes: the box east of x = 6 is
    concrete, the rest brick walls."""
    from built_core import Vec3
    from built_core.layers import Layer
    from built_core.section import SectionPlane
    plan.cut_fill_by_layer = by_layer
    scene.layers.append(Layer("Muros", hatch="brick", color=(0.8, 0.4, 0.3)))
    scene.layers.append(Layer("Losa", hatch="concrete", color=(0.5, 0.5, 0.55)))
    for f in scene.mesh.faces:
        f.attrs["layer"] = "Losa" if min(v.x() for v in f.vertices) >= 5.99 else "Muros"
    sp = SectionPlane(Vec3(0.0, 0.0, 1.0), Vec3(0.0, 0.0, 1.0))
    scene.section_planes.append(sp)
    scene.set_active_section(sp)
    scene.version += 1


def annotated(scene, plan, window) -> None:
    """Every kind of model annotation on the plan: a traced path with its
    chainages, two section planes, an extension's items, three dimensions
    (one on a hidden tag) with oblique ticks, two leader texts (one hidden)."""
    from built_core import Vec3
    from built_core.dimension import Dimension
    from built_core.layers import Layer
    from built_core.section import SectionPlane
    from built_core.textlabel import TextLabel
    from georef.geopath import GeoPath
    plan.annotations = plan.km_marks = plan.section_marks = True
    plan.annot_text_mm = 3.0
    scene.layers.append(Layer("Notas", visible=False))
    scene.geo_paths.append(GeoPath([Vec3(0.0, 0.0, 0.0), Vec3(8.0, 1.0, 0.0), Vec3(12.0, 6.0, 0.0)], name="eje"))
    scene.dimensions.append(Dimension(Vec3(0.0, 0.0, 0.0), Vec3(6.0, 0.0, 0.0), Vec3(0.0, -1.0, 0.0)))
    scene.dimensions.append(Dimension(Vec3(0.0, 0.0, 0.0), Vec3(0.0, 4.0, 0.0), Vec3(-1.0, 0.0, 0.0), axis="y"))
    scene.dimensions.append(Dimension(Vec3(1.0, 1.0, 0.0), Vec3(3.0, 1.0, 0.0), Vec3(0.0, 1.0, 0.0), layer="Notas"))
    scene.text_labels.append(TextLabel(Vec3(2.0, 2.0, 1.0), Vec3(1.5, 1.0, 0.0), "Muro\nde carga"))
    scene.text_labels.append(TextLabel(Vec3(5.0, 2.0, 1.0), Vec3(-1.5, 1.0, 0.0), "oculto", layer="Notas"))
    scene.section_planes.append(SectionPlane(Vec3(3.0, 0.0, 0.0), Vec3(1.0, 0.0, 0.0)))
    scene.section_planes.append(SectionPlane(Vec3(0.0, 2.0, 0.0), Vec3(0.0, -1.0, 0.0), symbol="B"))
    scene.dimension_style = dict(getattr(scene, "dimension_style", {}) or {}, ends="tick")
    scene.version += 1

    def provider(scene, sheet):
        return [{"line": [(0, 0, 0), (4, 4, 0)], "width_mm": 0.5, "color": (200, 30, 30)},
                {"text": "P1", "at": (4, 4, 0), "toward": (0, 4, 0), "anchor": "start", "background": (255, 255, 0),
                 "background_opacity": 50},
                {"text": "k", "at": (4.1, 4, 0), "priority": 1}]
    window._ext_sheet_annots = [provider]


def dxf_layers(scene) -> None:
    """Two tags, one hatched, and a plan cut: the DXF goes layer by layer."""
    from built_core import Vec3
    from built_core.layers import Layer
    from built_core.section import SectionPlane
    scene.layers.append(Layer("Muros", hatch="brick", color=(0.8, 0.4, 0.3)))
    scene.layers.append(Layer("Ejes", linetype="center", color=(0.2, 0.4, 0.9)))
    for f in scene.mesh.faces:
        f.attrs["layer"] = "Ejes" if min(v.x() for v in f.vertices) >= 5.99 else "Muros"
    sp = SectionPlane(Vec3(0.0, 0.0, 1.0), Vec3(0.0, 0.0, 1.0))
    scene.section_planes.append(sp)
    scene.set_active_section(sp)
    scene.version += 1


def set_up(case, scene, plan, window) -> None:
    if case.startswith("plan_cut"):
        plan_cut(scene, plan, case == "plan_cut_layer")
    elif case == "annotations":
        annotated(scene, plan, window)
    elif case == "dxf_layer":
        dxf_layers(scene)


# ---- The answers ------------------------------------------------------------------------------------
def dxf_entities(path) -> list:
    body = open(path, encoding="utf-8").read().split("ENTITIES", 1)[1]
    return sorted(re.sub(r"(-?\d+\.\d{4})\d*", r"\1", body).split("\n0\n"))


def _canon_points(pts, world):
    return sorted([_round(list(map(float, a)) + list(map(float, b)), 6) for a, b in zip(pts, world)])


def _canon_rings(rings):
    import numpy as np
    return [sorted(_round(np.asarray(r, float).tolist(), 4)) for r in rings]


def native(case, source, plan, tmp_path, window=None) -> dict:
    """What the workspace answers for ``case`` (``window``: the one holding
    the extensions' sheet providers)."""
    from built_render import sheet_frames as w
    if case == "snaps":
        pts, wpts, edges, _ew, _c = w.snaps(source, plan)
        return {"points": _canon_points(pts, wpts), "edges": len(edges)}
    if case == "circles":
        return {"circles": sorted(_round(c, 6) for c in w.circles(source, plan).tolist())}
    if case.startswith("plan_cut"):
        got = w.drawing(source, plan)
        rings = _canon_rings(got["fills"])
        looks = [[None if c is None else _round([float(v) for v in c], 4), h, f and f.get("cut")]
                 for c, h, _g, _i, f in got["fill_looks"]]
        return {"rings": sorted(zip(map(json.dumps, rings), map(json.dumps, looks)))}
    if case == "annotations":
        providers = getattr(window, "_ext_sheet_annots", None)
        return {"items": canon(_round([list(a) for a in w.annotations(source, plan, providers)]))}
    if case.startswith("dxf"):
        path = tmp_path / f"{case}.dxf"
        n = w.export_dxf(source, plan, path)
        return {"n": n, "entities": dxf_entities(path)}
    raise ValueError(case)


def reference(case, composer, plan, tmp_path) -> dict:
    """What the composer (views/composer.py) answered for ``case``."""
    composer._invalidate_geometry_caches()
    if case == "snaps":
        pts, wpts = composer.frame_snap_points(plan)
        return {"points": _canon_points(pts, wpts), "edges": len(composer.frame_snap_edges(plan)[0])}
    if case == "circles":
        return {"circles": sorted(_round([float(v) for v in c], 6) for c in composer.frame_circles(plan))}
    if case.startswith("plan_cut"):
        composer.compute_hlr(plan)
        rings = _canon_rings(composer.hlr_fills[id(plan)])
        rgb = lambda q: None if q is None else _round([float(v) for v in q.getRgbF()[:3]], 4)  # noqa: E731
        looks = [[rgb(look[0]), look[1], look[4] and look[4].get("cut")] for look in composer.hlr_fill_styles[id(plan)]]
        return {"rings": sorted(zip(map(json.dumps, rings), map(json.dumps, looks)))}
    if case == "annotations":
        return {"items": canon(_round([list(a) for a in composer.compute_annotations(plan)]))}
    if case.startswith("dxf"):
        path = tmp_path / f"{case}.dxf"
        n = composer.export_frame_dxf(plan, str(path))
        return {"n": n, "entities": dxf_entities(path)}
    raise ValueError(case)
