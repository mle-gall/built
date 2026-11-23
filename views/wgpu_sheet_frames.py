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


def frame_camera(live, frame, view, scene, origin=None):
    """An OrbitCamera at the frame's view whose matrices are the Rust ones
    (``origin``: the resident geometry's offset, subtracted)."""
    from ingetrazo_core import Matrix4, Vec3
    lo, hi = scene.bounds()
    bounds = None if lo is None else (lo.toTuple(), hi.toTuple())
    base = {k: getattr(live, k) for k in ("target", "distance", "yaw", "pitch", "up", "fov_deg",
                                          "perspective", "two_point", "shift_x", "shift_y",
                                          "roll_deg", "clip_near", "clip_far")}
    base["target"], base["up"] = live.target.toTuple(), live.up.toTuple()
    if bounds is not None:
        c = (np.add(bounds[0], bounds[1]) * 0.5)
        base["depth_reach"] = (tuple(c.tolist()), float(np.linalg.norm(np.subtract(bounds[1], bounds[0])) * 0.5))
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


def render(view_widget, frame, key) -> bool:
    """Render ``frame`` into the page texture ``key``; False when it has
    nothing to show (its saved view is gone)."""
    from core.composition import RENDER_DPI, pen_px
    source = view_widget.source_viewport
    scene = source.scene
    view = saved_view(scene, frame)
    if view is False:
        return False
    facade = frame_scene(scene, frame, view)
    style = frame_style(source, facade, frame)
    w_px, h_px = frame.render_px(RENDER_DPI)
    view_widget.render_image(
        w_px, h_px, sharp_shadows=True, page_key=key, scene=facade, style=style,
        camera=lambda origin: frame_camera(source.camera, frame, view, scene, origin),
        rows=lambda: frame_rows(view_widget, scene, frame, view),
        edge_px=pen_px(getattr(frame, "pen_edge_mm", 0.18), RENDER_DPI),
        profile_px=pen_px(getattr(frame, "pen_profile_mm", 0.35), RENDER_DPI))
    return True


def drawing(source, frame):
    """A vector frame's lines in paper mm, frame-local, and their kinds
    (``core.hlr`` KIND_*), or None when its saved view is gone."""
    from core._native import qt_hypot
    from core.composition import model_height_for_frame
    from core.hlr import hlr_drawing
    scene = source.scene
    view = saved_view(scene, frame)
    if view is False:
        return None
    facade = frame_scene(scene, frame, view)
    camera = frame_camera(source.camera, frame, view, scene)
    geometry, _tags = NATIVE.sheet_frame_geometry(scene._root, list(scene.groups), scene.loose_mesh,
                                                  view or None, qt_hypot() or "libc++-fma")
    d = hlr_drawing(facade, camera, geometry=geometry, profiles=bool(getattr(frame, "profiles", True)),
                    fills=(getattr(frame, "cut_fill", "solid") or "solid") != "none",
                    hidden=bool(getattr(frame, "hidden_lines", False)))
    model_h = model_height_for_frame(frame.h_mm, frame.scale_n)
    k = frame.h_mm / model_h                       # paper mm per metre
    half_h = model_h / 2.0
    half_w = half_h * (frame.w_mm / frame.h_mm)
    segs = np.asarray(d.segs, float).reshape(-1, 4)
    out = np.empty_like(segs)
    out[:, 0::2] = (segs[:, 0::2] + half_w) * k
    out[:, 1::2] = (half_h - segs[:, 1::2]) * k
    return out, np.asarray(d.kinds, np.int8)


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
