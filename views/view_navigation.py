# SPDX-License-Identifier: GPL-3.0-or-later
"""Navigation shared by document tool input and Metal presentation."""
from __future__ import annotations

import ingetrazo_render
from typing import Optional

from core.input_types import QPointF, Qt
from ingetrazo_core import Vec3 as QVector3D


class ViewNavigation:
    _ZOOM_OUT_STEP = 0.05

    def _pixel_to_ray(self, x, y):
        return ingetrazo_render.view_ray(self, x, y)

    def _world_to_pixel(self, world, mvp=None):
        return ingetrazo_render.view_pixel(self, world, mvp)

    def _zoom_to_box(self, start: QPointF, end: QPointF) -> None:
        return ingetrazo_render.zoom_box(self, start, end)

    def _project_to_lock_line(
        self,
        start: QVector3D,
        lock_dir: QVector3D,
        pixel_x: float,
        pixel_y: float,
    ) -> QVector3D:
        return ingetrazo_render.lock_line(self, start, lock_dir, pixel_x, pixel_y)

    def _depth_of(self, point) -> Optional[float]:
        return ingetrazo_render.view_depth(self, point)

    def _recentre_on(self, x: float, y: float) -> None:
        return ingetrazo_render.recentre(self, x, y)

    def _point_at_centre_depth(self, x: float, y: float):
        return ingetrazo_render.centre_depth(self, x, y)

    @staticmethod
    def _can_warp_pointer() -> bool:
        """Whether the pointer can be put back after each look move. Wayland
        does not let a client move it; there the look is a plain drag."""
        from core.input_types import NATIVE
        if NATIVE:
            return False  # winit delivers relative look drags in window coordinates
        from PySide6.QtGui import QGuiApplication
        name = QGuiApplication.platformName().lower()
        return not (name.startswith("wayland") or name in ("offscreen",
                                                            "minimal"))

    def _begin_look(self, ev) -> None:
        self._look_drag = (ev.button(), ev.position().toPoint())
        self.setCursor(Qt.BlankCursor)

    def _look_move(self, ev) -> None:
        return ingetrazo_render.look_move(self, ev)

    def _world_under_cursor(self, x: float, y: float) -> Optional[QVector3D]:
        return ingetrazo_render.cursor_world(self, x, y)

    def _zoom_at(self, steps: float, pos):
        return ingetrazo_render.cursor_zoom(self, steps, pos)

    def _zoom_out_step(self) -> float:
        return ingetrazo_render.retreat_step(self)
