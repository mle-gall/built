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

    # A free face and a Ctrl-copy show the naive sweep as overlay faces
    # (``preview_faces``) plus its own wireframe (``rubber_band_lines``); the
    # commit replaces both with the real stitched geometry. A prism or staged
    # preview shows the model itself and leaves both empty.
    #: A stage (one pipeline run, snapshot work included) slower than this, in
    #: seconds, ends staging for the drag: leaving every parked interval shows
    #: the overlay, and a release outside them runs the pipeline as before.
    #: One pause per drag on a big model instead of one per reversal.
    _SLOW_STAGE_S = 0.25
