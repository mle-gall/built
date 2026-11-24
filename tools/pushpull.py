# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Push/Pull tool: extrude a face along its normal.

UX (the usual push/pull convention):
- Hover a face; the cursor picks the front-most face under it.
- First click: lock onto that face and start a drag along its normal.
- Subsequent mouse motion slides the extrusion preview (wireframe of
  the future box) along the normal axis. Length is shown near the
  midpoint, same overlay as the line tool.
- Second click commits at the current distance.
- Typing a number + Enter (VCB) commits at exactly that distance,
  preserving the current direction's sign; a negative value ("-2")
  reverses it, and unit suffixes ("30cm", "1500mm") are understood.
- **Ctrl** = push/pull a copy: the start face stays in place (a slab
  division) and the extrusion stacks as a new segment — how floors are
  stacked.
- **Double-click** repeats the last committed distance on the face under
  the cursor (with Ctrl, as a copy).
- **Several faces selected**: pushing or pulling any one of them moves them
  all the same distance, each along its own normal, as one undo step.
- Esc cancels without committing.

Commit pipeline (the deterministic root-fix, no case tree):
1. Orient the mesh outward (entry invariant; no-op on open sheets).
2. Build the *naive* extrusion: consumed/kept base, moved cap (with holes),
   one wall quad per boundary/hole edge — even where a quad lands on an
   existing face's plane.
3. Stitch connectivity (T-junctions, collinear vertices).
4. On a solid: deterministically rebuild every touched plane (seams, overlaps,
   cracks) from its edges via ``Store::rebuild_push_planes``; on a flat sheet, fall back
   to the seeded coplanar merge (no outward side exists there).
5. Orient outward again — every committed solid upholds the invariant.

A bordered (embedded) base face is consumed either direction — pushing in
carves a recess / step / through-hole (the far face is punched when the push
reaches it); pushing out grows the solid. A free-standing face keeps its base
as the cap of the new box.
"""
from __future__ import annotations
from ingetrazo_core import register_tool_adapter

import math

from core.input_types import Qt
from ingetrazo_core import Vec3 as QVector3D, push_reference_target
from views import input_state

from core.i18n import tr
from core.mesh import _STITCH_TOL
from core.mesh import Face as MeshFace
from core.orient import orient_outward
from core.topology import _key
from tools.base import ToolContext, ctrl_clause
from tools.rust_tools import RustTool, _mods
from core.units import fmt_len
from core import push_scans
from core.push_pull import (  # noqa: E402,F401  (moved; tests and extensions import them here)
    PushPullOperation,
    _GroupScene,
    _MIN_EXTRUDE,
    _face_shape,
    _loops_overlap_2d,
    _made_since,
    _paint_of,
    _project_loop_2d,
)


@register_tool_adapter
class PushPullTool(PushPullOperation, RustTool):
    kind = "pushpull"
    _READ_DURING = ()

    def __init__(self):
        RustTool.__init__(self)
        PushPullOperation.__init__(self)
        self._ctx = None

    def on_click(self, ctx):
        self._pointer_event("click", ctx)

    def on_hover(self, ctx):
        self._pointer_event("hover", ctx)

    def on_double_click(self, ctx):
        self._pointer_event("double_click", ctx)

    def on_release(self, viewport):
        mods = _mods(input_state.modifiers(viewport, query=True)) if self.dragging else 0
        self._send(viewport, {"type": "release", "mods": mods})

    name = "Push / Pull"

    #: P is the usual key for this tool (Marco, 2026-09-10). U, the one
    #: IngeTrazo used until today, stays as a second
    #: shortcut on this same action — a decade of muscle memory is worth a
    #: line of code, and it costs nothing: P is what the card says.
    shortcut = "P"

    shortcut_alt = "U"

    description = (
        "Push or pull a face to extrude it into a solid or cut it "
        "back.")

    uses_snap = False  # picks a face to extrude; no snap markers

    hover_group_edges = True  # …but its distance infers to a group's edge too

    vcb_label = "Distance"

    # Preview lines in the normal edge colour, not the loose orange rubber band,
    # and depth-tested so the forming box hides its own back edges.
    wireframe_color = (0.13, 0.17, 0.23, 1.0)

    wireframe_depth_tested = True

    # The sweep is an OVERLAY, with no material around it to hide anything: a
    # recess drew all four of its walls, and the ones between the eye and the
    # pocket came out back-side-on, in back-face blue, over the shape the user
    # is forming (Marco, 2026-08-27). Real geometry never shows that because
    # the solid hides them. Culling the faces that point away is the same
    # result, and it costs a dot product each.
    preview_cull_back = True

    # ---- Spatial input ------------------------------------------------------
    def host_preview(self, viewport, args):
        self._preview_at(viewport, self._ctx)
        return self.dragging

    def host_close_retype(self, viewport, args):
        self._last = None

    def host_tiny_push(self, viewport, args):
        viewport.flash_status(tr("Too small a push — drag further or type a distance"), 4000)

    def _refuse_python_mesh(self, viewport) -> None:
        self.base_face = None
        self._extra_faces = []
        self._extra_normals = []
        viewport.flash_status(tr("Push refused: the geometry kernel could not take "
                                 "this mesh (left it unchanged)"), 6000)

    # ---- Visual preview -----------------------------------------------------
    # A free face and a Ctrl-copy show the naive sweep as overlay faces
    # (``preview_faces``) plus its own wireframe (``rubber_band_lines``); the
    # commit replaces both with the real stitched geometry. A prism or staged
    # preview shows the model itself and leaves both empty.
    def rubber_band_lines(self):
        return self._light_wires

    def preview_faces(self):
        return self._light_faces

    def _build_light_faces(self):
        from ingetrazo_core import push_light_faces
        return push_light_faces(self, _MIN_EXTRUDE)

    def _present(self, viewport, mode):
        from ingetrazo_core import present_push_preview
        present_push_preview(self, viewport, mode, _MIN_EXTRUDE, _STITCH_TOL)

    def _show_light_preview(self, viewport):
        self._present(viewport, "overlay")

    def _refuse_closed_group(self, viewport) -> None:
        """A face inside a group that has not been opened is not pushable.

        The tool used to push it anyway — an instance even opened a short
        editing session behind your back and shared the result to every copy
        (2026-06-10). In a real drawing that reads as
        the model changing where you did not point: a group is a box, and you
        open a box before reaching inside (Marco, 2026-09-10) — the usual
        convention: double-click the group or component to edit it first.

        Refusing has to SPEAK: a tool that does nothing without a word is the
        one that reads as broken.
        """
        viewport.flash_status(tr(
            "That face belongs to a group — double-click to open it first, "
            "then push inside"), 4000)

    def _show_prism_preview(self, viewport) -> None:
        self._present(viewport, "prism")

    #: A stage (one pipeline run, snapshot work included) slower than this, in
    #: seconds, ends staging for the drag: leaving every parked interval shows
    #: the overlay, and a release outside them runs the pipeline as before.
    #: One pause per drag on a big model instead of one per reversal.
    _SLOW_STAGE_S = 0.25

    def _show_staged_preview(self, viewport) -> None:
        self._present(viewport, "stage")

    # ---- Modifier: Ctrl keeps the starting face -----------------------------
    def on_key(self, viewport, key, modifiers):
        if key == Qt.Key_Control:
            return self._send(viewport, {"type": "key", "key": "Control", "mods": _mods(modifiers)})["consumed"]
        return super().on_key(viewport, key, modifiers)

    def on_key_release(self, viewport, key):
        if key != Qt.Key_Control:
            return False
        return self._send(viewport, {"type": "key_release", "key": "Control",
                                     "tapped": self.ctrl_tapped(viewport)})["consumed"]

    def host_modifier(self, viewport, values):
        self._keep_mode, self._keep_base = values

    def host_show_mode(self, viewport, enabled):
        viewport.flash_status(tr("Keep the base: on") if enabled else tr("Keep the base: off"))
        self.refresh_hint(viewport)
        viewport.update()

    def status_clause(self) -> str:
        # Short on purpose: the clause rides the same line as the hint, and
        # the classic bar spends a whole one on «Ctrl = Crear un nuevo punto
        # de partida». "Keep the base" / "move the base" is the same idea in
        # the tool's own words (base_face, _keep_base).
        return ctrl_clause(self._keep_mode, "keep the base", "move the base")
