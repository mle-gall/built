# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Host picking, protractor inference and resident-row preview for Rotate."""
from __future__ import annotations

from ingetrazo_core import register_tool_adapter

from core.input_types import Qt
from ingetrazo_core import CopyWire, MatrixGesture, Vec3 as QVector3D

from core.group import Group
from core.history import rotation_matrix

from core.i18n import tr
from tools.move_host import CopyPreview

from tools.protractor_host import ProtractorBase


@register_tool_adapter
class RotateHost(CopyPreview, ProtractorBase):
    accepts_angle_ratio = True  # VCB "3:12" (rise:run) arrives as degrees
    accepts_array = True  # VCB "3x" / "/3" after a copy: a POLAR array

    def __init__(self) -> None:
        super().__init__()
        self._groups: list[Group] = []
        self._splanes: list = []               # section planes being rotated
        self._images: list = []                # reference images being rotated
        self._moved_guides: list = []                # selected construction guides
        self._positions: list[QVector3D] = []
        self._verts: list = []
        self._sel_faces: list = []          # for copy mode (loose geometry)
        self._sel_edges: list = []
        self._base_segments = CopyWire()      # wireframe for the copy preview
        self._gesture = MatrixGesture()
