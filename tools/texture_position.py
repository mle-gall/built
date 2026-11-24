# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Position Texture: the classic fixed pins on one face (Rafael's review of
2026-09-10, C1: «no encontré manera de cambiar la escala a la textura»).

Right-click a textured face ▸ Texture ▸ Position. The image shows with a
dotted tile grid and four pins on the corners of the tile under the cursor:

* **red** — drag to MOVE the texture (dragging the texture itself does the
  same);
* **green** — drag to SCALE and ROTATE about the red pin. A small
  protractor appears on the red pin while you drag (Rafael, 2026-09-16:
  «te bloquea a los 0, a los 45 y a los 90… si no es como un poco a ojo»;
  Marco brought captures and a recording): a small disc of
  fixed screen size with the start arm across it, the wedge swept, a
  square on each arm, the current arm dashed through the green pin, and
  the angle in the Measurements box. The rotation snaps to 15° steps
  from where the drag began (0, 15, 30, 45… 90 — Rafael's 0/45/90 are
  among them); Ctrl while dragging turns the snap off («Ctrl
  = Sin ajuste»);
* **blue** — drag to SCALE vertically and SHEAR (red and green stay);
* a pin dragged or set down near a corner of the face or the middle of
  one of its edges holds there — with a mark on the
  point, as the drawing tools show it (a YouTube review of 0.6.1, #1308:
  «the texture map placement anchors don't snap to vertices or
  corners»). Ctrl lets go, as it frees the rotation;
* **yellow** — perspective distort. The engine maps textures
  with an affine map per face (what every exporter writes), so this pin is
  shown but not draggable yet.

A single click on a pin LIFTS it: it floats with the cursor, without
changing the texture, and the next click sets it down — the way to put the
red pin on a corner of the face before scaling from there. Right-click
inside the tool: Done, Reset, Flip, Rotate, Undo. Enter or a click outside
the face finishes; Esc first restores the position (and stays), a second
Esc leaves without changes.

The result is the face's world→UV affine map (``attrs["texture"]["uvw"]``,
see :func:`core.texture.face_uv_axes`): the viewport, the .skp/.dae/.obj
writers and the paint sampler all read that one map, so what is placed
here is what any .skp reader shows for the same file.
"""
from __future__ import annotations

from tools.overlay_painter import draw_overlay
from PySide6.QtCore import Qt
from ingetrazo_core import TextureMap, TexturePlacement, Vec3 as QVector3D

from core.i18n import tr
from tools.base import Tool, ToolContext

PIN_MOVE, PIN_SCALE_ROTATE, PIN_SCALE_SHEAR, PIN_DISTORT = range(4)

#: RGB of each pin, in the usual order: red, green, blue, yellow.
PIN_COLORS = ((0.86, 0.22, 0.27), (0.16, 0.62, 0.36),
              (0.20, 0.40, 0.78), (0.95, 0.78, 0.20))
PIN_NAMES = ("Move", "Scale / Rotate", "Scale / Shear", "Distort")


class TexturePositionTool(Tool):
    name = "Position Texture"
    icon = "texture_position"
    uses_snap = False
    #: The viewport shows this Qt cursor while the tool is active (a
    #: hand) — the tool has no drawn icon cursor.
    qt_cursor = Qt.OpenHandCursor
    #: Opacity of the live preview (the texture shows translucent
    #: with the dotted tile grid over it).
    preview_opacity = 0.75
    #: Pixel radius that grabs a pin.
    GRAB_PX = 12.0
    #: Press/release closer than this is a click (lifts a pin), not a drag.
    CLICK_PX = 4.0
    #: Pin half-size, px.
    PIN_PX = 7.0
    #: A small protractor on the red pin while the green one
    #: drags: a disc of fixed SCREEN radius. The rotation snaps to 15°
    #: steps from the drag's start WHEREVER the cursor is — Marco's
    #: recording (2026-09-18) shows the texture jumping in steps with the
    #: cursor three radii out, and the tool's own status line says «Ctrl
    #: = Sin ajuste», the escape hatch that only makes sense if the snap
    #: is the default everywhere. (Rotate's near/far rule does not apply.)
    DISC_PX = 30.0
    TICK_DEG = 15.0
    #: How near on screen a pin's spot must come to a corner or an edge
    #: middle of the face to hold to it. Not to the edges themselves: the
    #: green pin's protractor steps would lose to them along every side.
    SNAP_PX = 10.0

    def __init__(self) -> None:
        self.face = None
        self.side = "front"
        self._tex0: dict | None = None
        self._placement = None
        self._plane = None

    @property
    def map(self):
        return self._placement.map if self.active else None

    @property
    def pin_uv(self):
        return self._placement.pin_uv if self.active else []

    @property
    def gesture_active(self):
        return self.active and self._placement.gesture_active

    @property
    def undo_depth(self):
        return self._placement.undo_depth if self.active else 0

    @property
    def _lifted(self):
        return self._placement.lifted if self.active else None

    @property
    def _lifted_pos(self):
        return self._placement.lifted_pos if self.active else None

    @property
    def _held(self):
        return self._placement.held if self.active else None

    @property
    def _sweep_deg(self):
        return self._placement.sweep_deg if self.active else None

    @property
    def _on_tick(self):
        return self.active and self._placement.on_tick

    # ---- Entry ----------------------------------------------------------------
    @staticmethod
    def side_texture(face, side: str):
        """The image texture dict of ``face``'s ``side`` ("front"/"back"),
        or ``None``. A back painted as a mirror of the front (``True``)
        has no texture of its own."""
        attrs = getattr(face, "attrs", None) or {}
        if side == "back":
            back = attrs.get("back")
            tex = back.get("texture") if isinstance(back, dict) else None
        else:
            tex = attrs.get("texture")
        if not tex or not tex.get("path"):
            return None
        return tex

    def begin(self, viewport, face, at: QVector3D | None = None,
              side: str = "front") -> bool:
        """Start positioning the texture of ``face``'s ``side`` with the
        pins on the tile under ``at``. Returns False when that side has no
        image texture. The back side projects with the same plane basis as
        the front (the renderer builds both from the face normal), so one
        map serves either; only where it is stored differs."""
        tex = self.side_texture(face, side)
        if tex is None:
            return False
        self.side = side
        m = TextureMap.from_face(face, tex, at)
        if m is None:
            return False
        self.face = face
        self._tex0 = dict(tex)
        normal = face.normal().normalized()
        self._plane = (QVector3D(face.vertices[0]), normal)
        self._placement = TexturePlacement(m, normal)
        suppress = getattr(viewport, "set_suppressed_faces", None)
        if suppress is not None:
            suppress({face})
        flash = getattr(viewport, "flash_status", None)
        if flash is not None:
            flash(tr("Drag the texture or a pin — red moves, green scales and "
                     "rotates, blue scales and shears. Enter finishes."), 6000)
        update = getattr(viewport, "update", None)
        if update is not None:
            update()
        return True

    @property
    def active(self) -> bool:
        return self._placement is not None

    @property
    def dragging(self) -> bool:      # keeps Esc inside the tool (viewport rule)
        return self.active

    # ---- Lifecycle ------------------------------------------------------------

    def on_activate(self, viewport) -> None:
        """The picked face enters placement through begin()."""

    def on_deactivate(self, viewport) -> None:
        self._leave(viewport)

    def _leave(self, viewport) -> None:
        suppress = getattr(viewport, "set_suppressed_faces", None)
        if suppress is not None:
            suppress(set())
        self.face = None
        self._placement = None
        self._tex0 = None
        self._plane = None

    def _exit(self, viewport) -> None:
        """Back to Select (the classic tool returns to the previous one)."""
        self._leave(viewport)
        win = viewport.window() if hasattr(viewport, "window") else None
        activate = getattr(win, "_activate_tool", None)
        if activate is not None:
            activate("select")
        else:
            set_tool = getattr(viewport, "set_active_tool", None)
            if set_tool is not None:
                set_tool(None)
        update = getattr(viewport, "update", None)
        if update is not None:
            update()

    # ---- Geometry helpers -----------------------------------------------------
    def pins(self) -> list[QVector3D]:
        return self._placement.pins() if self.active else []

    def _on_plane(self, viewport, screen) -> QVector3D | None:
        """The cursor's point on the face plane."""
        if self._plane is None or screen is None:
            return None
        ray = getattr(viewport, "_pixel_to_ray", None)
        hit = getattr(viewport, "_ray_plane", None)
        if ray is None or hit is None:
            return None
        origin, direction = ray(screen.x(), screen.y())
        if origin is None:
            return None
        return hit(origin, direction, self._plane[0], self._plane[1])


    def _on_face(self, viewport, screen) -> bool:
        pick = getattr(viewport, "pick_face", None)
        if pick is None or screen is None:
            return True
        return pick(screen.x(), screen.y()) == self.face

    # ---- Mouse ----------------------------------------------------------------
    def on_click(self, ctx: ToolContext) -> None:
        if not self.active:
            return
        vp = ctx.viewport
        project = getattr(vp, "_world_to_pixel", None)
        action = self._placement.click(
            (ctx.screen.x(), ctx.screen.y()) if ctx.screen is not None else None,
            self._on_plane(vp, ctx.screen), bool(ctx.modifiers & Qt.ControlModifier),
            project, self.face, self.GRAB_PX, self.SNAP_PX)
        if action == "update":
            vp.update()
        elif action == "distort":
            vp.flash_status(tr("Distort (perspective) is not available yet — "
                               "use the red, green and blue pins."), 4000)
        elif action == "pick_face" and not self._on_face(vp, ctx.screen):
            self.commit(vp)

    def on_hover(self, ctx: ToolContext) -> None:
        if self.active:
            vp = ctx.viewport
            self._placement.hover(
                (ctx.screen.x(), ctx.screen.y()) if ctx.screen is not None else None,
                self._on_plane(vp, ctx.screen), bool(ctx.modifiers & Qt.ControlModifier),
                getattr(vp, "_world_to_pixel", None), self.face,
                self.GRAB_PX, self.SNAP_PX, self.CLICK_PX, self.TICK_DEG)
            vp.update()

    def on_release(self, viewport) -> None:
        if self.active:
            action = self._placement.release()
            if action == "lifted":
                viewport.flash_status(tr("Pin lifted — click where it should sit."), 3000)
            elif action == "update":
                viewport.update()

    def value_label(self):
        """``(text, anchor)`` while the green pin rotates — the angle swept,
        as the Measurements box shows it; ``None`` otherwise."""
        if self._sweep_deg is None or not self.active:
            return None
        from core.units import fmt_angle_deg
        return (fmt_angle_deg(self._sweep_deg, signed=True),
                self.map.world(*self.pin_uv[PIN_SCALE_ROTATE]))

    def _protractor_segments(self, viewport):
        if self.active:
            return self._placement.protractor(getattr(viewport, "_world_to_pixel", None),
                                              self.DISC_PX)

    def rotate(self, degrees: float) -> None:
        if self.active:
            self._placement.rotate(degrees)

    def flip(self, horizontal: bool) -> None:
        if self.active:
            self._placement.flip(horizontal)

    def undo_step(self) -> bool:
        return self.active and self._placement.undo_step()

    def reset(self) -> None:
        if self.active:
            self._placement.reset()

    def changed(self) -> bool:
        return self.active and self._placement.changed()

    def result_texture(self) -> dict | None:
        """The texture dict the face gets on Done."""
        if self.map is None or self._tex0 is None:
            return None
        uvw = self.map.uvw()
        if uvw is None:
            return None
        tex = {k: v for k, v in self._tex0.items() if k != "rot"}
        tex["uvw"] = uvw
        return tex

    def commit(self, viewport) -> None:
        """Done: write the map to the face as one undoable step."""
        if not self.active:
            return
        preview = getattr(getattr(viewport, 'view', None), '_texture_preview', None)
        if preview is not None:
            preview.restore()
        if self.changed():
            tex = self.result_texture()
            if tex is not None:
                viewport.history.execute(self.side_command(self.face, self.side, tex))
        self._exit(viewport)

    @staticmethod
    def side_command(face, side: str, tex: dict | None):
        """The undoable command that puts ``tex`` on ``face``'s ``side``
        (``None`` clears the texture of that side)."""
        if side == "back":
            from core.history import SetFaceBackCommand
            back = dict(face.attrs.get("back") or {})
            if tex is None:
                back.pop("texture", None)
            else:
                back["texture"] = dict(tex)
            return SetFaceBackCommand([face], [back])
        from core.history import SetFaceTextureCommand
        return SetFaceTextureCommand([face], tex)

    def cancel(self, viewport) -> None:
        self._exit(viewport)

    def on_cancel(self, viewport) -> None:
        """Esc: the first press restores the entry placement and stays; a
        second one (or one with nothing to restore) leaves."""
        if not self.active:
            return
        action = self._placement.escape()
        if action == "exit":
            self.cancel(viewport)
        else:
            if action == "restored":
                viewport.flash_status(tr("Position restored — Esc again to leave."), 3000)
            viewport.update()

    def on_key(self, viewport, key: int, modifiers) -> bool:
        if not self.active:
            return False
        if key in (Qt.Key_Return, Qt.Key_Enter):
            self.commit(viewport)
            return True
        return False

    def context_menu(self, viewport, global_pos) -> bool:
        """The tool's own right-click menu (replaces the viewport's)."""
        if not self.active:
            return False
        from PySide6.QtWidgets import QMenu
        menu = QMenu(getattr(viewport, "keys_stand_in", None) or viewport)
        menu.addAction(tr("Done"), lambda: self.commit(viewport))
        menu.addAction(tr("Reset"), lambda: (self.reset(), viewport.update()))
        flip = menu.addMenu(tr("Flip"))
        flip.addAction(tr("Left / Right"), lambda: (self.flip(True), viewport.update()))
        flip.addAction(tr("Up / Down"), lambda: (self.flip(False), viewport.update()))
        rot = menu.addMenu(tr("Rotate"))
        for deg in (90, 180, 270):
            rot.addAction(f"{deg}°", lambda d=deg: (self.rotate(d), viewport.update()))
        undo = menu.addAction(tr("Undo"), lambda: (self.undo_step(), viewport.update()))
        undo.setEnabled(bool(self.undo_depth))
        menu.exec(global_pos)
        return True

    # ---- Preview --------------------------------------------------------------
    def preview_faces(self):
        if not self.active:
            return []
        tex = self.result_texture()
        if tex is None:
            return []
        from core.geometry import Face as PreviewFace
        return [PreviewFace(list(self.face.vertices),
                            [list(h) for h in self.face.holes], {"texture": tex})]

    def grid_lines(self) -> list[tuple[QVector3D, QVector3D]]:
        return self._placement.grid_lines(self.face.vertices) if self.active else []

    overlay_kind = "texture_position"
    draw_overlay = draw_overlay
