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
from ingetrazo_core import register_tool_adapter

from tools.overlay_painter import draw_overlay
from core.input_types import Qt
from ingetrazo_core import TextureMap, TexturePlacement, Vec3 as QVector3D

from core.i18n import tr
from tools.base import Tool, ToolContext

PIN_MOVE, PIN_SCALE_ROTATE, PIN_SCALE_SHEAR, PIN_DISTORT = range(4)

#: RGB of each pin, in the usual order: red, green, blue, yellow.
PIN_COLORS = ((0.86, 0.22, 0.27), (0.16, 0.62, 0.36),
              (0.20, 0.40, 0.78), (0.95, 0.78, 0.20))
PIN_NAMES = ("Move", "Scale / Rotate", "Scale / Shear", "Distort")


@register_tool_adapter
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


    def context_menu(self, viewport, global_pos) -> bool:
        """The tool's own right-click menu (replaces the viewport's)."""
        if not self.active:
            return False
        from core.input_types import NATIVE
        rows = self._placement.menu_rows()
        if NATIVE:
            from ingetrazo_render import ViewportMenu, context_menu_send
            view = viewport.view
            pos = view.mapFromGlobal(global_pos)
            context_menu_send(view, ViewportMenu.for_tool(
                self, tr("Position Texture"),
                [(tr(group) if group else "", [(key, tr(label), on) for key, label, on in items]) for group, items in rows],
                (pos.x(), pos.y())))
        else:
            from PySide6.QtWidgets import QMenu
            menu = QMenu(getattr(viewport, "keys_stand_in", None) or viewport)
            for group, items in rows:
                section = menu.addMenu(tr(group)) if group else menu
                for key, label, on in items:
                    action = section.addAction(tr(label), lambda checked=False, key=key: self.menu_action(viewport, key))
                    action.setEnabled(on)
            menu.exec(global_pos)
        return True

    overlay_kind = "texture_position"
    draw_overlay = draw_overlay
