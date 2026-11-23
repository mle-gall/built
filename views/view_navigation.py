# SPDX-License-Identifier: GPL-3.0-or-later
"""Navigation shared by document tool input and Metal presentation."""
from __future__ import annotations

import math
from typing import Optional

from PySide6.QtCore import QPointF, Qt
from ingetrazo_core import Vec3 as QVector3D


class ViewNavigation:
    _ZOOM_OUT_STEP = 0.05

    def _pixel_to_ray(self, x, y):
        camera = self.camera
        projection = getattr(camera, "picking_projection_matrix", camera.projection_matrix)
        matrix = projection() * camera.view_matrix()
        return matrix.pixel_ray(x, y, self.width(), self.height())

    def _world_to_pixel(self, world, mvp=None):
        if mvp is None:
            mvp = self.camera.projection_matrix() * self.camera.view_matrix()
        return mvp.world_pixel(world, self.width(), self.height())

    def _zoom_to_box(self, start: QPointF, end: QPointF) -> None:
        """Zoom the camera so the dragged screen rectangle fills the view
        (Zoom Window). Re-centres on the box's centre and scales the distance
        by the fraction of the viewport the box covers."""
        bw, bh = abs(end.x() - start.x()), abs(end.y() - start.y())
        if bw < 6 or bh < 6:          # a click, not a real box → ignore
            return
        cx, cy = (start.x() + end.x()) / 2.0, (start.y() + end.y()) / 2.0
        focus = self._world_under_cursor(cx, cy)
        if focus is not None:
            self.camera.target = focus
        frac = max(bw / max(self.width(), 1), bh / max(self.height(), 1))
        frac = max(frac, 0.04)        # don't zoom past a sane limit in one go
        from core.camera import MAX_DISTANCE
        self.camera.distance = max(
            self.camera.min_distance(),
            min(self.camera.distance * frac, MAX_DISTANCE))
        self.update()

    def _project_to_lock_line(
        self,
        start: QVector3D,
        lock_dir: QVector3D,
        pixel_x: float,
        pixel_y: float,
    ) -> QVector3D:
        """Closest point on the lock line (``start``, ``lock_dir``) to the
        camera ray that passes through ``(pixel_x, pixel_y)``.

        This is what makes Z-axis locks actually let you draw vertical
        lines — moving the mouse up/down on screen slides the projected
        point along the Z line.

        The cursor is first dropped square onto the line as the screen
        shows it, so only motion ALONG the line changes the length. In
        perspective the closest point to the cursor's own ray also moved
        with the cursor's sideways position (#769).
        """
        step = max(float(getattr(self.camera, "distance", 1.0)) * 0.25, 1e-3)
        pa = self._world_to_pixel(start)
        pb = self._world_to_pixel(start + lock_dir.normalized() * step)
        if pa is not None and pb is not None:
            dx, dy = pb[0] - pa[0], pb[1] - pa[1]
            dd = dx * dx + dy * dy
            if dd > 1.0:
                s = ((pixel_x - pa[0]) * dx + (pixel_y - pa[1]) * dy) / dd
                pixel_x, pixel_y = pa[0] + dx * s, pa[1] + dy * s
        ray_origin, ray_dir = self._pixel_to_ray(pixel_x, pixel_y)
        if ray_origin is None or ray_dir is None:
            return start
        d1 = lock_dir.normalized()
        d2 = ray_dir
        r = start - ray_origin
        b = QVector3D.dotProduct(d1, d2)
        d = QVector3D.dotProduct(d1, r)
        e = QVector3D.dotProduct(d2, r)
        denom = 1.0 - b * b
        if abs(denom) < 1e-6:
            # Lock line is parallel to the camera ray — project the ray
            # origin onto the lock line as a stable fallback.
            t = -d
        else:
            t = (b * e - d) / denom
        return start + d1 * t

    def _depth_of(self, point) -> Optional[float]:
        """How far ``point`` lies in front of the eye, along the view
        direction: what one pixel of pan is worth there (#184)."""
        if point is None:
            return None
        cam = self.camera
        return QVector3D.dotProduct(point - cam.eye(), cam.forward())

    def _recentre_on(self, x: float, y: float) -> None:
        """Bring the model point under pixel ``(x, y)`` to the centre of the
        view: a slide, the view keeps its direction and what is at that depth
        keeps its size on screen. With no model there, the point at the depth
        of the view's centre: the same slide, and the zoom stays (#1154)."""
        cam = self.camera
        point = self._model_point_at(x, y)
        if point is None:
            point = self._point_at_centre_depth(x, y)
            if point is None:
                return
            cam.target = point
            self.update()
            return
        if cam.perspective:
            # The depth along the view, not the slant length to the point:
            # the eye slides sideways and comes no nearer. A parallel view
            # keeps its distance, which is its zoom.
            depth = self._depth_of(point)
            if depth is None or not math.isfinite(depth) or depth <= 0.0:
                return
            cam.distance = max(cam.min_distance(point), depth)
        cam.target = QVector3D(point)
        self.update()

    def _point_at_centre_depth(self, x: float, y: float):
        """Where the ray of pixel ``(x, y)`` meets the plane through the
        camera target, square to the view."""
        origin, direction = self._pixel_to_ray(x, y)
        if origin is None or direction is None:
            return None
        cam = self.camera
        view = (cam.target - cam.eye()).normalized()
        along = QVector3D.dotProduct(direction, view)
        if abs(along) < 1e-6:
            return None
        t = QVector3D.dotProduct(cam.target - origin, view) / along
        return origin + direction * t if t > 0.0 else None

    @staticmethod
    def _can_warp_pointer() -> bool:
        """Whether the pointer can be put back after each look move. Wayland
        does not let a client move it; there the look is a plain drag."""
        from PySide6.QtGui import QGuiApplication
        name = QGuiApplication.platformName().lower()
        return not (name.startswith("wayland") or name in ("offscreen",
                                                            "minimal"))

    def _begin_look(self, ev) -> None:
        self._look_drag = (ev.button(), ev.position().toPoint())
        self.setCursor(Qt.BlankCursor)

    def _look_move(self, ev) -> None:
        button, last = self._look_drag
        p = ev.position().toPoint()
        dx, dy = p.x() - last.x(), p.y() - last.y()
        if dx or dy:
            self.active_tool.on_look(self, dx, dy)
        if (dx or dy) and self._can_warp_pointer():
            from PySide6.QtGui import QCursor
            QCursor.setPos(self.mapToGlobal(last))
            p = last
        self._look_drag = (button, p)

    def _world_under_cursor(self, x: float, y: float) -> Optional[QVector3D]:
        """Nearest geometry hit, otherwise the focal plane through the target.

        An empty-space ray near the horizon meets Z=0 arbitrarily far away.
        Zooming toward that point magnified a tiny trackpad gesture into a
        huge camera translation. The focal plane keeps background zoom at
        the current viewing distance, on either side of the horizon.
        """
        origin, direction = self._pixel_to_ray(x, y)
        if origin is None or direction is None:
            return None
        best_t = None
        seen = self._depth_world_at(x, y) if hasattr(self, "_depth_world_at") \
            else None
        if seen is not None and seen[0] == "hit":
            return seen[1]
        # No frame to read (wheel notches faster than paints): on a huge
        # model the ray against the index cost 2 s a notch (live profile,
        # #158) — the focal plane below answers at once.
        huge = (seen is None and hasattr(self, "_pick_lazy_table")
                and self._pick_lazy_table() is not None)
        idx = (self._pick_index(near=("px", x, y))
               if seen is None and not huge else None)
        if idx is not None and idx.entities:
            import numpy as np
            t = self._ray_hits(idx, origin, direction, idx.ent_vis,
                               reduce_global=True)
            if t is not None and np.isfinite(t):
                best_t = float(t)
        if best_t is not None:
            return origin + direction * best_t
        view = (self.camera.target - self.camera.eye())
        if view.length() > 1e-9:
            view = view.normalized()
            denom = QVector3D.dotProduct(direction, view)
            if abs(denom) > 1e-6:
                t = QVector3D.dotProduct(self.camera.target - origin, view) / denom
                if t > 0:
                    return origin + direction * t
        return None

    def _zoom_at(self, steps: float, pos):
        """Zoom ``steps`` notches toward the point under ``pos`` -- the
        wheel's zoom, shared with the trackpad pinch (#248). Returns
        ``(reprojected, reused)`` for the perf log."""
        # Zoom toward the cursor: keep the point under the
        # pointer fixed on screen, not the origin. During a wheel burst the
        # focus is CACHED: zoom_to pins that world point, so re-picking every
        # tick both wasted ~25 ms/tick against a big model (the zoom felt
        # heavy) and let float error drift the pinned point.
        def pose():
            c = self.camera
            t = c.target
            return (round(c.yaw, 9), round(c.pitch, 9),
                    round(c.distance, 6),
                    round(t.x(), 6), round(t.y(), 6), round(t.z(), 6))

        # The pin is anchored to the CAMERA POSE, not a time window:
        # zoom_to keeps the focus fixed on screen by construction, so slow
        # one-notch-per-second zooming reuses it indefinitely (a 1 s window
        # expired between deliberate notches and re-picked 280k triangles
        # per notch — the zoom jank). Orbit/pan or an edit changes the pose
        # or the version and honestly invalidates it.
        cached = getattr(self, "_zoom_focus", None)
        reused = (cached is not None and cached[0] == pose()
                  and abs(pos.x() - cached[1]) < 24.0
                  and abs(pos.y() - cached[2]) < 24.0
                  and self.scene.version == cached[3])
        reproj = False
        if reused:
            focus = cached[4]
        else:
            focus = None
            if (cached is not None and cached[4] is not None
                    and self.scene.version == cached[3]):
                # Orbit/pan changed the pose, but the cursor usually still
                # rests on the SAME world point (zoom → orbit a touch → zoom
                # again, the modeling rhythm). Re-validate the pin by
                # projecting it with the current camera: one matrix map
                # instead of a ~25 ms re-pick of the whole model per notch.
                p = self._world_to_pixel(cached[4])
                if (p is not None and abs(p[0] - pos.x()) < 24.0
                        and abs(p[1] - pos.y()) < 24.0):
                    focus = cached[4]
                    reproj = True
            if focus is None:
                focus = self._world_under_cursor(pos.x(), pos.y())
        if focus is not None:
            # A perceptible retreat even from a close-up: ~1 % of the
            # model's size per notch when the frame scaling alone would move
            # the eye less than that (the "zoom is stuck" report). Scale the
            # floor by the actual wheel fraction: a trackpad can send dozens
            # of tiny events per gesture, each previously getting a full notch.
            self.camera.zoom_to(steps, focus,
                                min_step=self._zoom_out_step() * abs(steps))
        else:
            self.camera.zoom(steps)
        self._zoom_focus = (pose(), pos.x(), pos.y(),
                            self.scene.version, focus)
        return reproj, reused

    def _zoom_out_step(self) -> float:
        """Zoom-out floor per notch: 1 % of the model, never less than 5 cm
        — as it always was — except closer than 2 cm to a small part, where
        5 cm would throw it out of sight in one notch: there the 5 cm
        shrinks with the distance (1.25 cm at the 5 mm close-up)."""
        diag = self._model_diag()
        if diag <= 0.0:
            return self._ZOOM_OUT_STEP
        from core.camera import MIN_DISTANCE_FAR
        near = min(1.0, float(self.camera.distance) / MIN_DISTANCE_FAR)
        return max(self._ZOOM_OUT_STEP * near, 0.01 * diag)
