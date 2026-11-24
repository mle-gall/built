# SPDX-License-Identifier: GPL-3.0-or-later
"""Viewport picks and shared toolbar/tray brush state for the Rust Paint tool."""
from core.i18n import tr
from core.mesh import Face
from core.paint_ops import _face_plane, paint_stamp
from core.style import DEFAULT_FACE_COLOR
from ingetrazo_core import Vec3

#: How many materials the «Recent» row keeps.
RECENT_MAX = 6


def recent_key(entry: dict) -> tuple:
    """What makes two recent entries THE SAME material: its name when it
    has one, else the image with its size and turn (or the colour)."""
    if entry.get("mat"):
        return ("m", entry["mat"])
    tex = entry.get("texture")
    if tex:
        tint = tex.get("tint")
        return ("t", tex.get("path"), tex.get("sw"), tex.get("sh"),
                tex.get("rot", 0.0), tuple(tint) if tint else None,
                entry.get("opacity"))
    col = entry.get("color") or ()
    return ("c", tuple(round(float(c), 4) for c in col),
            entry.get("opacity"))


def push_recent(entries: list, entry: dict, cap: int = RECENT_MAX) -> list:
    """``entries`` with ``entry`` moved (or added) to the front, without
    duplicates and at most ``cap`` long."""
    key = recent_key(entry)
    rest = [e for e in entries if recent_key(e) != key]
    return [entry] + rest[:max(cap - 1, 0)]


def rename_recent(entries: list, old: str, stamp: dict) -> list:
    """``entries`` with every one named *old* now wearing *stamp* (the
    new material's face attrs): a material renamed or recoloured in the
    model is not offered back under its old name and colour."""
    out = []
    for e in entries:
        if e.get("mat") == old:
            e = {k: v for k, v in e.items()
                 if k not in ("mat", "color", "texture", "opacity", "pbr")}
            e.update({k: (list(v) if isinstance(v, tuple) else v)
                      for k, v in stamp.items()})
        out.append(e)
    return out


def clicked_back_side(viewport, face, group, x: float, y: float) -> bool:
    """Whether the cursor at ``(x, y)`` sees the BACK of ``face``: the
    pick ray runs along the face's world normal instead of against it.
    ``group`` is what ``pick_face_any`` returned beside the face (its
    transform places the normal). A viewport without a ray (the stubs in
    tests) reads as the front."""
    from ingetrazo_core import Vec3 as QVector3D
    ray = getattr(viewport, "_pixel_to_ray", None)
    if ray is None:
        return False
    origin, direction = ray(x, y)
    if origin is None or direction is None:
        return False
    from core.snap import face_plane_world
    _p, normal = face_plane_world(face, getattr(group, "xform", None))
    return QVector3D.dotProduct(normal, direction) > 0.0


class PaintHost:
    _READ_DURING = ()
    # Shared current paint colour (RGB, 0..1), set from the toolbar swatch.
    current_color: tuple[float, float, float] = (0.80, 0.45, 0.30)
    # Shared current texture ({"path","sw","sh"}) or None. When set, the click
    # applies a texture instead of a colour — chosen from the toolbar.
    current_texture: dict | None = None
    # Shared current material IDENTITY (core.materials.Material) or None.
    # Set by the tray when the active swatch has a name; painting then
    # stamps attrs["mat"] alongside the colour/texture (and registers the
    # material in the scene on first use). None = anonymous paint, which
    # CLEARS any previous identity — a red face is no longer "Concreto
    # visto", so the per-material takeoff never lies.
    current_material = None
    # Shared translucency (glass): None = opaque paint, which also CLEARS
    # any previous opacity on the painted faces.
    current_opacity: float | None = None
    # Armed by the toolbar's eyedropper button: the NEXT click samples
    # instead of painting, then disarms. The pipette sits
    # beside the material — Alt works for people who know it, the button is
    # how you find it.
    sample_armed: bool = False
    # Plane the current texture's explicit ``uvw`` belongs to, as
    # ``(normal, offset)`` — set when the eyedropper samples a face that
    # carries one. Only faces on that plane inherit the map; see the module
    # docstring. ``None`` = the texture has no map of its own to hand on.
    current_texture_plane: tuple | None = None
    # The «Default» material (no material): the eyedropper picks it
    # up from an unpainted side, the tray offers it, and painting with it
    # REMOVES the paint (@pacaeiro, #47 point 2). While it is set the
    # colour/texture fields above are ignored.
    current_is_default: bool = False
    # The last materials painted with, newest first, as material dicts
    # (``_current_as_material``). The tray loads them from the settings,
    # shows them and saves them back (``recent_materials_changed``).
    recent: list[dict] = []

    @staticmethod
    def _container_under(vp, group, x: float, y: float):
        """The group or component a click at ``(x, y)`` paints as a whole —
        a face inside a container reached from OUTSIDE it — or ``None``
        when the click paints the face. The hover and the click both ask
        here, so the marking always shows what the click will paint."""
        if group is None:
            return None
        pick_obj = getattr(vp, "pick_group", None)
        return pick_obj(x, y) if pick_obj is not None else None

    @staticmethod
    def _every_face_painted(obj) -> bool:
        """Whether every face of the group or component — nested
        placements included — carries its own material, so painting it as
        a whole shows no change."""
        from core.group import iter_placements
        from core.materials import has_own_material
        from core.mesh import face_attrs
        seen = False
        for g, _m in iter_placements(obj):
            for attrs in face_attrs(g.mesh):     # builds no lazy mesh (#1092)
                seen = True
                if not has_own_material(attrs):
                    return False
        return seen

    @classmethod
    def _remember_current(cls, vp=None) -> None:
        """Put the material just painted at the head of ``recent`` and let
        the tray know. The «Default» material is not one: painting with it
        takes paint off."""
        if cls.current_is_default:
            return
        before = cls.recent
        cls.recent = push_recent(cls.recent, cls._current_as_material())
        if cls.recent == before:
            return
        win = vp.window() if hasattr(vp, "window") else None
        panel = getattr(getattr(win, "tray", None), "materials", None)
        if hasattr(panel, "recent_materials_changed"):
            panel.recent_materials_changed()

    @classmethod
    def _current_as_material(cls):
        return paint_stamp(cls.current_color, cls.current_texture, cls.current_opacity, cls.current_material)

    @classmethod
    def _back_material_for(cls, face):
        return paint_stamp(cls.current_color, cls.current_texture, cls.current_opacity,
                           cls.current_material, cls.current_texture_plane, face)

    def _input(self, kind, ctx):
        try:
            return self._send(ctx.viewport, self._pointer(kind, ctx))
        finally:
            self._face = self._owner = self._container = None

    def on_hover(self, ctx):
        self._input("hover", ctx)

    def on_click(self, ctx):
        if self._input("click", ctx)["redraw"]:
            ctx.viewport.update()

    def host_sampler_armed(self, vp, _):
        return bool(self.sample_armed or getattr(vp, "_alt_down", False))

    def host_probe_container(self, vp, px):
        from views.array_bake import reading_only
        with reading_only():
            self._face, self._owner = vp.pick_face_any(*px)
        self._container = self._container_under(vp, self._owner, *px)
        return self._container is not None and self._face is not None

    def host_pick_paint_face(self, vp, px):
        self._face, self._owner = vp.pick_face_any(*px)
        return self._face is not None

    def host_clear_paint_hover(self, vp, _):
        vp.set_hover(None)

    def host_mark_paint_face(self, vp, px):
        pick = getattr(vp, "pick_face_placement", None)
        if pick is not None:
            vp.set_hover(*pick(*px))
        else:
            vp.set_hover(vp.pick_face_any(*px)[0])

    def host_sample_image(self, vp, px):
        if self._face is None:
            pick = getattr(vp, "image_plane_at", None)
            image = pick(*px) if pick else None
        else:
            pick = getattr(vp, "image_in_front_of", None)
            image = pick(self._face, *px) if pick else None
        if image is None:
            return None
        tex = image.texture()
        n = image.normal()
        opacity = float(getattr(image, "opacity", 1.))
        return {"attrs": {"texture": tex, "opacity": opacity if opacity < 1. else None},
                "plane": (n, Vec3.dotProduct(n, image.origin)) if tex.get("uvw") else None}

    def host_sample_face(self, vp, px):
        from core.group import effective_material
        from core.materials import effective_attrs
        face = self._face
        front = effective_attrs(face.attrs, effective_material(self._owner))
        # Only material fields cross the bridge; extension attrs can contain arbitrary objects.
        fields = ("color", "texture", "opacity", "pbr", "mat")
        back = face.attrs.get("back")
        return {"front": {k: front[k] for k in fields if k in front},
                "back": {k: back[k] for k in fields if k in back} if isinstance(back, dict) else back,
                "back_side": clicked_back_side(vp, face, self._owner, *px),
                "plane": _face_plane(face)}

    def host_adopt_sample(self, vp, sample):
        from core.materials import has_own_material
        cls = type(self)
        src = sample["attrs"] or {}
        src = src if isinstance(src, dict) else {}
        cls.current_is_default = not has_own_material(src)
        cls.current_texture = src.get("texture")
        plane = sample["plane"]
        cls.current_texture_plane = (Vec3(*plane[0]), plane[1]) if plane else None
        if cls.current_texture is None:
            cls.current_color = tuple(src.get("color") or DEFAULT_FACE_COLOR)
        cls.current_opacity = src.get("opacity")
        cls.current_material = vp.scene.materials.get(src["mat"]) if src.get("mat") else None
        win = vp.window() if hasattr(vp, "window") else None
        if cls.sample_armed:
            cls.sample_armed = False
            if hasattr(win, "release_eyedropper"):
                win.release_eyedropper()
        panel = getattr(getattr(win, "tray", None), "materials", None)
        if hasattr(panel, "sync_from_paint"):
            panel.sync_from_paint()
        if callable(flash := getattr(vp, "flash_status", None)):
            flash(tr("Material sampled — click a face to paint it"))

    def host_brush(self, vp, _):
        from ingetrazo_core import refresh_library_paint
        from core.material_commands import material_for_paint
        brush = type(self)
        refresh_library_paint(vp.scene, brush)
        if not brush.current_is_default and brush.current_material is None:
            brush.current_material = material_for_paint(
                vp.scene, color=None if brush.current_texture is not None else brush.current_color,
                texture=brush.current_texture, opacity=brush.current_opacity)
        return {"material": brush.current_material.to_dict() if brush.current_material else None,
                "default": brush.current_is_default, "color": brush.current_color,
                "texture": brush.current_texture, "opacity": brush.current_opacity,
                "plane": brush.current_texture_plane if brush.current_texture is not None else None}

    def host_paint_targets(self, vp, args):
        from core.protocol.keys import mesh_key
        px, container = args
        if container:
            return {"placement": self._container.uid}
        mesh = self._owner.mesh if self._owner is not None else vp.scene.mesh
        selected = [f for f in vp.scene.selection if isinstance(f, Face)]
        faces = selected if self._face in selected else mesh.surface_of(self._face)
        return {"mesh": mesh_key(mesh), "seed": str(self._face.key), "faces": [str(f.key) for f in faces],
                "back": clicked_back_side(vp, self._face, self._owner, *px)}

    def host_container_painted(self, vp, _):
        flash = getattr(vp, "flash_status", None)
        if callable(flash):
            name = getattr(self._container, "name", "") or tr("Group")
            if self._every_face_painted(self._container):
                flash(tr("{name}: every face has its own material, so nothing changed — open "
                         "it and paint the faces", name=name), 10000)
            else:
                flash(tr("{name} painted as a whole; a face that already had its own material "
                         "keeps it", name=name))

    def host_remember_paint(self, vp, _):
        self._remember_current(vp)
