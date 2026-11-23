"""Viewport services used by the Rust inference rule engine for one mouse event."""
import math

from ingetrazo_core import Vec3 as V

from core import snap as S
from core.snap_query import point_result


def _coords(p):
    return p.toTuple() if p is not None else None


def _edge(e):
    return (e.a.toTuple(), e.b.toTuple()) if e is not None else None


class SnapFrame:
    def __init__(self, options, query):
        self.__dict__.update(options)
        self.query = query
        self.axes = S._axis_tuples()
        self.world = self.candidate_world.toTuple()
        self.start = _coords(self.start_point)
        self.normal = _coords(self.work_plane_normal)
        self.on_plane = self.work_plane_normal if self.face_under_cursor else None
        self.line_start = None if self.diagonal_span else self.start_point
        if self.acquired_point is not None and self.start_point is not None:
            if (self.acquired_point-self.start_point).lengthSquared() < 1e-12:
                self.acquired_point = None
        if self.diagonal_span and self.start_point is not None and self.acquired_points:
            self.acquired_points = [p for p in self.acquired_points if p is not None and
                                    (p-self.start_point).lengthSquared() >= 1e-12]
        self.acquired = _coords(self.acquired_point)
        self.shift_direction = _coords(self.shift_lock_dir)
        self.shift_reference = None
        if self.shift_lock_ref and self.shift_lock_ref[1] in S.AXIS_COLORS:
            p, axis = self.shift_lock_ref
            self.shift_reference = (p.toTuple(), "xyz".index(axis))
        self.arrow = "xyz".index(self.axis_lock) if self.axis_lock else None
        self.sticky = "xyz".index(self.sticky_axis) if self.sticky_axis else None
        self.reference = ((*_edge(self.reference_edge), self.reference_mode == 'perpendicular')
                          if self.reference_edge is not None and self.reference_mode else None)
        self.acquired_edge_points = _edge(self.acquired_edge)
        self.acquired_normal = _coords(self.acquired_face_normal)
        self.angle = self.inference_angle_deg
        self.magnet = self.magnetic_axis_deg
        self.screen = self.screen_axis_px
        self.threshold = self.threshold_px
        self.et = self.threshold if self.edge_threshold_px is None else self.edge_threshold_px
        self.diagonal = self.diagonal_span
        self.planar = self.axis_in_plane
        self.radial = self.radial_arm
        self.shift = self.shift_held
        self.dwell = self.dwell_axis_lock
        self.projectable = self.project_onto_line is not None
        self.on_face = self.face_under_cursor
        self.linear = self.linear_mode == 'all'
        self.parperp = self.linear_mode != 'off'
        self.has_close = (self.start_point is not None and self.chain_first_point is not None
                          and self.start_point is not self.chain_first_point)
        self.has_refs = bool(self.acquired_points)
        self.circles = getattr(self.scene, 'circles', ())
        self.has_circles = bool(self.circles)
        self.rims = []
        if self.circles:
            from core.axes import plane_axes
            for circle in self.circles:
                data = (circle.center.toTuple(), circle.radius, circle.normal.toTuple())
                basis = plane_axes(circle.normal)[0].toTuple()
                hit = self.query.rim(data, self.world, basis, self.et)
                if hit is not None:
                    self.rims.append((data, hit, circle))
        self.has_outline = self.face_outline is not None
        self.has_crossing = self.face_crossing is not None

    def project_line(self, start, direction):
        return _coords(self.project_onto_line(V(*start), V(*direction)))

    def hidden(self, point):
        return self.is_occluded is not None and self.is_occluded(V(*point))

    def locked(self, start, direction, axis, shift, guide_reference, orient):
        s, d = V(*start), V(*direction)
        locked = self.project_onto_line(s, d)
        if orient and V.dotProduct(locked-s, d) < 0:
            d = -d
        ref = self.acquired_point if orient else s
        result = S._lock_line_snaps(self.scene, s, d, *self.candidate_pixel,
                                    self.world_to_pixel, self.threshold, self.is_occluded, ref, self.chain_first_point,
                                    self.project_onto_line, self.hover_lock_edge, self.hover_lock_plane,
                                    self.are_occluded, self.query)
        name = 'xyz'[axis] if axis is not None else None
        color = (S.AXIS_COLORS[name] if name else self.shift_lock_color
                 if shift and self.shift_lock_color is not None else S.COLOR_REFERENCE)
        if result is None:
            result = S.SnapResult(locked, 'axis' if name else 'reference', color, axis=name)
        else:
            result.lock_color = color
        if guide_reference and result.guide is None:
            p, k = guide_reference
            p = V(*p)
            result.guide = (p, self.project_onto_line(p, S._AXIS_VECTORS['xyz'[k]]))
            result.guide_color = S.AXIS_COLORS['xyz'[k]]
        return result

    def close(self):
        px = self.world_to_pixel(self.chain_first_point)
        if px is not None and math.dist(px, self.candidate_pixel) <= self.threshold:
            return S.SnapResult(self.chain_first_point, 'close', S.COLOR_CLOSE)

    def point(self, midpoints):
        centers = [] if midpoints else [(p, c) for p, c in self.face_centers or ()
                                        if self.start_point is None or (p-self.start_point).length() >= 1e-6]
        quadrants = [] if midpoints else [q for c in self.circles
                                          for q in S._circle_quadrants(c.center, c.radius, c.normal)]
        return point_result(self.query, self.scene.edges, self.threshold, self.is_occluded,
                            midpoints=midpoints, centers=centers, quadrants=quadrants, hidden_many=self.are_occluded)

    def from_point(self, start, direction, line):
        return S._from_point_snap(self.scene, V(*start), V(*direction), *self.candidate_pixel,
                                  self.world_to_pixel, self.threshold, self.is_occluded, extra_point=self.acquired_point,
                                  line_dir=V(*line) if line is not None else None, query=self.query)

    def perpendicular_from_start(self, start, linear):
        s = V(*start)
        draw = self.candidate_world-s
        if draw.length() <= 1e-6:
            return
        for e in [self.scene.edges[i] for i in self.query.starting_edges(start)] + list(self.start_edges or ()):
            if not S._point_on_segment_world(s, e.a, e.b):
                continue
            d = S._direction_from_edge(e, 'perpendicular', self.work_plane_normal)
            if d is None or abs(V.dotProduct(draw.normalized(), d)) < math.cos(math.radians(8)):
                continue
            crossings = self.query.crossings(self.et, (start, d.toTuple()), draw.toTuple())
            for point, _ in crossings:
                p = V(*point)
                if V.dotProduct(p-s, draw) > 0 and not self.hidden(point):
                    return S.SnapResult(p, 'intersection', S.COLOR_ENDPOINT, guide=(s, p))
            if linear:
                ref = self.from_point(start, draw.toTuple(), None)
                if ref is not None:
                    return ref
            if self.target(include_ends=True, exclude=start) is not None:
                return
            return S.SnapResult(self.project_onto_line(s, d), 'reference', S.COLOR_REFERENCE)

    def extension(self):
        for p, end, d in self.query.extensions(self.start, self.world, self.et,
                                               self.project_line if self.projectable else None, _coords(self.on_plane)):
            if self.hidden(p):
                continue
            for hit, _ in self.query.crossings(self.et, (p, d)):
                if not self.hidden(hit):
                    if (V(*hit)-V(*end)).lengthSquared() < 1e-12:
                        return S.SnapResult(V(*hit), 'endpoint', S.COLOR_ENDPOINT)
                    return S.SnapResult(V(*hit), 'intersection', S.COLOR_ENDPOINT, guide=(V(*end), V(*hit)))
            return S.SnapResult(V(*p), 'extension', S.COLOR_EXTENSION, guide=(V(*end), V(*p)))

    def intersection(self, axis):
        hits = self.query.crossings(self.et, circles=[c for c, _, _ in self.rims])
        points = [V(*p) for p, _ in hits]
        k = S._first_visible(points, self.is_occluded, self.are_occluded)
        if k is not None and (axis is None or S._vertex_on_line(
                points[k], self.start_point, S._AXIS_VECTORS['xyz'[axis]])):
            return S.SnapResult(points[k], 'endpoint' if hits[k][1] else 'intersection',
                                S.COLOR_ENDPOINT)

    def piercing(self):
        outline = self.face_outline()
        if outline is not None:
            return S._edge_face_snap(*self.candidate_pixel, self.scene, self.world_to_pixel,
                                     self.et, self.is_occluded, outline, query=self.query)

    def tangent(self):
        pending = []
        for c in self.circles:
            for p in S._circle_tangent_points(c.center, c.radius, c.normal, self.line_start):
                pixel = self.world_to_pixel(p)
                if pixel is not None and (d := math.dist(pixel, self.candidate_pixel)) <= self.threshold:
                    pending.append((d, p, 'tangent', S.COLOR_TANGENT, None, True))
        win = S._resolve_point_candidates(pending, self.is_occluded, self.are_occluded)
        if win:
            return S.SnapResult(win[1], win[2], win[3])

    def hidden_batch(self, points):
        vectors = [V(*p) for p in points]
        if len(vectors) > 1 and self.are_occluded is not None:
            return self.are_occluded(vectors)
        return [self.is_occluded(v) for v in vectors]

    def target(self, include_ends=False, skip_clipped=False, exclude=None):
        rims = [] if include_ends else [hit for _, hit, _ in self.rims]
        return self.query.target(self.et, rims, self.project_line if self.projectable else None,
                                 self.hidden_batch if self.is_occluded is not None else None,
                                 include_ends, skip_clipped, exclude)

    def crossing_result(self, point, start):
        p = V(*point)
        return S.SnapResult(p, 'intersection', S.COLOR_ENDPOINT, guide=(V(*start), p))

    def edge_reference_result(self, point, index, axis):
        p = V(*point)
        return S.SnapResult(p, 'from_point', S.COLOR_ENDPOINT, guide=(self.acquired_point, p),
                            guide_color=S.AXIS_COLORS["xyz"[axis]],
                            context=getattr(self.scene.edges[index], 'context', None))

    def edge_result(self, point, index, kind):
        if kind == 'on_edge':
            edge = self.scene.edges[index]
            return S.SnapResult(V(*point), kind, S.COLOR_ON_EDGE, edge=edge,
                                context=getattr(edge, 'context', None))
        return S.SnapResult(V(*point), kind, S.COLOR_ON_EDGE)

    def face_crossing_result(self):
        crossing = self.face_crossing()
        if crossing is not None:
            return S.SnapResult(crossing[0], 'intersection', S.COLOR_ENDPOINT, context=crossing[1])

    def direction_result(self, kind, start, direction, reference):
        p = self.project_onto_line(V(*start), V(*direction))
        return S.SnapResult(p, kind, S.COLOR_REFERENCE, guide=(V(*reference), p) if reference else None)

    def two_points(self):
        return S._two_point_snap(self.acquired_points, self.candidate_world, *self.candidate_pixel,
                                 self.world_to_pixel, self.threshold, self.is_occluded)

    def encouraged(self):
        hit = S._first_point_from_point(self.acquired_point, self.candidate_world, *self.candidate_pixel,
                                        self.world_to_pixel, self.threshold, self.is_occluded, self.on_plane, self.project_onto_line,
                                        self.free_3d)
        if hit is None and self.on_plane is not None:
            hit = S._in_plane_with_point_snap(self.acquired_point, self.candidate_world, self.on_plane,
                                              *self.candidate_pixel, self.world_to_pixel, self.threshold, self.is_occluded)
        return hit

    def axis_result(self, point, axis, locked):
        name = 'xyz'[axis]
        return S.SnapResult(V(*point), 'axis' if locked else 'axis_inference', S.AXIS_COLORS[name], axis=name)

    def color_lock(self, result, axis):
        result.lock_color = S.AXIS_COLORS['xyz'[axis]]
        return result

    def work_point(self):
        if self.on_face:
            return S.SnapResult(self.candidate_world, 'on_face', S.COLOR_ON_FACE, context=self.face_context)
        return S.SnapResult(self.candidate_world, 'none')
