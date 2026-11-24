# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Scale grip picking and resident-row previews for the Rust machine."""
from __future__ import annotations
from ingetrazo_core import register_tool_adapter

from core.input_types import Qt
from ingetrazo_core import MatrixGesture, ScaleGripSpec, scale_grab, Vec3 as QVector3D

from core.i18n import tr


# Below this extent an axis is flat: it gets no grips and never scales
# (a 2D face shows the 8-grip box instead of a degenerate 26).
_FLAT = 1e-6
_AXIS_NAMES = ("Red", "Green", "Blue")     # X east, Y north, Z up (Z-up)


@register_tool_adapter
class _Grip:
    """One scaling grip: box-parameter position + the axes it scales."""

    __slots__ = ("params", "mask")

    def __init__(self, params: tuple, mask: tuple) -> None:
        self.params = params               # (tx, ty, tz), each 0 / 0.5 / 1
        self.mask = mask                   # axis indices this grip scales

@register_tool_adapter
class ScaleHost:
    # The VCB tags unit-suffixed entries for us ("2m" = absolute size), so a
    # bare "2" can mean ×2, the usual reading.
    accepts_absolute_length = True

    def __init__(self) -> None:
        super().__init__()
        # Box + grips, in the coordinates of the box's frame (see
        # ``_pick_frame``): the world's unless the selection has its own.
        self._lo: QVector3D | None = None
        self._hi: QVector3D | None = None
        self._axes = None                  # (red, green, blue) or None = world
        self._gesture = MatrixGesture()
        self._orig = None
        self._grips: list[_Grip] = []
        self._box_version = -1
        # Targets resolved at grab time.
        self._groups: list = []
        self._positions: list[QVector3D] = []
        self._verts: list = []
        self._images: list = []
        # The operation in flight.
        self._grip: _Grip | None = None
        self._hover_grip: _Grip | None = None
        self._anchor: QVector3D | None = None
        # A tap toggles.
        self.about_center = False
        self.uniform = False

    #: Half the side of a drawn grip square, in pixels.
    _GRIP_HALF_PX = 5.0

    #: What a held grip may land on: named points and edges, projected onto
    #: the grip's line. Not a face (the grip would jump across every
    #: surface it passes) and not the engine's own directional inferences.
    _TARGET_KINDS = frozenset({
        "endpoint", "midpoint", "intersection", "intersection_edge",
        "intersection_face", "center", "origin",
        "on_edge", "on_line", "from_point", "close", "guide",
        "component_origin", "arc_midpoint", "face_center"})

    _factors = property(lambda self: self._gesture.factors)
