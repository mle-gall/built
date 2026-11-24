# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""SKP import seam — IngeTrazo's own reader, behind one seam.

IngeTrazo aims to open **any** ``.skp`` (old → recent). This module is the
single seam between IngeTrazo and *how* an ``.skp`` is read, so the parser can
evolve independently of the app. The backend is **OpenSKP**
(https://github.com/iamahsanmehmood/openskp, MIT) or a maintained fork: pure
Python, offline, no Wine, no proprietary code. (Until 2026-09-28 a second
path ran a proprietary SDK through the former external converter for files
the reader could not open; it has been removed.)

**Parse then apply.** A backend *parses* a file into a plain **payload** (world-
space face loops, no ``Scene`` touched). The heavy parse runs outside the undo
history; :func:`apply_payload` then adds the geometry cheaply inside a command.
When no backend can produce geometry, :func:`parse_skp` raises
:class:`NeedsConverter` before any mutation and the UI says the file could
not be read -- so a failed parse never leaves a half-applied edit.

Nothing here imports a parser at module load — a missing OpenSKP is just an
unavailable backend.
"""
from __future__ import annotations

import logging
from pathlib import Path


class NeedsConverter(Exception):
    """No backend can read this ``.skp``; the caller reports it. (The name is
    historical: it once meant "use the external converter".) Carries the
    path and detected format."""

    def __init__(self, path, fmt: str, reason: str = "") -> None:
        msg = f"No pure SKP backend for {path} (format={fmt})"
        super().__init__(f"{msg}: {reason}" if reason else msg)
        self.path = Path(path)
        self.format = fmt
        self.reason = reason


def detect_format(path) -> str:
    """Best-effort container detection from the file's first bytes, no parser
    involved:

    * ``"skp"``     — a ``.skp`` document (UTF-16 ``Sketch…`` model marker, or
      a ``PK`` ZIP-wrapped container). Covers legacy MFC and 2021+ files alike —
      both begin with the same marker, so the *era* is not observable from the
      magic bytes (OpenSKP handles the range, so we don't need to tell them
      apart here).
    * ``"unknown"`` — not recognisably a ``.skp`` file / unreadable.
    """
    try:
        # The first 64 bytes only: read_bytes() pulled the whole file into
        # memory to look at them — 442 MB for a big model, before the
        # import had even started.
        with open(path, "rb") as fh:
            head = fh.read(64)
    except OSError:
        return "unknown"
    if head[:4] == b"PK\x03\x04":
        return "skp"
    if b"S\x00k\x00e\x00t\x00c\x00h" in head:   # UTF-16LE "Sketch"
        return "skp"
    return "unknown"


class _OpenSkpBackend:
    """Pure-Python OpenSKP backend. Parses via :mod:`formats.skp_openskp`, which
    imports ``openskp`` lazily. ``available()`` is True only when the package is
    importable; ``supports`` covers any recognised ``.skp`` file (OpenSKP reads
    a broad version range). A parse that yields no geometry returns ``None`` from
    :meth:`parse`, so the seam falls back to the converter."""

    name = "openskp"

    def available(self) -> bool:
        try:
            import openskp  # noqa: F401
        except Exception as exc:  # noqa: BLE001 — optional dependency
            # NEVER silent: a broken editable install (it happened when the
            # repo moved and ~/openskp died) otherwise degrades every .skp
            # to the Wine converter and reads as "slow / does not open".
            import sys
            print(f"IngeTrazo: openskp backend unavailable ({exc!r}); "
                  f".skp imports fall back to the external converter",
                  file=sys.stderr)
            return False
        return True

    def supports(self, fmt: str) -> bool:
        return fmt == "skp"

    def parse(self, path, progress=None):
        from formats import skp_openskp
        return skp_openskp.parse(path, progress=progress)


#: Ordered list of pure-Python backends. Extend/replace as coverage grows.
_BACKENDS: list = [_OpenSkpBackend()]


def backends_status() -> list[tuple[str, bool]]:
    """``(name, available)`` for each pure backend — for diagnostics / an
    About-style report of what can be opened without the converter."""
    return [(b.name, b.available()) for b in _BACKENDS]


def can_handle(path) -> bool:
    """True when a pure backend is available and recognises ``path``. Note this
    does NOT guarantee a non-empty parse — the actual geometry check happens in
    :func:`parse_skp`, which raises :class:`NeedsConverter` on an empty result."""
    fmt = detect_format(path)
    return any(b.available() and b.supports(fmt) for b in _BACKENDS)


def parse_skp(path, progress=None) -> dict:
    """Parse ``path`` with the first pure backend that produces geometry, and
    return its payload. Raises :class:`NeedsConverter` when no pure backend can
    read the file (unrecognised format, parser error, or an empty parse) — the
    caller reports the file as unreadable. Touches no ``Scene``.

    The returned payload owns its geometry: ``apply_payload`` releases each
    group after building it. Pass ``consume=False`` to ``apply_payload`` if
    the caller needs to retain/reapply the parsed data."""
    path = Path(path)
    fmt = detect_format(path)
    error = None
    for backend in _BACKENDS:
        if not (backend.available() and backend.supports(fmt)):
            continue
        try:
            payload = backend.parse(path, progress=progress)
        except Exception as exc:  # noqa: BLE001 — a parser that chokes → try next / fall back
            # The user only hears "could not read"; the reader's own error
            # goes to the error log, or a failed import is undiagnosable.
            logging.getLogger(__name__).warning(
                "SKP backend %s could not read %s", backend.name, path,
                exc_info=True)
            error = exc
            payload = None
        # Protos count as geometry: a file whose whole content is
        # components placed once yields no plain groups at all, and
        # reading that as "empty parse" reported it as unreadable.
        if payload and (payload.get("groups") or payload.get("protos")
                        or payload.get("empty")):
            payload["_consume_geometry"] = True
            return payload
    raise NeedsConverter(path, fmt, str(error or "")) from error


def apply_payload(scene, payload, progress=None, *, consume: bool | None = None) -> str:
    """Add a parsed payload's geometry to ``scene`` as reference groups (an
    isolated ``Mesh`` per group, like the big-DAE import). Runs the same
    clean-up the DAE reference import does — coplanar fusion (merges the raw
    ``.skp`` polygons and drops double-sided duplicates) and smooth-edge
    softening — so a ``.skp`` opened through the pure backend looks identical
    to one that came through the converter. Cheap relative to the parse; the
    caller wraps it in a command for undo. Returns the backend name.

    ``progress(frac, text)``, when given, is called now and then (at most
    every 0.15 s) while the scene is built: on a 946 000-face file this
    stage ran for minutes without returning to the event loop, and Windows
    marked the window "Not Responding" (#787). It is only called while no
    half-built group is in the scene — inside a mesh not yet added, or
    between whole groups.

    ``consume=None`` uses the ownership marker set by ``parse_skp``; ordinary
    caller-built payloads remain reusable. ``consume=True`` drops each part's
    geometry after building it, then its placement records after placement.
    A consumed payload must be parsed again to retry after a failed import.
    SnapshotImport redo restores the built groups, without reading the payload.
    ``consume=False`` retains the input geometry explicitly."""
    import gc
    from ingetrazo_core import Vec3 as QVector3D
    from core.group import Group
    from core.mesh import Mesh
    from formats.dae import _add_fused
    from formats.fuse import fuse_coplanar_loops, soften_smooth_edges

    # Mass object construction ahead (vertices/edges/faces per group): the
    # generational GC re-scans the ever-growing heap throughout and can cost
    # more than the build itself on many-group files. Collection is merely
    # deferred — import cycles are still reclaimed at re-enable.
    _gc_was_enabled = gc.isenabled()
    gc.disable()
    try:
        if consume is None:
            consume = payload.get("_consume_geometry", False)
        return _apply_payload_inner(scene, payload, progress, consume=consume)
    finally:
        if _gc_was_enabled:
            gc.enable()


def _soften_bulk_edges(mesh, vertices, edge_map, segments, tick):
    """Use the weld's vertices and face pass's edge index for display flags.

    Exact stored positions need no second weld lookup. Other endpoints
    (including points welded across cell boundaries) use the final registry:
    a point's vertex during construction is not necessarily what vertex_at
    returns after a later point has claimed its cell. Cache those lookups too.
    """
    from ingetrazo_core import Vec3 as QVector3D

    point_ids = {v.position.toTuple(): i for i, v in enumerate(vertices)}
    vertex_ids = None
    missing = object()

    def resolve(point):
        nonlocal vertex_ids
        vertex = mesh.vertex_at(QVector3D(*point))
        if vertex_ids is None:
            vertex_ids = {v: i for i, v in enumerate(vertices)}
        index = vertex_ids.get(vertex)
        point_ids[point] = index
        return index

    count = len(vertices)
    for k, (a, b) in enumerate(segments):
        if not k & 4095:
            tick()
        a, b = tuple(a), tuple(b)
        ia = point_ids.get(a, missing)
        ib = point_ids.get(b, missing)
        if ia is missing:
            ia = resolve(a)
        if ib is missing:
            ib = resolve(b)
        if ia is None or ib is None or ia == ib:
            continue
        key = ia * count + ib if ia <= ib else ib * count + ia
        edge = (mesh.find_edge(vertices[ia], vertices[ib]) if edge_map is None
                else edge_map.get(key))
        if edge is not None:
            edge.soft = True


def _apply_payload_inner(scene, payload, progress=None, *, consume=False) -> str:
    import time
    from ingetrazo_core import Vec3 as QVector3D
    from core.group import Group
    from core.mesh import Mesh
    from formats.dae import _add_fused
    from formats.fuse import fuse_coplanar_loops, soften_smooth_edges

    groups = payload.get("groups", []) or []
    protos = payload.get("protos", []) or []
    remaining = {}
    if consume:
        for parts in (groups, protos):
            for part in parts:
                key = id(part)
                remaining[key] = remaining.get(key, 0) + 1

    def _release(part):
        if not consume:
            return
        key = id(part)
        remaining[key] -= 1
        if remaining[key]:
            return
        del remaining[key]
        # Replace references; never clear lists another part may share.
        part["faces"] = []
        for field in ("soft_edges", "loose_edges", "curves"):
            part.pop(field, None)

    n_parts = max(1, len(groups) + 2 * len(protos))
    done = [0, 0.0]               # parts built, time of the last report

    def _tick(step=0):
        """Report progress through this stage (0.75 → 0.96 of the bar),
        throttled so the event loop is pumped without slowing the build."""
        done[0] += step
        if progress is None:
            return
        now = time.monotonic()
        if now - done[1] < 0.15:
            return
        done[1] = now
        progress(0.75 + 0.21 * min(1.0, done[0] / n_parts),
                 "Building geometry…")

    def _q(p):
        # Payload corners are plain [x, y, z] lists from the openskp
        # adapter, or QVector3D from the image-quad path — normalize for
        # the QVector3D-based fuse/add_face clean-up below.
        return p if hasattr(p, "x") else QVector3D(p[0], p[1], p[2])

    def _add_lines(mesh, loose_edges):
        """The file's loose edges (lines on no face), welded to the faces'
        corners where they meet them."""
        for a, b in loose_edges or ():
            try:
                mesh.add_edge(_q(a), _q(b))
            except ValueError:      # endpoints weld to one vertex
                continue

    def _build_mesh(faces, soft_edges=None, loose_edges=None, curves=None):
        if soft_edges is not None and payload.get("backend") == "openskp":
            from ingetrazo_core import skp_definition
            mesh = Mesh()
            if hasattr(mesh, "_store"):
                return skp_definition(mesh, faces, soft_edges, loose_edges, curves)
        mesh = _build_faces(faces, soft_edges)
        _add_lines(mesh, loose_edges)
        for segments in curves or ():
            cid = Mesh.next_curve_id()
            for a, b in segments:
                va, vb = mesh.vertex_at(_q(a)), mesh.vertex_at(_q(b))
                if va is not None and vb is not None:
                    edge = mesh.find_edge(va, vb)
                    if edge is not None:
                        edge.curve = cid
        return mesh

    def _build_faces(faces, soft_edges=None):
        mesh = Mesh()
        if soft_edges is not None:
            # The backend carried the file's ORIGINAL polygons plus the
            # file's own per-edge display flags — add everything as-is and
            # soften exactly the flagged edges. No coplanar fusion: the .skp
            # format keeps coplanar same-material faces separate with their
            # edges visible (glass mullions, beam/column lines), so fusing them
            # dissolved real user lines. Built in one vectorized bulk pass —
            # the per-face add_face welding dominated big imports.
            from core.topology import _maximal_holes
            from formats.skp_coordinates import flatten_rings
            flat_rings: list = []
            corner_count = 0
            ring_sizes: list = []
            ring_counts: list = []
            attrs_list: list = []
            for outer, holes, attrs in faces:
                if holes and len(holes) > 1:
                    try:
                        holes = _maximal_holes(
                            [[QVector3D(*p) for p in h] for h in holes])
                        holes = [[p.toTuple() for p in h] for h in holes]
                    except Exception:  # noqa: BLE001 — degenerate hole ring
                        continue       # add_face would have raised: skip face
                rings = [outer] + [h for h in (holes or [])]
                ring_counts.append(len(rings))
                for r in rings:
                    ring_sizes.append(len(r))
                    flat_rings.append(r)
                    corner_count += len(r)
                attrs_list.append(attrs)
            bulk = None
            if ring_counts:
                if corner_count >= 1024:          # corners; bulk wins above ~700
                    vertices, inverse = mesh.bulk_weld(flatten_rings(flat_rings, corner_count))
                    # A store has its own native pair index. Passing a Python
                    # dict to it would materialize the entire mesh.
                    edge_map = {} if soft_edges and type(mesh) is Mesh else None
                    mesh.add_faces_welded(vertices, inverse, ring_sizes, ring_counts,
                                          attrs_list, edge_map=edge_map)
                    bulk = vertices, edge_map
                else:
                    # Small group: the bulk pass's fixed NumPy cost loses to
                    # the plain per-face walk (a file with a thousand small
                    # groups pays that fixed cost a thousand times).
                    corners = [_q(p) for ring in flat_rings for p in ring]
                    ci = 0
                    ri = 0
                    for fi, nr in enumerate(ring_counts):
                        rings = []
                        for k in range(nr):
                            n = ring_sizes[ri + k]
                            rings.append(corners[ci:ci + n])
                            ci += n
                        ri += nr
                        try:
                            face = mesh.add_face(rings[0], rings[1:] or None)
                        except Exception:  # noqa: BLE001 — degenerate
                            continue
                        if attrs_list[fi]:
                            face.attrs.update(attrs_list[fi])
            if soft_edges and bulk is not None:
                _soften_bulk_edges(mesh, *bulk, soft_edges, _tick)
            elif soft_edges:
                # Soften through the WELD itself: resolve each flagged
                # segment's endpoints to their shared vertices and mark that
                # edge — the weld tolerance is the matching tolerance, so
                # float noise between the payload's segment coordinates and
                # the stored vertex positions can never drop a flag (the old
                # rounded-cell comparison could).
                for k, (a, b) in enumerate(soft_edges):
                    if not k & 4095:
                        _tick()     # one group can carry 1.7 M flags
                    va = mesh.vertex_at(QVector3D(a[0], a[1], a[2]))
                    vb = mesh.vertex_at(QVector3D(b[0], b[1], b[2]))
                    if va is None or vb is None or va == vb:
                        continue
                    e = mesh.find_edge(va, vb)
                    if e is not None:
                        e.soft = True
            return mesh
        # Legacy payloads without edge flags: the DAE-style clean-up.
        # Fusion works on plain loops; faces with holes (window rings) keep
        # their explicit topology and are added directly.
        raw = [([_q(p) for p in outer], attrs)
               for outer, holes, attrs in faces if not holes]
        for item in fuse_coplanar_loops(raw):
            _add_fused(mesh, [item])
        for outer, holes, attrs in faces:
            if not holes:
                continue
            try:
                face = mesh.add_face([_q(p) for p in outer],
                                     [[_q(p) for p in h] for h in holes])
            except Exception:  # noqa: BLE001 — skip a degenerate polygon
                continue
            if attrs:
                face.attrs.update(attrs)
        soften_smooth_edges(mesh)
        return mesh

    from ingetrazo_core import skp_apply_metadata
    skp_apply_metadata(scene, payload)

    from ingetrazo_core import skp_apply_geometry
    skp_apply_geometry(scene, payload, _build_mesh, _release, _tick)
    if consume:
        payload["groups"] = []
        payload["protos"] = []
    back = payload.get("back_color")
    if back and getattr(scene, "back_face_color", None) is None:
        # Adopt the file's style back-face colour so unpainted faces seen
        # from behind read like they did for the author.
        scene.back_face_color = tuple(back)
    for dim in scene.dimensions:        # groups are in: hold their vertices
        if hasattr(dim, "bind"):
            dim.bind(scene)
    scene.version += 1
    return payload.get("backend", "?")


from ingetrazo_core import skp_apply_folders as _import_tag_folders


def load_skp(scene, path, progress=None) -> str:
    """Convenience: :func:`parse_skp` then :func:`apply_payload` into ``scene``.
    Returns the backend name; raises :class:`NeedsConverter` if none applies.
    (The UI splits these two so the heavy parse runs outside the undo history.)"""
    return apply_payload(scene, parse_skp(path, progress=progress))
