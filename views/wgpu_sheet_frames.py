"""A sheet frame's content for the native page.

Glue only (docs/python-glue.md). The frame's camera
(``sheet_frame_camera``), what it shows (``sheet_frame_hidden``) and the
geometry its vector pass draws (``sheet_frame_geometry``, read from the
mesh stores) come from the document in Rust. A shaded frame is drawn by
the renderer into a texture the page keeps (``page_frame_render``), with no
read-back. A vector frame's lines come from the hidden-line pass
(``core.hlr.hlr_drawing``, its kernel in Rust) over that geometry. The live scene and camera are
never touched: what the frame's saved view changes besides visibility
(style, section, shadows, base map) is read through :class:`FrameScene`, so
the style payload gets the frame's values. The composer's
``_with_frame_camera`` used to switch all of that on the live scene and back.
"""
from __future__ import annotations

from copy import copy

import numpy as np

import ingetrazo_core as NATIVE

#: The fields of the frame camera the render path reads besides its matrices.
_CAMERA_FIELDS = ("distance", "yaw", "pitch", "fov_deg", "perspective", "aspect", "two_point",
                  "shift_x", "shift_y", "roll_deg", "clip_near", "clip_far")


class _Shown:
    """A geographic layer as the frame's view shows it."""

    def __init__(self, obj, visible):
        self.__dict__["_obj"], self.__dict__["visible"] = obj, visible

    def __getattr__(self, name):
        return getattr(self._obj, name)


class FrameScene:
    """The live scene, read as the frame's view shows it. Writes are refused."""

    def __init__(self, scene, **over):
        self.__dict__["_scene"] = scene
        self.__dict__.update(over)

    def __getattr__(self, name):
        return getattr(self._scene, name)

    def __setattr__(self, name, value):
        raise AttributeError(f"a sheet frame does not change the scene ({name})")


def saved_view(scene, frame):
    """The frame's saved view, or None; ``False`` when that view is gone."""
    key = getattr(frame, "view_key", "") or ""
    if not key.startswith("scene:"):
        return None
    return next((v for v in getattr(scene, "saved_views", ()) if v.name == key[6:]), False)


def frame_scene(scene, frame, view) -> FrameScene:
    """What ``SavedView.apply_state`` and ``apply_frame_shadows`` changed on
    the live scene, as overrides of a facade."""
    from core.saved_views import georef_objects
    over = {"edit_group": None, "_bounds_cache": None}
    if view is not None and view.style:
        from core.style import Style
        over["display_style"] = Style.from_dict(view.style)
    if view is not None and view.section is not None:
        uid = view.section.get("active")
        target = next((sp for sp in getattr(scene, "section_planes", []) if sp.uid == uid), None)
        over["active_section"] = lambda: target
        over["show_section_planes"] = bool(view.section.get("planes_shown", True))
        over["show_section_cuts"] = bool(view.section.get("cuts_shown", True))
    if view is not None and view.georef is not None:
        names = {"map": "tile_layer", "terrain": "terrain", "survey": "photo_mesh"}
        for key, obj in georef_objects(scene):
            if obj is not None and key in view.georef:
                over[names[key]] = _Shown(obj, bool(view.georef[key]))
    if view is not None and view.hidden_shown is not None:
        over["show_hidden_objects"] = bool(view.hidden_shown.get("objects"))
        over["show_hidden_geometry"] = bool(view.hidden_shown.get("geometry"))
    shadows = getattr(scene, "shadows", None)
    if shadows is not None:
        from core.sun import ShadowSettings
        state = shadows.to_dict()
        if view is not None and view.shadows is not None:
            state = ShadowSettings.from_dict(view.shadows).to_dict()
        want, hour = getattr(frame, "shadows", None), getattr(frame, "sun_hour", None)
        if want is not None:
            state["enabled"] = bool(want)
        if hour is not None:
            h = max(0.0, min(23.999, float(hour)))
            state["hour"], state["minute"] = int(h), int(round((h - int(h)) * 60.0))
        over["shadows"] = ShadowSettings.from_dict(state)
    return FrameScene(scene, **over)


def frame_style(viewport, scene, frame):
    """The style the frame renders with: its own pick over the view's."""
    from dataclasses import replace
    from core.style import Style, style_by_name
    style = frame.style
    if style == "tecnico":
        return Style(name="tecnico", face_mode="hidden_line", sky=False, background=(1.0, 1.0, 1.0))
    if style == "lineas":
        return Style(name="lineas", face_mode="wireframe", sky=False, background=(1.0, 1.0, 1.0))
    picked = style_by_name(style[6:]) if isinstance(style, str) and style.startswith("style:") else None
    if picked is None:
        picked = getattr(viewport, "view_style", None) or scene.display_style or Style()
    if getattr(frame, "paper_bg", False):
        picked = replace(picked, background=(1.0, 1.0, 1.0), sky=False)
    return picked


def camera_inputs(live, scene):
    """What a frame camera starts from (sheet_frame_camera's ``base`` and
    ``bounds``): the live camera's fields, the model's box and depth reach."""
    lo, hi = scene.bounds()
    bounds = None if lo is None else (lo.toTuple(), hi.toTuple())
    base = {k: getattr(live, k) for k in ("target", "distance", "yaw", "pitch", "up", "fov_deg",
                                          "perspective", "two_point", "shift_x", "shift_y",
                                          "roll_deg", "clip_near", "clip_far")}
    base["target"], base["up"] = live.target.toTuple(), live.up.toTuple()
    if bounds is not None:
        c = (np.add(bounds[0], bounds[1]) * 0.5)
        base["depth_reach"] = (tuple(c.tolist()), float(np.linalg.norm(np.subtract(bounds[1], bounds[0])) * 0.5))
    return base, bounds


def frame_camera(live, frame, view, scene, origin=None):
    """An OrbitCamera at the frame's view whose matrices are the Rust ones
    (``origin``: the resident geometry's offset, subtracted)."""
    from ingetrazo_core import Matrix4, Vec3
    base, bounds = camera_inputs(live, scene)
    got = NATIVE.sheet_frame_camera(frame, view or None, base, bounds)
    cam = copy(live)
    for k in _CAMERA_FIELDS:
        setattr(cam, k, got[k])
    shift = np.zeros(3) if origin is None else np.asarray(origin, float)
    cam.target = Vec3(*(np.asarray(got["target"]) - shift).tolist())
    cam.up = Vec3(*got["up"])
    cam.depth_reach = None if "depth_reach" not in base else (
        Vec3(*(np.asarray(base["depth_reach"][0]) - shift).tolist()), base["depth_reach"][1])
    view_m = Matrix4(got["view"]).transposed()
    if origin is not None:
        view_m.translate(*shift.tolist())
    proj_m = Matrix4(got["projection"]).transposed()
    cam.view_matrix = lambda: Matrix4(list(view_m.data())).transposed()
    cam.projection_matrix = lambda: Matrix4(list(proj_m.data())).transposed()
    return cam


def frame_rows(view_widget, scene, frame, view):
    """The resident rows with the frame's visibility (and the navigator
    view's filter and tint, which stay Python: records)."""
    packed = view_widget.packed
    hidden = np.frombuffer(NATIVE.sheet_frame_hidden(scene._root, list(scene.groups),
                                                     list(packed.groups), view or None), np.uint8)
    rows = packed.instances.copy()
    billboard = np.array([8 if getattr(g, "billboard", False) else 0 for g in packed.groups], np.uint32)
    rows["info"][:, 2] = hidden.astype(np.uint32) | billboard
    rows["info"][:, 3] = 0
    key = getattr(view, "navigator_view", None) if view else None
    if key:
        _navigator(rows, packed, scene, key)
    return rows


def _navigator(rows, packed, scene, key) -> None:
    from types import SimpleNamespace
    from core import bim_registry
    from core.agent_facets import set_display
    saved = bim_registry.get(scene).get("views", {}).get(key)
    if saved is None:
        return
    shadow = copy(scene)
    shadow.selection = set(scene.selection)
    shadow.edit_group = None
    proxy = SimpleNamespace(scene=shadow, update=lambda: None)
    set_display(proxy, saved["filter"], saved.get("color_by"), isolate=saved.get("isolate", False),
                key=key, remember=False, exact=saved.get("exact", False))
    allowed, through = shadow._navigator_allowed, shadow._navigator_through or ()
    tint = getattr(proxy, "view_tint", None) or {}
    for i, (uid, g) in enumerate(zip(packed.uids, packed.groups)):
        if g is not None and allowed is not None:
            k = shadow.navigator_key(g)
            if k not in allowed and k not in through:
                rows["info"][i, 2] |= 1
        rgb = tint.get(uid)
        if rgb is not None:
            rows["info"][i, 3] = int(np.array([*np.clip(np.array(rgb[:3]) * 255, 0, 255), 255],
                                              np.uint8).view("<u4")[0])


def _render(view_widget, frame, **out):
    """The renderer's pass for a shaded frame, with the frame's camera,
    rows and look; ``out`` says where the picture goes."""
    from core.composition import RENDER_DPI, pen_px
    source = view_widget.source_viewport
    scene = source.scene
    view = saved_view(scene, frame)
    if view is False:
        return False
    facade = frame_scene(scene, frame, view)
    style = frame_style(source, facade, frame)
    w_px, h_px = frame.render_px(RENDER_DPI)
    return view_widget.render_image(
        w_px, h_px, sharp_shadows=True, scene=facade, style=style,
        camera=lambda origin: frame_camera(source.camera, frame, view, scene, origin),
        rows=lambda: frame_rows(view_widget, scene, frame, view),
        edge_px=pen_px(getattr(frame, "pen_edge_mm", 0.18), RENDER_DPI),
        profile_px=pen_px(getattr(frame, "pen_profile_mm", 0.35), RENDER_DPI), **out)


def render(view_widget, frame, key) -> bool:
    """Render ``frame`` into the page texture ``key``; False when it has
    nothing to show (its saved view is gone)."""
    return _render(view_widget, frame, page_key=key) is not False


def picture(view_widget, frame):
    """A shaded frame's render read back for a file: ``(w, h, rgba)``, or
    None when its saved view is gone."""
    from PySide6.QtGui import QImage
    image = _render(view_widget, frame)
    if image is False or image is None or image.isNull():
        return None
    rgba = image.convertToFormat(QImage.Format_RGBA8888)
    w, h = rgba.width(), rgba.height()
    rows = np.frombuffer(rgba.constBits(), np.uint8, count=rgba.bytesPerLine() * h).reshape(h, -1)
    # The render composed its own background: on paper it is opaque.
    px = rows[:, :w * 4].reshape(h, w, 4).copy()
    px[..., 3] = 255
    return w, h, px.tobytes()


def _frame_pass(source, frame, tags: bool = False):
    """What the frame's hidden-line pass runs on: the scene as its view
    shows it, its camera and its geometry (Rust), with each triangle's and
    line's tag when ``tags``; None when its view is gone."""
    from core._native import qt_hypot
    scene = source.scene
    view = saved_view(scene, frame)
    if view is False:
        return None
    facade = frame_scene(scene, frame, view)
    camera = frame_camera(source.camera, frame, view, scene)
    geometry, tagged = NATIVE.sheet_frame_geometry(scene._root, list(scene.groups), scene.loose_mesh,
                                                   view or None, qt_hypot() or "libc++-fma", tags)
    if tags:
        geometry = (*geometry, *(np.array(t, dtype=object) for t in tagged))
    return facade, camera, geometry


def _hlr(source, frame, tags: bool = False, **kw):
    """The frame's hidden-line pass (Rust camera and geometry, with each
    triangle's and line's tag when ``tags``) and the map from its camera
    plane to frame paper mm; None when its view is gone."""
    from core.composition import model_height_for_frame
    from core.hlr import hlr_drawing
    got = _frame_pass(source, frame, tags)
    if got is None:
        return None
    facade, camera, geometry = got
    d = hlr_drawing(facade, camera, geometry=geometry, layers=tags, **kw)
    model_h = model_height_for_frame(frame.h_mm, frame.scale_n)
    k = frame.h_mm / model_h                       # paper mm per metre
    half_h = model_h / 2.0
    half_w = half_h * (frame.w_mm / frame.h_mm)

    def to_mm(xy):
        out = np.asarray(xy, float).copy()
        out[..., 0] = (out[..., 0] + half_w) * k
        out[..., 1] = (half_h - out[..., 1]) * k
        return out
    return d, to_mm


def drawing(source, frame):
    """A vector frame's content for the painter (sheets::items_d): its
    lines in paper mm, frame-local, their kinds (``core.hlr`` KIND_*), its
    cut rings, and — when the frame inks layer by layer (a layer with a
    line type, weight or cut weight of its own, colours by layer) — each
    line's tag and each tag's look (``core.layers.line_styles``, the line
    type resolved to its dash); when it fills its cut by layer or a layer
    has a hatch, each ring's look (colour, pattern, ground, ink, form) as
    the composer's ``compute_hlr`` gave them. None when its view is gone."""
    from core.composition import model_height_for_frame
    from core.layers import (LINETYPES, cut_fill_colours, cut_hatch_forms, cut_hatch_grounds,
                             cut_hatch_inks, cut_hatches, grain_layers, layer_lines_wanted,
                             line_styles, section_cut_kind)
    scene = source.scene
    view = saved_view(scene, frame)
    if view is False:
        return None
    facade = frame_scene(scene, frame, view)
    coloured = bool(getattr(frame, "layer_colours", False))
    filled = (getattr(frame, "cut_fill", "solid") or "solid") != "none"
    fill_by_layer = bool(getattr(frame, "cut_fill_by_layer", False)) and filled
    hatches = cut_hatches(facade) if filled else {}
    by_layer = layer_lines_wanted(facade) or coloured or fill_by_layer or bool(hatches)
    extra = {"depth_layers": grain_layers(facade)} if by_layer else {}
    got = _hlr(source, frame, tags=by_layer, profiles=bool(getattr(frame, "profiles", True)), fills=filled,
               hidden=bool(getattr(frame, "hidden_lines", False)), **extra)
    if got is None:
        return None
    d, to_mm = got
    out = {"segs": to_mm(np.asarray(d.segs, float).reshape(-1, 2, 2)).reshape(-1, 4),
           "kinds": np.asarray(d.kinds, np.int8),
           "fills": [to_mm(np.asarray(ring, float).reshape(-1, 2)) for ring in d.loops]}
    if d.layers is not None:
        style = getattr(facade, "display_style", None)
        looks = line_styles(facade, coloured or bool(getattr(style, "color_by_layer", False)))
        out["tags"] = [str(t) for t in d.layers]
        out["styles"] = {tag: (st[0], st[2], st[3], list(st[4] if st[4] is not None else LINETYPES.get(st[1], ())),
                               st[5] if len(st) > 5 else None)
                         for tag, st in looks.items()}
    if (fill_by_layer or hatches) and d.loop_layers is not None:
        table = cut_fill_colours(facade) if fill_by_layer else {}
        grounds, inks = cut_hatch_grounds(facade), cut_hatch_inks(facade)
        cut = section_cut_kind(facade)
        forms = {tag: dict(f, cut=cut) for tag, f in cut_hatch_forms(facade, float(frame.scale_n or 100.0)).items()}
        k = frame.h_mm / model_height_for_frame(frame.h_mm, frame.scale_n)
        rgb = lambda t, tag: list(t[tag][:3]) if tag in t else None  # noqa: E731
        fill_looks = []
        for i, tag in enumerate(d.loop_layers):
            form = forms.get(tag)
            depth = d.loop_depths[i] if d.loop_depths else None
            if form is not None and depth is not None:
                form = dict(form, depth=round(depth * k, 4))
            fill_looks.append((rgb(table, tag), hatches.get(tag), rgb(grounds, tag), rgb(inks, tag), form))
        out["fill_looks"] = fill_looks
    return out


def snaps(source, frame):
    """What the drawing tools catch in ``frame`` (the composer's
    ``frame_snap_points`` / ``frame_snap_edges``, exact pass): the drawn
    edges' ends and middles in page mm with their model points, and the
    edges themselves, clipped to the frame. None when there is nothing to
    catch (its view is gone, or a perspective frame)."""
    if getattr(frame, "perspective", False):
        return None
    got = _hlr(source, frame, profiles=False, fills=False, hidden=False)
    if got is None:
        return None
    d, to_mm = got
    segs = to_mm(np.asarray(d.segs, float).reshape(-1, 2, 2)) + (frame.x_mm, frame.y_mm)
    world = np.asarray(d.world, float).reshape(-1, 2, 3)
    pts = np.concatenate([segs[:, 0], segs[:, 1], segs.mean(axis=1)]).reshape(-1, 2)
    wpts = np.concatenate([world[:, 0], world[:, 1], world.mean(axis=1)]).reshape(-1, 3)
    x0, y0, x1, y1 = frame.x_mm - 0.5, frame.y_mm - 0.5, frame.x_mm + frame.w_mm + 0.5, frame.y_mm + frame.h_mm + 0.5
    keep = (pts[:, 0] >= x0) & (pts[:, 0] <= x1) & (pts[:, 1] >= y0) & (pts[:, 1] <= y1)
    flat = segs.reshape(-1, 4)
    near = ((np.maximum(flat[:, 0], flat[:, 2]) >= x0) & (np.minimum(flat[:, 0], flat[:, 2]) <= x1)
            & (np.maximum(flat[:, 1], flat[:, 3]) >= y0) & (np.minimum(flat[:, 1], flat[:, 3]) <= y1))
    return pts[keep], wpts[keep], flat[near], world[near], circles(source, frame)


_CIRCLES = {}


def circles(source, frame):
    """The model's circles and arcs ``frame`` sees face on, as page
    ``(cx, cy, r)`` mm (the composer's ``frame_circles``): what the radius
    tool takes from one click on the arc. The model's circles are collected
    once per model version (core.hlr.collect_circles)."""
    from core.hlr import _to_cam, camera_basis, collect_circles
    if getattr(frame, "perspective", False):
        return np.empty((0, 3))
    scene = source.scene
    stamp = (id(scene), getattr(scene, "version", None))
    if _CIRCLES.get("stamp") != stamp:
        _CIRCLES.update(stamp=stamp, circles=collect_circles(scene))
    view = saved_view(scene, frame)
    if view is False:
        return np.empty((0, 3))
    camera = frame_camera(source.camera, frame, view, scene)
    eye, right, up, fwd = camera_basis(camera)
    got = _hlr_to_mm(frame)
    out = []
    for c, r, n in _CIRCLES["circles"]:
        if abs(n.x() * fwd[0] + n.y() * fwd[1] + n.z() * fwd[2]) < 0.9995:
            continue                                  # seen at a slant
        c3 = np.array([c.x(), c.y(), c.z()], float)
        cam = _to_cam(np.stack([c3, c3 + right * r]), eye, right, up, fwd)[:, :2]
        (px, py), (qx, qy) = got(cam)
        out.append((frame.x_mm + px, frame.y_mm + py, float(np.hypot(qx - px, qy - py))))
    return np.array(out, float).reshape(-1, 3)


def annotations(source, frame, providers=None):
    """A frame's model annotations (sheets::annots, the composer's
    ``compute_annotations``) as overlay tuples, frame-local mm: the scene
    read here as plain data — traced paths draped on the ground, the
    dimensions with their texts, the leader labels, the section planes, the
    extensions' ``providers`` (``add_sheet_annotations``) — and their
    projection through the frame camera, what the frame's view hides, and
    the layout in Rust. [] when its view is gone."""
    from ingetrazo_core import Vec3
    from core.layers import layer_of
    from core.units import format_length
    scene = source.scene
    view = saved_view(scene, frame)
    if view is False:
        return []
    base, bounds = camera_inputs(source.camera, scene)
    drape = getattr(source, "drape", None)
    lift = (lambda p: drape(p).toTuple()) if drape is not None else (lambda p: p.toTuple())  # noqa: E731
    hidden = lambda e: scene.entity_hidden(e) or scene.similar_hidden(e)  # noqa: E731
    data = {"root": scene._root, "groups": list(scene.groups), "loose": scene.loose_mesh,
            "paths": [([lift(p) for p in path.points], path.closed, [p.toTuple() for p in path.profile_points()])
                      for path in getattr(scene, "geo_paths", None) or []],
            "drape": lambda x, y: lift(Vec3(x, y, 0.0)),
            "planes": [(sp.point.toTuple(), sp.normal.toTuple(), sp.symbol)
                       for sp in getattr(scene, "section_planes", None) or []]}
    if providers:
        data["providers"], data["scene"] = providers, frame_scene(scene, frame, view)
    if getattr(frame, "annotations", False):
        style = getattr(scene, "dimension_style", {}) or {}
        fmt = lambda v: format_length(v, style.get("units", "m"), int(style.get("decimals", 2)),  # noqa: E731
                                      bool(style.get("show_unit", True)))
        data["ends"] = style.get("ends", "arrow")
        data["dims"] = [(d.a.toTuple(), d.b.toTuple(), *(p.toTuple() for p in d.line_points()),
                         d.display_text(fmt(d.value())), layer_of(d), hidden(d))
                        for d in getattr(scene, "dimensions", []) or []]
        data["labels"] = [(lab.anchor.toTuple(), lab.position().toTuple(), lab.text, layer_of(lab), hidden(lab))
                          for lab in getattr(scene, "text_labels", []) or []]
    return NATIVE.sheet_frame_annotations(frame, {"view": view, "base": base, "bounds": bounds}, data)


def _hlr_to_mm(frame):
    """A frame's camera plane (metres) to frame paper mm."""
    from core.composition import model_height_for_frame
    model_h = model_height_for_frame(frame.h_mm, frame.scale_n)
    k = frame.h_mm / model_h
    half_h = model_h / 2.0
    half_w = half_h * (frame.w_mm / frame.h_mm)

    def to_mm(xy):
        out = np.asarray(xy, float).copy()
        out[..., 0] = (out[..., 0] + half_w) * k
        out[..., 1] = (half_h - out[..., 1]) * k
        return out
    return to_mm


def frame_contents(view_widget, comps, ground=None, providers=None):
    """Each frame's content for a file, worked out afresh: ``(contents,
    pictures, profiles, troubled)`` as sheet_pdf / sheet_image take them,
    with the model's annotations over each (``providers``: the extensions'
    sheet items); ``troubled`` names the sheets where a frame would not
    render (it prints its placeholder, the rest goes on)."""
    source = view_widget.source_viewport
    scene = source.scene
    contents, pictures, profiles, troubled = {}, {}, {}, []
    for comp in comps:
        if ground is not None:
            profiles.update(ground.profiles(comp, scene) or {})
        for frame in comp.frames:
            key = frame._doc_key
            try:
                if saved_view(scene, frame) is False:
                    contents[key] = {"missing": frame.view_key[6:]}
                elif frame.style == "vectorial" and not getattr(frame, "perspective", False):
                    got = drawing(source, frame)
                    if got is not None:
                        contents[key] = got
                else:
                    got = picture(view_widget, frame)
                    if got is not None:
                        contents[key] = {"image": got[:2]}
                        pictures[f"frame:{key}"] = got
                if key in contents and "missing" not in contents[key]:
                    contents[key]["annots"] = annotations(source, frame, providers)
            except Exception:  # noqa: BLE001 — one frame, not the document
                import traceback
                traceback.print_exc()
                if comp.name not in troubled:
                    troubled.append(comp.name)
    return contents, pictures, profiles or None, troubled


def export_dxf(source, frame, path) -> int:
    """``frame``'s hidden-line view as DXF at its scale, in model units (the
    composer's ``export_frame_dxf``): each model layer as a DXF layer of its
    own with its colour and line type when the model has layers, else one
    per line class; the cut's hatches and fills as the frame draws them
    (formats.dxf_views). Returns the lines written, None when its view is
    gone."""
    from core.composition import frame_view_name, model_height_for_frame
    from core.hlr import hlr_drawing
    from core.layers import export_labels, layer_lines_wanted, line_styles
    from formats.dxf_out import save_dxf_layers
    from formats.dxf_views import dxf_cut_layers, dxf_groups_by_class, dxf_groups_by_layer
    got = _frame_pass(source, frame, True)
    if got is None:
        return None
    facade, camera, geometry = got
    hidden = bool(getattr(frame, "hidden_lines", False))
    by_layer = layer_lines_wanted(facade) or len(getattr(facade, "layers", ())) > 1
    d = hlr_drawing(facade, camera, geometry=geometry if by_layer else geometry[:4], layers=by_layer,
                    profiles=bool(getattr(frame, "profiles", True)), fills=False, hidden=hidden)
    layer = frame_view_name(frame).split(" — ")[0]
    texts = []
    if d.layers is not None:
        d.styles, d.labels = line_styles(facade, True), export_labels(facade)
        groups = dxf_groups_by_layer(d, layer, hidden, texts)
    else:
        groups = dxf_groups_by_class(d, layer, hidden)
    lines, solids = [], []
    if (getattr(frame, "cut_fill", "solid") or "solid") != "none":
        scale_n = float(frame.scale_n or 100.0)
        lines, solids = dxf_cut_layers(facade, camera, scale_n,
                                       frame.h_mm / model_height_for_frame(frame.h_mm, scale_n),
                                       max(0.2, float(getattr(frame, "cut_hatch_mm", 1.5) or 1.5) / 1.5),
                                       geometry=geometry)
    return save_dxf_layers(path, groups + lines, solids, texts)


def export_pdf(view_widget, comps, path, ground=None, doc_path=None, providers=None) -> list:
    """Sheets ``comps`` into the PDF at ``path`` (sheets::pdf); returns the
    names of the sheets where a frame would not render."""
    contents, pictures, profiles, troubled = frame_contents(view_widget, comps, ground, providers)
    NATIVE.sheet_pdf(str(path), list(comps), contents, pictures, profiles, view_widget.source_viewport.scene,
                     None if doc_path is None else str(doc_path))
    return troubled


def export_image(view_widget, comp, path, dpi=200.0, ground=None, doc_path=None, providers=None) -> list:
    """Sheet ``comp`` as a PNG or JPEG at ``dpi`` (sheets::raster)."""
    contents, pictures, profiles, troubled = frame_contents(view_widget, [comp], ground, providers)
    NATIVE.sheet_image(str(path), comp, float(dpi), contents, pictures, profiles,
                       view_widget.source_viewport.scene, None if doc_path is None else str(doc_path))
    return troubled


class Ground:
    """The samplers terrain profiles read (georef service): the DEM over the
    scene's datum, or the survey when it shows. ``changed`` is called when
    tiles land, for the page to be sent again."""

    def __init__(self, changed):
        self._changed = changed
        self._dem = self._survey = None

    def profiles(self, comp, scene):
        """``{item key: (profile, path name, message)}`` (sheets::profile)."""
        if not comp.perfiles:
            return None
        datum = getattr(scene, "georef", None)
        if self._dem is None or self._dem.datum is not datum:
            from georef.dem import DEMSampler
            self._dem = DEMSampler(datum)
            self._dem.changed.connect(self._changed)
        survey = getattr(scene, "photo_mesh", None)
        sampler = None
        if datum is not None and survey is not None and getattr(survey, "visible", False):
            if (self._survey is None or self._survey.mesh is not survey
                    or self._survey.datum is not datum):
                from georef.photomesh import PhotoMeshSampler
                self._survey = PhotoMeshSampler(survey, datum)
            sampler = self._survey
        return NATIVE.sheet_terrain_profiles(comp, scene, self._dem, sampler)


def capture_hold(scene, item, attr, segment=None) -> bool:
    """Hold ``item.<attr>`` (a model point a sheet tool caught) by the model
    element under it, so it follows the model however far that moves: the
    edge it lies along, else the vertex (the model's own attachments,
    core.annotation_anchor; the composer's ``_follow_anchor`` capture)."""
    from ingetrazo_core import Vec3
    from core.annotation_anchor import Attachment, edge_by_endpoints
    from core.dimension import resolve_vertex_anchor
    cur = getattr(item, attr, None)
    if not cur:
        return False
    slot = "hold_" + attr[0]
    if segment is not None:
        hit, owner = edge_by_endpoints(scene, *(Vec3(*p) for p in segment))
        ref = Attachment.capture(scene, hit, owner, Vec3(*cur)) if hit is not None else None
        if ref is not None:
            setattr(item, slot, (tuple(cur), ref))
            return True
    hold = resolve_vertex_anchor(scene, Vec3(*cur))
    if hold is not None:
        owner = hold.chain[-1] if hold.chain else None
        ref = Attachment.capture(scene, hold.vertex, owner, Vec3(*cur))
        if ref is not None:
            setattr(item, slot, (tuple(cur), ref))
            return True
    return False


def frame_uid(frame) -> str:
    """The frame's uid, made when it has none (what a bound item names)."""
    import uuid
    if not frame.uid:
        frame.uid = uuid.uuid4().hex
    return frame.uid


def anchor(item, frame, scene, points) -> None:
    """Bind ``item`` to ``frame``'s model: its uid (made when it has none),
    each ``(attr, picked point)`` as a world point with its hold."""
    item.anchor_uid = frame_uid(frame)
    for attr, pt in points:
        setattr(item, attr, [float(v) for v in pt["world"]])
        if attr in ("a_world", "b_world"):
            capture_hold(scene, item, attr, pt.get("segment"))


def level_at(source, frame, x, y):
    """The model height a page point stands at in ``frame`` (the composer's
    ``frame_level_at``): in an elevation or a horizontal section the page's
    vertical is the model's Z; a plan or an axonometric cannot tell (None)."""
    from core.composition import model_height_for_frame
    from core.hlr import camera_basis
    if getattr(frame, "perspective", False) or frame.h_mm <= 0:
        return None
    scene = source.scene
    view = saved_view(scene, frame)
    if view is False:
        return None
    camera = frame_camera(source.camera, frame, view, scene)
    eye, right, up, fwd = camera_basis(camera)
    if abs(float(fwd[2])) > 1e-3:              # not a horizontal view
        return None
    half_h = model_height_for_frame(frame.h_mm, frame.scale_n) / 2.0
    if half_h <= 1e-9:
        return None
    k = frame.h_mm / (2.0 * half_h)
    half_w = half_h * (frame.w_mm / frame.h_mm)
    cx = (x - frame.x_mm) / k - half_w
    cy = half_h - (y - frame.y_mm) / k
    return float(eye[2] + right[2] * cx + up[2] * cy)


def follow_hold(scene, item, attr, holds) -> bool:
    """Carry ``item.<attr>`` along with the model element it is held by (the
    composer's ``_follow_anchor`` without the capture): its own hold when it
    has one for this point, else a session hold on the vertex under it,
    kept in ``holds``. True when a hold answered (moved or not)."""
    from ingetrazo_core import Vec3
    from core.annotation_anchor import Attachment
    from core.dimension import resolve_vertex_anchor
    cur = getattr(item, attr, None)
    if not cur:
        return False
    slot = "hold_" + attr[0]
    saved = getattr(item, slot, None)
    if saved is not None and tuple(saved[0]) == tuple(cur):
        ref = saved[1]
        if isinstance(ref, dict):
            ref = Attachment.restore(scene, ref, Vec3(*cur))
        point = ref.position() if ref is not None else None
        if point is not None:
            now = point.toTuple()
            if tuple(cur) != now:
                setattr(item, attr, list(now))
            if tuple(cur) != now or ref is not saved[1]:
                setattr(item, slot, (now, ref))
            return True
    key = (item._doc_key, attr)
    held = holds.get(key)
    if held is None or held[0] != list(cur):
        hold = resolve_vertex_anchor(scene, Vec3(*cur))
        holds[key] = (list(cur), hold)
        return hold is not None
    hold = held[1]
    if hold is None:
        return False
    p = hold.position(scene)
    if p is None:                     # erased or welded away: let go
        holds[key] = (list(cur), None)
        return False
    now = [float(p.x()), float(p.y()), float(p.z())]
    if now != list(cur):
        setattr(item, attr, now)
        holds[key] = (now, hold)
    return True


def follow_anchors(source, comp, memo, holds) -> int:
    """Every item of sheet ``comp`` bound to a frame follows the model
    (sheets::anchors): its hold, else the frame's current snap points,
    then reprojected through its frame. Returns how many moved."""
    scene = source.scene
    base, bounds = camera_inputs(source.camera, scene)
    frames = []
    for frame in comp.frames:
        view = saved_view(scene, frame)
        if view is False or not frame.uid:
            continue
        frames.append({"frame": frame, "view": view, "base": base, "bounds": bounds})

    def snaps_of(frame):
        got = snaps(source, frame)
        return None if got is None else (got[1], got[3])
    return NATIVE.sheet_follow_anchors(comp, frames, snaps_of, lambda item, attr: follow_hold(scene, item, attr, holds),
                                       memo)
