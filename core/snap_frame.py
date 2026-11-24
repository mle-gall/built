"""Viewport services used by the Rust inference rule engine for one mouse event."""
import math

from ingetrazo_core import Vec3 as V, hidden_batch, FrameState

from core import snap as S
from core.snap_query import point_result


def _coords(p):
    return p.toTuple() if p is not None else None


class SnapFrame:
    _api = S

    def __init__(self, options, query):
        self.__dict__.update(options)
        self.query = query
        self.threshold = self.threshold_px
        self.et = self.threshold if self.edge_threshold_px is None else self.edge_threshold_px
        self.on_face = self.face_under_cursor
        self.on_plane = self.work_plane_normal if self.on_face else None
        self.circles = getattr(self.scene, "circles", ())
        self.state = FrameState(self, query, options, S._axis_tuples(), S._MAX_AXIS_REACH)

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
        centers, quadrants = self.state.point_inputs(self, midpoints)
        return point_result(self.query, self.scene.edges, self.threshold, self.is_occluded,
                            midpoints=midpoints, centers=centers, quadrants=quadrants, hidden_many=self.are_occluded)

    def from_point(self, start, direction, line):
        return S._from_point_snap(self.scene, V(*start), V(*direction), *self.candidate_pixel,
                                  self.world_to_pixel, self.threshold, self.is_occluded, extra_point=self.acquired_point,
                                  line_dir=V(*line) if line is not None else None, query=self.query)

    def perpendicular_from_start(self, start, linear):
        return self.state.perpendicular(self.query, self, start, linear)

    def extension(self):
        return self.state.extension(self.query, self)

    def intersection(self, axis):
        return self.state.intersection(self.query, self, axis)

    def piercing(self):
        outline = self.face_outline()
        if outline is not None:
            return S._edge_face_snap(*self.candidate_pixel, self.scene, self.world_to_pixel,
                                     self.et, self.is_occluded, outline, query=self.query)

    def tangent(self):
        return self.state.tangent(self)

    def hidden_batch(self, points):
        return hidden_batch(points, self.is_occluded, self.are_occluded)

    def target(self, include_ends=False, skip_clipped=False, exclude=None):
        return self.state.target(self.query, self, include_ends, skip_clipped, exclude)

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
