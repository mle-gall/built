# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Orbital camera for the 3D viewport.

Z-up convention (Blender, FreeCAD): X red (east), Y green (north),
Z blue (up). The camera orbits around a ``target`` point in spherical
coordinates (``yaw``, ``pitch``, ``distance``). Both perspective and parallel
("orthographic") projections are supported. Target, up and the eye derived
from the orbit parameters use binary64 model coordinates.
"""
from __future__ import annotations

import math

import ingetrazo_core as native
from ingetrazo_core import Matrix4 as QMatrix4x4, Vec3 as QVector3D
from core.camera_point import CameraPoint


#: How close the camera may get to what it is looking at. A component is
#: centimetres, not metres: half a metre away is as close as you can stand
#: to a wall, and it left the face of a figure or the rim of a wheel
#: impossible to look at. A part for a 3D printer is millimetres: at 2 cm a
#: 10 mm cube never filled the screen (#653, #692), so near the origin the
#: camera comes to 5 mm. Not closer: the engine welds vertices to a 0.1 mm
#: grid, and nearer than this that grid is what you would see.
MIN_DISTANCE = 0.005

#: Farther from the origin the floor rises (:meth:`OrbitCamera.min_distance`)
#: back to the 2 cm it always was: float32 positions lose about 0.06 mm at
#: 1 km, and a closer look there would show the model trembling. Five
#: millimetres hold up to ~40 m from the origin; past ~160 m it is 2 cm.
MIN_DISTANCE_FAR = 0.02
_MIN_PER_METRE = MIN_DISTANCE / 40.0

#: The near clipping plane never comes closer than this — what it was at
#: the old 2 cm floor (2 % of it), so the depth buffer is never coarser
#: than it was: things far behind do not start to flicker while you look
#: at a small part up close.
NEAR_FLOOR = 4e-4

#: A scene with nothing in it has no size to frame: it is framed as if it
#: were this big, as it always was.
_EMPTY_SIZE = 0.02

#: Ceiling: past this the model is a dot and the depth buffer is noise.
MAX_DISTANCE = 10000.0

#: How far behind the orbit target the view still reaches, in orbit
#: distances. A fixed 10 km far plane cut a large model in half when you
#: zoomed out to see it — the far side vanished and came back on zooming
#: in (#340). Framing a model puts the target on it at about its size, so
#: four distances is the whole model behind the target with room to spare.
FAR_PER_DISTANCE = 4.0

#: A parallel view's depth range, in multiples of the farthest reach of
#: what it holds from the eye (``OrbitCamera.parallel_depth``): the axes
#: and guides run past the model, and a dragged object may too.
PARALLEL_REACH_MARGIN = 4.0

#: The film or sensor a focal length is read for, (width, height) in
#: millimetres. A lens of so many millimetres gives the picture it gives on
#: THAT camera: the smaller the sensor, the narrower the same lens sees.
#: The full frame of 35 mm film is the default, and what a focal length
#: means when nothing else is said.
FULL_FRAME = (36.0, 24.0)
SENSORS = {
    "full_frame": FULL_FRAME,
    "aps_c": (23.6, 15.6),
    "micro_four_thirds": (17.3, 13.0),
    "super_35": (24.9, 18.7),
}
SENSOR_HEIGHT_MM = FULL_FRAME[1]

#: Bounds for a sensor typed by hand, in millimetres.
SENSOR_RANGE_MM = (1.0, 100.0)

#: The angles of view the camera may take: down to 1° for a long lens (533
#: mm on full frame is 2.6°; 1° is about 1 375 mm) and up to 120° wide.
FOV_RANGE = (1.0, 120.0)

#: The lens settings beyond the field of view (Camera ▸ Camera Settings),
#: each with the value that leaves the view as it always was.
LENS_DEFAULTS = {
    "shift_x": 0.0,         # frame slid sideways, in view widths
    "shift_y": 0.0,         # frame slid up, in view heights
    "roll_deg": 0.0,        # turn about the sight line, degrees
    "clip_near": None,      # metres from the eye, or None = automatic
    "clip_far": None,       # metres from the eye, or None = automatic
    "frame_aspect": None,   # width / height of the frame drawn, or None
    "sensor": None,         # (width, height) mm, or None = full frame
}


def sensor_picture_height(sensor, aspect: float | None) -> float:
    """How much of the sensor's height a picture of shape ``aspect`` (width
    / height) uses, in millimetres — as in a real camera, the picture is
    the largest rectangle of its shape inside the sensor: a 16:9 picture on
    a 3:2 sensor takes its whole width and less than its height."""
    w, h = sensor or FULL_FRAME
    if aspect and aspect > 0.0 and aspect > w / h:
        return w / aspect
    return h

#: How far the frame may slide: a whole view each way is already a picture
#: taken entirely beside where the camera points.
MAX_SHIFT = 1.0


def focal_from_fov(fov_deg: float,
                   picture_height_mm: float = SENSOR_HEIGHT_MM) -> float:
    """The focal length, in millimetres, of a lens whose vertical angle of
    view is ``fov_deg`` on a picture ``picture_height_mm`` tall (by default
    the 24 mm of the 35 mm full frame)."""
    half = math.radians(max(1e-3, min(179.0, float(fov_deg)))) / 2.0
    return (float(picture_height_mm) / 2.0) / math.tan(half)


def fov_from_focal(focal_mm: float,
                   picture_height_mm: float = SENSOR_HEIGHT_MM) -> float:
    """The vertical angle of view, in degrees, of a ``focal_mm`` lens on a
    picture ``picture_height_mm`` tall — the inverse of
    :func:`focal_from_fov`."""
    f = max(1e-3, float(focal_mm))
    return math.degrees(2.0 * math.atan((float(picture_height_mm) / 2.0) / f))


def frame_rect(width: float, height: float,
               aspect: float | None) -> tuple[float, float, float, float] | None:
    """The picture frame of shape ``aspect`` (width / height) in a view of
    ``width`` × ``height`` pixels: the largest such rectangle, centred, as
    ``(x, y, w, h)``. ``None`` without a frame or with an empty view."""
    if not aspect or aspect <= 0.0 or width <= 0 or height <= 0:
        return None
    if width / height > aspect:
        h = float(height)
        w = h * aspect
    else:
        w = float(width)
        h = w / aspect
    return ((width - w) / 2.0, (height - h) / 2.0, w, h)


class OrbitCamera:
    """Camera that orbits around a target point."""

    def __setattr__(self, name, value):
        if name in ("target", "up") and not isinstance(value, CameraPoint):
            value = CameraPoint(value)
        super().__setattr__(name, value)

    def __init__(self) -> None:
        self.target = QVector3D(0.0, 0.0, 0.0)
        self.distance = 20.0
        self.yaw = math.radians(-45.0)
        self.pitch = math.radians(30.0)
        self.up = QVector3D(0.0, 0.0, 1.0)
        self.fov_deg = 45.0
        self.aspect = 1.0
        self.znear = 0.1
        self.zfar = 10000.0
        self.perspective = True
        #: Two-Point Perspective: vertical lines stay vertical
        #: (José Castro Basso, FADU–UDELAR, for teaching architectural
        #: drawing). Only in perspective; see ``view_matrix``.
        self.two_point = False
        #: What the view holds, as a bounding sphere ``(centre, radius)``
        #: the viewport keeps up to date, or ``None`` (unknown). Parallel
        #: projection fits its depth range to it (``parallel_depth``).
        self.depth_reach = None
        #: The perspective near/far fitted to what the view shows
        #: (``fit_depth_range``), with the view it was fitted for, or
        #: ``None``: the automatic planes.
        self._depth_range = None
        #: The lens (:data:`LENS_DEFAULTS`). The frame SHIFT slides the
        #: picture without turning the camera, as a view camera's rise
        #: does: a tall facade fills the frame and its verticals stay
        #: parallel. ROLL turns the picture about the sight line. The
        #: CLIP distances, when set, replace the automatic near and far
        #: planes — a near plane past a wall looks through it. The FRAME
        #: is the shape of the picture, drawn over the view.
        self.shift_x = 0.0
        self.shift_y = 0.0
        self.roll_deg = 0.0
        self.clip_near = None
        self.clip_far = None
        self.frame_aspect = None
        #: The SENSOR a focal length is read for, ``(width, height)`` mm,
        #: or ``None`` for the 35 mm full frame (:data:`SENSORS`).
        self.sensor = None

    # ---- Lens -------------------------------------------------------------
    def lens_dict(self) -> dict:
        """The lens settings that differ from :data:`LENS_DEFAULTS` — what
        a scene and the document keep (``{}`` for an ordinary camera)."""
        out = {}
        for k, default in LENS_DEFAULTS.items():
            v = getattr(self, k)
            if k == "sensor":
                if v is not None and tuple(v) != FULL_FRAME:
                    out[k] = [float(v[0]), float(v[1])]
            elif v != default:
                out[k] = v
        return out

    @staticmethod
    def _read_sensor(v):
        """A sensor as given in a lens dict — ``[width, height]`` mm — or
        ``None`` when it is not one (the full frame then)."""
        if not isinstance(v, (list, tuple)) or len(v) != 2:
            return None
        try:
            w, h = float(v[0]), float(v[1])
        except (TypeError, ValueError):
            return None
        lo, hi = SENSOR_RANGE_MM
        if not (math.isfinite(w) and math.isfinite(h)
                and lo <= w <= hi and lo <= h <= hi):
            return None
        return None if (w, h) == FULL_FRAME else (w, h)

    def apply_lens(self, raw) -> None:
        """Put back what :meth:`lens_dict` gave; every setting it does not
        name goes back to its default, so a scene saved with an ordinary
        lens takes off a shift or a roll left by the one before. Values
        that are not numbers are ignored (the default stays)."""
        raw = raw if isinstance(raw, dict) else {}
        for k, default in LENS_DEFAULTS.items():
            setattr(self, k, default)
            v = raw.get(k)
            if v is None:
                continue
            if k == "sensor":
                self.sensor = self._read_sensor(v)
                continue
            try:
                v = float(v)
            except (TypeError, ValueError):
                continue
            if not math.isfinite(v):
                continue
            if k in ("shift_x", "shift_y"):
                v = max(-MAX_SHIFT, min(MAX_SHIFT, v))
            elif k == "roll_deg":
                v = math.remainder(v, 360.0)
            elif v <= 0.0:          # a clip distance or aspect must be > 0
                continue
            setattr(self, k, v)
        if (self.clip_near is not None and self.clip_far is not None
                and self.clip_far <= self.clip_near):
            self.clip_far = None

    def reset_lens(self) -> None:
        """Shift, roll, clipping and frame back to :data:`LENS_DEFAULTS`.
        The sensor stays: it is the camera, not a setting of the shot."""
        sensor = self.sensor
        self.apply_lens({})
        self.sensor = sensor

    def set_sensor(self, size) -> None:
        """Read focal lengths for the sensor ``size`` ``(width, height)`` mm
        (``None``: the full frame). The picture stays as it is — the angle
        of view is kept; only what it reads in millimetres changes."""
        self.sensor = None if size is None else self._read_sensor(size)

    def sensor_size(self) -> tuple[float, float]:
        """The sensor, ``(width, height)`` mm."""
        return tuple(self.sensor) if self.sensor else FULL_FRAME

    def picture_aspect(self) -> float:
        """The shape of the picture: the frame's when one is set, else the
        view's."""
        return self.frame_aspect or self.aspect

    def picture_height_mm(self) -> float:
        """The height of the sensor the picture uses (as in a real camera,
        the largest rectangle of the picture's shape inside the sensor)."""
        return sensor_picture_height(self.sensor_size(),
                                     self.picture_aspect())

    @property
    def focal_mm(self) -> float:
        """The field of view as a focal length on this camera's sensor."""
        return focal_from_fov(self.fov_deg, self.picture_height_mm())

    def fov_for_focal(self, focal_mm: float) -> float:
        """The vertical angle of view of a ``focal_mm`` lens on this
        camera's sensor and picture."""
        return fov_from_focal(focal_mm, self.picture_height_mm())

    def focal_for_fov(self, fov_deg: float) -> float:
        """The focal length that gives ``fov_deg`` on this camera."""
        return focal_from_fov(fov_deg, self.picture_height_mm())

    def place(self, eye: QVector3D, target: QVector3D) -> bool:
        """Put the eye exactly at ``eye`` looking at ``target`` — the
        coordinates typed in the camera settings. The target becomes the
        orbit centre, so orbiting afterwards turns about it. ``False`` (and
        nothing changes) when the two points coincide or are not numbers."""
        d = target - eye
        length = d.length()
        if not math.isfinite(length) or length < 1e-6:
            return False
        before = (CameraPoint(self.target), self.distance, self.yaw, self.pitch)
        self.distance = min(length, MAX_DISTANCE)
        self.look_from(eye, d)
        if not self.is_finite():
            self.target, self.distance, self.yaw, self.pitch = before
            return False
        return True

    # ---- Derived state ------------------------------------------------------
    # The pose and the projection are computed in Rust
    # (``rust/ingetrazo_core/src/camera_live.rs`` over the sheets' ``Cam``):
    # one implementation for the viewport and the sheet frames.

    def eye(self) -> QVector3D:
        """``target + distance·(cos p·cos y, cos p·sin y, sin p)``."""
        return native.camera_eye(self)

    def forward(self) -> QVector3D:
        """Unit view direction, eye → target."""
        return native.camera_forward(self)

    def up_vector(self) -> QVector3D:
        """The up vector every projection must use: ``up`` itself, unless
        the sight line runs along it — the exact Top and Bottom views —
        where ``lookAt`` has no right. There it is the limit of the view a
        hair short of vertical: north up the screen in Top (y+ at the
        default yaw), the same axis the other way in Bottom.

        A ``roll_deg`` turns that up about the sight line (positive turns
        the camera to the left, the picture to the right). Panning, Zoom
        Extents and the picture itself all read the up from here, so they
        agree. A two-point perspective keeps its verticals upright and
        takes no roll."""
        return native.camera_up_vector(self)

    # ---- First person (Position Camera / Look Around / Walk) ---------------
    # The orbit model stays: a walkthrough only ever asks "the eye is HERE,
    # looking THERE", and that is a target at ``distance`` along the look
    # direction. Nothing else in the viewport has to learn a second camera.

    def look_from(self, eye: QVector3D, direction: QVector3D) -> None:
        """Put the eye at ``eye`` looking along ``direction`` (any length).
        A near-vertical look keeps just off the pole, like ``orbit``."""
        d = QVector3D(direction)
        if d.lengthSquared() < 1e-18:
            d = self.forward()
        d = d.normalized()
        # eye = target + distance·(cp·cy, cp·sy, sp): the eye sits BEHIND
        # the target along -forward, so the spherical angles come from -d.
        pitch = math.asin(max(-1.0, min(1.0, -d.z())))
        pitch = max(min(pitch, math.radians(89.0)), math.radians(-89.0))
        if abs(math.cos(pitch)) > 1e-9:
            yaw = math.atan2(-d.y(), -d.x())
        else:
            yaw = self.yaw
        self.yaw, self.pitch = yaw, pitch
        self.target = CameraPoint(eye) + d * self.distance

    def turn(self, d_yaw_deg: float, d_pitch_deg: float) -> None:
        """Turn the head: the eye stays put, the look direction swings by
        ``d_yaw_deg`` to the right (clockwise seen from above) and
        ``d_pitch_deg`` upward."""
        eye = self.eye()
        d = self.forward()
        yaw = math.atan2(d.y(), d.x()) - math.radians(d_yaw_deg)
        pitch = math.asin(max(-1.0, min(1.0, d.z()))) + math.radians(d_pitch_deg)
        pitch = max(min(pitch, math.radians(89.0)), math.radians(-89.0))
        cp = math.cos(pitch)
        self.look_from(eye, QVector3D(cp * math.cos(yaw), cp * math.sin(yaw),
                                      math.sin(pitch)))

    def move_eye(self, delta: QVector3D) -> None:
        """Walk: eye and target move together, the look stays the same."""
        self.target = self.target + delta

    def view_matrix(self) -> QMatrix4x4:
        """``lookAt`` from the eye; a two-point perspective (Camera ▸
        Two-Point Perspective, below about 78° of pitch) looks along the
        level sight line so verticals stay vertical."""
        return native.camera_view_matrix(self)

    def projection_matrix(self) -> QMatrix4x4:
        """Perspective: the near plane follows the camera in at close range
        (never closer than :data:`NEAR_FLOOR`), a two-point view slides the
        frustum by the target's height, the frame shift slides it sideways
        or up. Parallel: sized from the distance so the framing matches the
        perspective, deep as :meth:`parallel_depth`. A depth range fitted to
        the view (:meth:`fit_depth_range`) replaces the automatic planes."""
        return native.camera_projection_matrix(self, self._fitted_depth_range())

    def parallel_depth(self) -> float:
        """How far in front of and behind the eye a parallel view reaches:
        a few times the reach of what the view holds (``depth_reach``), so
        the depth buffer is not split over ±10 km (#874); ``far_plane()``
        when the reach is unknown."""
        return native.camera_depths(self, self._fitted_depth_range())[2]

    def near_plane(self) -> float:
        """The near clipping distance in perspective: ``clip_near`` set by
        hand, else the fitted range, else ``znear`` stepping back with the
        camera at close range — never closer than :data:`NEAR_FLOOR`."""
        return native.camera_depths(self, self._fitted_depth_range())[0]

    def min_distance(self, at: QVector3D | None = None) -> float:
        """How close the camera may come to ``at`` (the target when not
        given): :data:`MIN_DISTANCE` near the origin, rising with the
        distance from it up to :data:`MIN_DISTANCE_FAR`."""
        return native.camera_min_distance(self.target if at is None else at)

    def far_plane(self) -> float:
        """The far clipping distance: ``clip_far`` set by hand (never
        nearer than just past the near plane), else the fitted range, else
        ``zfar`` growing with the orbit distance when zoomed far out
        (#340)."""
        return native.camera_depths(self, self._fitted_depth_range())[1]

    # ---- Depth range fitted to the view ----------------------------------
    def picking_projection_matrix(self) -> QMatrix4x4:
        """The projection with the automatic near/far, for editing rays.

        A ray's ideal direction does not depend on the clipping planes, but
        the float32 rounding of the inverted matrix does: fitting the depth
        range would move every pick and snap ray by a rounding step. Rays
        keep the unfitted matrix so they stay byte-for-byte what they were;
        depth read-back must still use :meth:`projection_matrix`."""
        fitted = self._depth_range
        self._depth_range = None
        try:
            return self.projection_matrix()
        finally:
            self._depth_range = fitted

    def _depth_view_key(self):
        return (tuple(self.view_matrix().data()), self.fov_deg, self.aspect,
                self.perspective, self.two_point, self.znear, self.zfar,
                self.distance, self.shift_x, self.shift_y,
                self.clip_near, self.clip_far)

    def _fitted_depth_range(self):
        fitted = self._depth_range
        if fitted is not None and fitted[0] == self._depth_view_key():
            return fitted[1:]
        return None

    def fit_depth_range(self, lo, hi) -> None:
        """Fit the perspective near/far to world-space boxes ``lo``/``hi``
        (``N×3``) of what the view draws, for depth precision: a near plane
        0.1 m from the eye leaves plates a few millimetres thick fighting at
        50 m on a fixed-point or standard float depth buffer.

        Only boxes inside the four side planes count (not the old near/far:
        that would exclude geometry the fit should recover). The fit holds
        for this exact view only and lapses as soon as the camera moves.
        Nothing is fitted in a parallel view, with a near or far clip set by
        hand (Camera Settings ▸ Clipping), with unknown or non-finite
        bounds, or when a box crosses the eye: moving near inward would make
        precision worse, never better, so the automatic near stays."""
        import numpy as np
        self._depth_range = None
        if (not self.perspective or self.clip_near is not None
                or self.clip_far is not None or len(lo) == 0):
            return
        lo, hi = np.asarray(lo, float), np.asarray(hi, float)
        if not (np.isfinite(lo).all() and np.isfinite(hi).all()):
            return
        view = np.asarray(self.view_matrix().data()).reshape(4, 4, order="F")
        m = np.asarray((self.projection_matrix() * self.view_matrix()).data()
                       ).reshape(4, 4, order="F")
        keep = np.ones(len(lo), bool)
        for pl in (m[3] + m[0], m[3] - m[0], m[3] + m[1], m[3] - m[1]):
            px = hi[:, 0] if pl[0] >= 0 else lo[:, 0]
            py = hi[:, 1] if pl[1] >= 0 else lo[:, 1]
            pz = hi[:, 2] if pl[2] >= 0 else lo[:, 2]
            keep &= ((px * pl[0] + py * pl[1]) + pz * pl[2]) + pl[3] >= 0
        direction, offset = -view[2, :3], -view[2, 3]
        nx, ny, nz = direction
        lx, hx = (lo[:, 0], hi[:, 0]) if nx >= 0 else (hi[:, 0], lo[:, 0])
        ly, hy = (lo[:, 1], hi[:, 1]) if ny >= 0 else (hi[:, 1], lo[:, 1])
        lz, hz = (lo[:, 2], hi[:, 2]) if nz >= 0 else (hi[:, 2], lo[:, 2])
        near = ((lx * nx + ly * ny) + lz * nz) + offset
        far = ((hx * nx + hy * ny) + hz * nz) + offset
        keep &= far > NEAR_FLOOR
        if not keep.any():
            return
        n = max(NEAR_FLOOR, float(near[keep].min()) * 0.95)
        if n <= self.near_plane():
            return
        f = max(n + NEAR_FLOOR, float(far[keep].max()) * 1.05)
        self._depth_range = (self._depth_view_key(), n, f)

    # ---- Navigation ---------------------------------------------------------
    def orbit(self, dx_pixels: float, dy_pixels: float, viewport_h: int) -> None:
        """Turn the model under the cursor, Blender style.

        Both axes GRAB THE MODEL: drag right and the model swings right,
        drag down and it tips down — you come up over it and see its top.
        Same gesture as :meth:`pan`, and that is the check that matters,
        because until 2026-09-09 the vertical axis disagreed with it:
        ``pitch`` went the other way, so dragging down moved the CAMERA
        down and the model appeared to slide up while the horizontal axis
        kept grabbing. Two users reported it the same day (GitHub #7 and
        an e-mail), neither able to say which of the two axes was wrong —
        which is exactly what a single inverted axis feels like from the
        outside.

        Callers that let the user ask for the old feel negate ``dy_pixels``
        (Preferences ▸ Invert vertical orbit); the convention itself lives
        here.
        """
        self.yaw, self.pitch = native.orbit_angles(
            self.yaw, self.pitch, dx_pixels, dy_pixels, viewport_h)

    def orbit_about(self, pivot: QVector3D, dx_pixels: float,
                    dy_pixels: float, viewport_h: int) -> None:
        """:meth:`orbit` around ``pivot`` instead of the target (#164).

        Same drag convention and the same yaw/pitch change, but the whole
        camera (eye AND target) turns rigidly about ``pivot``: the model
        point the gesture started on stays where it was on screen, instead
        of the view swinging around the target -- which, on a model far
        from the origin, was a point nowhere near what you were looking at.
        The yaw turn is about the world vertical through the pivot, the
        pitch turn about the view's horizontal axis, so the horizon stays
        level (no roll) and the distance to the target never changes.
        """
        old_yaw, old_pitch, old_target = self.yaw, self.pitch, self.target
        self.orbit(dx_pixels, dy_pixels, viewport_h)
        self.target = native.orbit_target(
            old_target, pivot, old_yaw, old_pitch, self.yaw, self.pitch)
        if not self.is_finite():
            self.yaw, self.pitch, self.target = old_yaw, old_pitch, old_target

    def pan(self, dx_pixels: float, dy_pixels: float, viewport_h: int,
            depth: float | None = None) -> None:
        """Slide the view by a drag of ``(dx, dy)`` pixels.

        ``depth`` is how far in front of the eye the point grabbed under
        the cursor lies: in perspective, a pixel spans more the deeper it
        is, so moving by that depth keeps the grabbed point under the
        cursor, as users expect. Without it the pan used the distance to
        the orbit target, which zooming in shrinks to 2 cm: at full zoom a
        wall metres away barely moved (Alejandro Limón, #184). Parallel
        views scale the same at every depth and ignore it."""
        span = self.distance
        if (self.perspective and depth is not None and math.isfinite(depth)
                and depth > 1e-6):
            span = depth
        self.target = native.pan_target(
            self.target, self.up_vector(), self.yaw, self.pitch, span, self.fov_deg,
            dx_pixels, dy_pixels, viewport_h)

    def zoom(self, steps: float) -> None:
        self.distance = native.zoom_distance(
            self.distance, steps, self.min_distance(), MAX_DISTANCE)

    def zoom_to(self, steps: float, focus: QVector3D,
                min_step: float = 0.0) -> None:
        """Zoom keeping the world point ``focus`` (under the cursor) fixed on
        screen, as usual. The whole frame scales toward ``focus``, so both
        the distance and the target move by the same factor.

        Two escapes from the "stuck" close-up (the orbit distance pinned at
        :meth:`min_distance`): zooming IN keeps sliding the target toward the
        focus even when the distance cannot shrink any more, and zooming OUT
        retreats the eye by at least ``min_step`` (the viewport passes ~1 %
        of the model's size) — at 2 cm a plain 10 % step was 2 mm per
        notch, dozens of notches to see anything, so users reached for Zoom
        Extents instead.

        Zooming OUT has a ceiling, though: once the eye is MAX_DISTANCE
        from the focus, or the distance is pinned there, the frame stops
        growing. Until 0.5.7 the target kept scaling away from the focus
        ×1.11 per notch with the distance already pinned — 125 km after a
        hundred notches, where float32 can no longer draw the model, then
        infinity and NaN, and a camera that refused to save (#383)."""
        eye_before = self.eye()
        if steps < 0 and (eye_before - focus).length() >= MAX_DISTANCE:
            self.zoom(steps)
            return
        before = self.target, self.distance
        self.target, self.distance = native.zoom_focus(
            self.target, focus, eye_before, self.yaw, self.pitch, self.distance,
            steps, self.min_distance(focus), MAX_DISTANCE, min_step)
        if not self.is_finite():
            self.target, self.distance = before

    def is_finite(self) -> bool:
        """Every number of the pose is a real number (no NaN, no inf)."""
        return all(math.isfinite(v) for v in (
            *self.target.toTuple(), *self.up.toTuple(), self.distance, self.yaw, self.pitch))

    def sees_from_below(self, z: float) -> bool:
        """The view looks at the horizontal plane at height ``z`` from
        underneath: in perspective the eye is at or below it; in parallel
        projection, whose eye is only a point at ``distance``, the view
        looks upward. The ground's shadows are not drawn then (#1301)."""
        if self.perspective:
            return self.eye().z() <= z
        return self.forward().z() > 0.0

    def set_aspect(self, w: int, h: int) -> None:
        self.aspect = max(w, 1) / max(h, 1)

    def toggle_projection(self) -> None:
        self.perspective = not self.perspective
        self.two_point = False

    def toggle_two_point(self) -> None:
        """Camera ▸ Two-Point Perspective; turning it on also
        turns a parallel view back into a perspective."""
        self.two_point = not self.two_point
        if self.two_point:
            self.perspective = True

    #: The three projections the View toolbar's quick buttons offer.
    PROJECTIONS = ("parallel", "perspective", "two_point")

    def projection_mode(self) -> str:
        """``"parallel"``, ``"perspective"`` or ``"two_point"`` — what the
        camera is showing, as the quick buttons name it. ``two_point`` set
        on a parallel camera is no two-point view, so it reads parallel."""
        if not self.perspective:
            return "parallel"
        return "two_point" if self.two_point else "perspective"

    def set_projection(self, mode: str) -> None:
        """Put the camera in one of :attr:`PROJECTIONS` (an unknown name
        is ignored). The pose — target, angles, distance — is untouched."""
        if mode not in self.PROJECTIONS:
            return
        self.perspective = mode != "parallel"
        self.two_point = mode == "two_point"

    # ---- Navigation presets ------------------------------------------------
    def fit_to(self, min_pt: QVector3D, max_pt: QVector3D, margin: float = 1.3) -> None:
        """Center the camera on the AABB and back up enough to frame it."""
        center = CameraPoint(
            (min_pt.x() + max_pt.x()) * 0.5,
            (min_pt.y() + max_pt.y()) * 0.5,
            (min_pt.z() + max_pt.z()) * 0.5,
        )
        diag = (max_pt - min_pt).length()
        floor = self.min_distance(center)
        if diag < 1e-9:
            diag = _EMPTY_SIZE       # an empty scene still needs a distance
        diag = max(diag, floor)
        self.target = center
        fov_rad = math.radians(self.fov_deg)
        self.distance = max(diag * margin / (2.0 * math.tan(fov_rad / 2.0)),
                            floor)

    def fit_box(self, min_pt: QVector3D, max_pt: QVector3D,
                margin: float = 1.1, free=None) -> None:
        """Zoom Extents: frame the box as the camera looks at it NOW.

        ``fit_to`` frames the box's bounding SPHERE, which does not depend
        on the view — so after an orbit or a standard view it gave back the
        very same target and distance, and a second Zoom Extents did
        nothing even with the model a fraction of the screen (Marco: «a
        veces no hace efecto, en determinadas posiciones»). Here the eight
        corners are measured along the camera's right and up, so the model
        fills the window in this orientation and for this window shape;
        in perspective the camera also backs off by how far the box comes
        toward it, so the near side is not cut by the frame.

        ``free`` — (x0, y0, x1, y1) in device coordinates, the part of the
        window the panels leave free (the chrome's ``ui_free_ndc``) — frames
        the box into that part instead (``ingetrazo_core.frame_box``)."""
        # Project a nearby box in Qt's local frame. A world-space float
        # matrix cannot distinguish its corners at survey coordinates.
        from core.coordinate_frame import choose_origin
        center = (CameraPoint(min_pt) + max_pt) * .5
        origin = choose_origin(center.toTuple())
        if any(origin):
            from copy import copy
            anchor = CameraPoint(origin)
            local = copy(self)
            local.target = (0., 0., 0.)
            local.fit_box(min_pt - anchor, max_pt - anchor, margin, free)
            self.target = anchor + local.target
            self.distance = local.distance
            return
        f = self.forward()
        r = QVector3D.crossProduct(f, self.up_vector())
        if r.lengthSquared() < 1e-18:
            self.fit_to(min_pt, max_pt)
            return
        r = r.normalized()
        u = QVector3D.crossProduct(r, f)
        xs = (min_pt.x(), max_pt.x())
        ys = (min_pt.y(), max_pt.y())
        zs = (min_pt.z(), max_pt.z())
        corners = [QVector3D(x, y, z) for x in xs for y in ys for z in zs]
        center = (min_pt + max_pt) * 0.5
        pr = [QVector3D.dotProduct(c - center, r) for c in corners]
        pu = [QVector3D.dotProduct(c - center, u) for c in corners]
        target = center + r * ((max(pr) + min(pr)) * 0.5) \
            + u * ((max(pu) + min(pu)) * 0.5)
        half_w = (max(pr) - min(pr)) * 0.5
        half_h = (max(pu) - min(pu)) * 0.5
        t = math.tan(math.radians(self.fov_deg) / 2.0)
        aspect = self.aspect if self.aspect > 1e-6 else 1.0
        floor = self.min_distance(target)
        half = max(half_h, half_w / aspect, floor) * margin
        distance = half / t
        if self.perspective:
            toward = max(-QVector3D.dotProduct(c - target, f)
                         for c in corners)
            distance += max(toward, 0.0)
        self.target = target
        self.distance = min(max(distance, floor), MAX_DISTANCE)
        if self.perspective:
            self._settle_perspective_fit(corners, margin)
        if free is not None:
            self.target, self.distance = native.frame_box(
                min_pt, max_pt, self.target, self.distance, r, u, f, self.fov_deg, self.aspect, self.perspective,
                (self.shift_x, self.shift_y), tuple(free), margin,
                self.min_distance(self.target), MAX_DISTANCE)

    def _settle_perspective_fit(self, corners, margin: float) -> None:
        """The perspective estimate above backs off for the NEAREST corner
        as if every corner were that close, and centres the box in space,
        not on screen — a deep box came out at 60 % of the window and low.
        Measure the real projection: slide the target so the box is centred
        on screen, then walk the distance in until its wider half-span sits
        at ``1 / margin`` of the window — never past a corner reaching the
        camera."""
        from ingetrazo_core import Vec4 as QVector4D
        goal = 1.0 / margin
        t = math.tan(math.radians(self.fov_deg) / 2.0)
        for _ in range(8):
            mvp = self.projection_matrix() * self.view_matrix()
            xs, ys = [], []
            for c in corners:
                p = mvp.map(QVector4D(c.x(), c.y(), c.z(), 1.0))
                if p.w() <= 1e-9:
                    return
                xs.append(p.x() / p.w())
                ys.append(p.y() / p.w())
            cx = (max(xs) + min(xs)) * 0.5
            cy = (max(ys) + min(ys)) * 0.5
            reach = max((max(xs) - min(xs)) * 0.5, (max(ys) - min(ys)) * 0.5)
            if reach <= 0.0:
                return
            if abs(reach - goal) < 0.005 and abs(cx) < 0.005 and abs(cy) < 0.005:
                return
            f = self.forward()
            r = QVector3D.crossProduct(f, self.up_vector()).normalized()
            u = QVector3D.crossProduct(r, f)
            half_h = self.distance * t
            self.target = self.target + r * (cx * half_h * self.aspect) \
                + u * (cy * half_h)
            nearest = max(-QVector3D.dotProduct(c - self.target, f)
                          for c in corners)
            gap = self.distance - nearest       # eye to the nearest corner
            self.distance = min(max(nearest + gap * reach / goal,
                                    self.min_distance()), MAX_DISTANCE)

    # Yaw / pitch presets for standard architectural views (Z-up convention).
    # Top and Bottom are EXACTLY vertical: at 89° the parallel projection
    # showed every vertical edge as a short line and a plan came out a
    # hair oblique (@pacaeiro, issue #45: «Top and Bottom views are not
    # straight (camera not perpendicular to view)»). ``up_vector`` keeps
    # lookAt and the camera bases from degenerating there.
    _STANDARD_VIEWS = {
        "top":    (math.radians(-90.0), math.radians(90.0)),
        "bottom": (math.radians(-90.0), math.radians(-90.0)),
        "front":  (math.radians(-90.0), 0.0),
        "back":   (math.radians(90.0), 0.0),
        "right":  (0.0, 0.0),
        "left":   (math.radians(180.0), 0.0),
        "iso":    (math.radians(-45.0), math.radians(30.0)),
    }

    #: The four diagonal yaws of the eight isometric views (four above,
    #: four below). -45° comes first so it wins the ties — the classic corner.
    _ISO_YAWS = (-45.0, 45.0, 135.0, 225.0)
    _ISO_PITCH_DEG = 30.0

    def set_view(self, name: str) -> None:
        preset = self._STANDARD_VIEWS.get(name)
        if preset is None:
            return
        self.yaw, self.pitch = preset

    def snap_iso(self) -> None:
        """Snap to the isometric orientation nearest the current camera pose.

        The static ``"iso"`` preset is one fixed corner, so the command
        always swung the camera there no matter where it was. The eight
        isometric views (four diagonal yaws, seen from above or below)
        are the same picture from different corners; the command now
        lands on whichever one the camera already points nearest to, so
        the swing is the shortest available. ``set_view("iso")`` stays
        the deterministic corner for sheets and scripts.
        """
        f = self.forward()
        best_yaw = math.radians(self._ISO_YAWS[0])
        best_pitch = math.radians(self._ISO_PITCH_DEG)
        best_dot = -2.0
        for sign in (1.0, -1.0):
            pitch = sign * math.radians(self._ISO_PITCH_DEG)
            cp = math.cos(pitch)
            sp = math.sin(pitch)
            for yaw_deg in self._ISO_YAWS:
                yaw = math.radians(yaw_deg)
                # The direction the camera looks from that corner.
                d = QVector3D(-cp * math.cos(yaw), -cp * math.sin(yaw), -sp)
                dot = QVector3D.dotProduct(f, d)
                if dot > best_dot:
                    best_dot = dot
                    best_yaw, best_pitch = yaw, pitch
        self.yaw, self.pitch = best_yaw, best_pitch
