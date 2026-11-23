# SPDX-License-Identifier: GPL-3.0-or-later
"""Reference partition until the document-suite gate."""
from ingetrazo_core import Vec3 as QVector3D
from core.intersect import world_polys

def _partition_faces(mesh, segments):
    """Weld the lines, then partition each original face in its own plane."""
    from core import scan_index
    from core.edits import _ScanEdges
    from core.mesh import edge_flags, stamp_edge_flags
    from core.topology import plan_edge_split

    lines = {}
    for a, b in segments:
        sc = scan_index.scan(mesh)
        pieces, cuts = (_ScanEdges(sc).split_plan(a, b) if sc is not None
                        else plan_edge_split(mesh.edges, a, b))
        for edge, point in cuts.items():
            flags = edge_flags(edge)
            was_cut_line = lines.pop(edge.key, None) is not None
            for half in mesh.split_edge(edge, point):
                stamp_edge_flags(half, flags)
                if was_cut_line:
                    lines[half.key] = half
        for p, q in pieces:
            edge = mesh.add_edge(p, q)
            lines[edge.key] = edge

    # Collect all lines before replacing any face. One cut can be a chord
    # on one operand and part of a closed interior contour on the other.
    sc = scan_index.scan(mesh)
    cuts_by_face = {}
    polys = {}
    for edge in lines.values():
        a, b = edge.a, edge.b
        bounds = list(zip(a.toTuple(), b.toTuple()))
        box = (tuple(min(v) - 1e-4 for v in bounds), tuple(max(v) + 1e-4 for v in bounds))
        faces = sc.faces(sc.faces_meeting(box)) if sc is not None else mesh.faces
        for face in faces:
            if face.key not in polys:
                polys[face.key] = world_polys(face)
            poly = polys[face.key]
            if poly is not None:
                cuts = poly.native.clip(a.toTuple(), b.toTuple())
                if cuts:
                    cuts_by_face.setdefault(face, []).extend(
                        (QVector3D(*p), QVector3D(*q)) for p, q in cuts)

    for face, cuts in cuts_by_face.items():
        _replace_regions(mesh, face, polys[face.key], cuts)


def _replace_regions(mesh, face, poly, cuts):
    from copy import deepcopy
    from core.arrangement import planar_rebuild
    from core.make_faces import _probe
    from core.triangulate import _newell

    boundary = [(a, b) for ring in poly.rings for a, b in zip(ring, ring[1:] + ring[:1])]
    _, regions = planar_rebuild(boundary + cuts, poly.rings[0][0], poly.n)
    # Arrangement also returns a cell for each existing opening.
    covered = []
    for outer, holes in regions:
        if len(poly.rings) > 1:
            mid = _probe(outer, holes, poly.n)
            if mid is None:
                continue
            if poly.native.in_hole(mid.toTuple()):
                continue
        if QVector3D.dotProduct(_newell(outer), poly.n) < 0:
            outer.reverse()
            for hole in holes:
                hole.reverse()
        covered.append((outer, holes))
    if len(covered) > 1:
        attrs, interior = face.attrs, face.interior
        mesh.remove_face(face)
        for outer, holes in covered:
            new = mesh.add_face(outer, holes)
            new.attrs = deepcopy(attrs)
            new.interior = interior


