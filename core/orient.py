# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Consistent outward orientation of closed solids.

:func:`orient_outward` flips the faces of a closed solid so every boundary
normal points outward and marks interior partitions (``face.interior``);
:func:`is_closed` and :func:`signed_volume` are the checks around it. The
pass is the engine's (``rust/ingetrazo_core/src/orient.rs``: parity rays
from each face's interior probe, partitions peeled off to a fixpoint, the
flips that would worsen an open solid rolled back), over the mesh's scan.
The Python pass it reproduced bit for bit was deleted once the object mesh
was retired; its answers are pinned in ``tests/test_orient_golden.py`` and
``tests/fixtures/push_native_orient_golden.json.gz``.
"""
from __future__ import annotations

import sys

from ingetrazo_core import Vec3 as QVector3D

from core.mesh import turn_face_over  # noqa: F401  (part of what plugins see)

#: Parity rays per face and their jitter (radians), as the pass was tuned.
_RAYS = 7
_JITTER = 3e-4


def is_closed(mesh) -> bool:
    """Whether the mesh has no boundary: every edge borders at least two faces.
    A necessary condition for inside/outside (and thus orientation) to mean
    anything. Open sheets and flat drawings return ``False``."""
    if not mesh.faces:
        return False
    from core import push_scans
    got = push_scans.is_closed(mesh)        # a store-backed mesh, natively
    if got is not None:
        return got
    return all(len(e.faces) >= 2 for e in mesh.edges)


def signed_volume(mesh) -> float:
    """Signed volume of the mesh as currently wound (sum of tetrahedra from the
    origin over each triangulated face). For a consistently-oriented closed
    solid its magnitude is the real volume and its sign reports the global
    winding (positive == outward by the right-hand rule). Mixed winding gives a
    meaningless value — use it only to confirm consistency. Interior partitions
    (marked by :func:`orient_outward`) are not boundary and are skipped — their
    arbitrary winding would bias the sum."""
    total = 0.0
    for f in mesh.faces:
        if f.interior:
            continue
        for a, b, c in f.triangulate():
            total += QVector3D.dotProduct(a, QVector3D.crossProduct(b, c)) / 6.0
    return total


def orient_outward(mesh, seed: int = 12345, only=None) -> list:
    """Flip the faces of a closed solid so every boundary normal points
    outward, and mark interior partitions (``face.interior``). Returns the
    faces that were flipped.

    No-op (returns ``[]``) on a mesh that isn't closed — an open sheet or flat
    drawing has no outside, so there is nothing to orient. ``only`` (faces)
    restricts the pass to the edge-connected components those faces belong
    to; every other face keeps its winding and its ``interior`` mark.

    The pass is native and needs the engine's scans; with them switched off
    (``INGETRAZO_SCANS=0``, a caller's own loops in Python) the mesh is
    still scanned for this pass alone."""
    if not mesh.faces:
        return []
    from core import _native, scan_index
    mod, hypot = _native.native, _native.qt_hypot()
    if mod is None or hypot is None or not hasattr(mod, "orient_decision"):
        raise RuntimeError("orient_outward needs the native engine (ingetrazo_core.orient_decision)")
    scan = scan_index.scan(mesh, fresh=not scan_index.store_backed(mesh))
    if scan is None:
        scan = scan_index.Scan(mesh)
    marks, flips = mod.orient_decision(
        scan.native, None if only is None else list(only), seed, _RAYS,
        float(_JITTER), hypot, sys.version_info >= (3, 12))
    from core.store_mesh import StoreMesh
    if type(mesh) is StoreMesh:
        from core.mesh import swap_orientation_paint
        marked = scan.faces([i for i, _ in marks]) if marks else []
        out = scan.faces(flips) if flips else []
        if marked or out:
            mesh._store.apply_orientation(
                [(f.key, value) for f, (_, value) in zip(marked, marks)],
                [f.key for f in out])
            for f in out:
                swap_orientation_paint(f)
        return out
    if marks:
        for f, (_, v) in zip(scan.faces([i for i, _ in marks]), marks):
            f.interior = v
    out = scan.faces(flips) if flips else []
    for f in out:
        turn_face_over(f)
    return out
