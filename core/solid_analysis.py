# SPDX-License-Identifier: GPL-3.0-or-later
"""Watertightness diagnosis and repair commands shared by both solid inspectors."""
from __future__ import annotations

from typing import Optional

from ingetrazo_core import Vec3 as QVector3D
from core import bim
from core.history import Command
from core.i18n import tr


def _live(face) -> bool:
    """A face that is part of the skin (an interior partition is not)."""
    return not getattr(face, "interior", False)


def _steps(face):
    """Every boundary step of ``face`` as a pair of vertices."""
    for lp in (face.loop, *face.hole_loops):
        n = len(lp)
        for i in range(n):
            yield lp[i], lp[(i + 1) % n]


def _step_key(a, b) -> frozenset:
    return frozenset((a.key, b.key))


def _face_key(face) -> tuple:
    """Same key ⇔ same corners, whatever the starting point or winding."""
    return (frozenset(v.key for v in face.loop),
            frozenset(frozenset(v.key for v in h) for h in face.hole_loops))


def internal_faces(mesh) -> list:
    """Faces inside the solid that are not marked as partitions.

    First every face repeating another one's corners (only the first copy
    stays: a doubled top is still a top, not two). Then, among the rest,
    every face whose edges ALL see more than two faces — a wall with skin
    on both of its sides."""
    seen: set = set()
    duplicates, rest = [], []
    for f in mesh.faces:
        if not _live(f):
            continue
        key = _face_key(f)
        if key in seen:
            duplicates.append(f)
        else:
            seen.add(key)
            rest.append(f)
    counts: dict = {}
    for f in rest:
        for a, b in _steps(f):
            k = _step_key(a, b)
            counts[k] = counts.get(k, 0) + 1
    walls = [f for f in rest
             if all(counts[_step_key(a, b)] > 2 for a, b in _steps(f))]
    return duplicates + walls


def _copy_faces(faces):
    """A throwaway mesh of ``faces``, and the original of each copied face —
    for asking a mutating pass what it WOULD do."""
    from core.mesh import Mesh
    copy = Mesh()
    origin = {}
    for f in faces:
        g = copy.add_face([v.position for v in f.loop],
                          [[v.position for v in h] for h in f.hole_loops])
        g.interior = f.interior
        origin[g.key] = f
    return copy, origin


def reversed_faces(mesh) -> list:
    """The faces of a closed mesh whose front looks inward: exactly the ones
    :func:`core.orient.orient_outward` would turn. Asked on a copy, so
    inspecting never changes the model. An open mesh has no inside, so
    nothing on it is reversed."""
    import ingetrazo_core as native
    from core.orient import is_closed, orient_outward
    if not is_closed(mesh):
        return []
    # The parity pass costs seconds on thousands of faces, and almost every
    # piece is fine: one whose shared edges all agree and whose signed
    # volume is positive already looks out. Only the others are asked.
    pieces: dict = {}
    # Edge-connected pieces and their edge agreement, from the store
    # (``model/solid_checks.rs``).
    every = list(mesh.faces)
    for f, c in zip(every, native.solid_face_components(mesh, every)):
        pieces.setdefault(c, []).append(f)
    suspect = []
    for faces in pieces.values():
        skin = [f for f in faces if _live(f)]
        if native.solid_incoherent_edges(mesh, skin) or _signed_volume(skin) <= 0.0:
            suspect.extend(faces)
    if not suspect:
        return []
    copy, origin = _copy_faces(suspect)
    return [origin[g.key] for g in orient_outward(copy)]


def _signed_volume(faces) -> float:
    """Signed volume of ``faces`` as wound (positive = looking out). The
    partitions are skipped: they are not skin."""
    total = 0.0
    for f in faces:
        if not _live(f):
            continue
        for a, b, c in f.triangulate():
            total += QVector3D.dotProduct(a, QVector3D.crossProduct(b, c))
    return total / 6.0


def inspect_mesh(mesh) -> dict:
    """Classify every edge and face of *mesh* and compute the closed volume.

    Returns ``{"faces", "edges", "stray", "open", "over", "internal",
    "reversed", "watertight", "volume"}`` where the edge problems are lists
    of Edge objects, ``internal``/``reversed`` lists of Face objects and
    ``volume`` is in m³ (or ``None`` when the mesh is not closed, or
    closed with faces turned inward).
    """
    from core.store_mesh import EdgeView, StoreMesh
    if isinstance(mesh, StoreMesh):
        stray, open_borders, over = (
            [EdgeView(mesh, k) for k in keys]
            for keys in mesh._store.snapshot().edge_defects())
    else:
        stray, open_borders, over = [], [], []
        for e in mesh.edges:
            n = sum(1 for f in e.faces if _live(f))
            if n == 0:
                stray.append(e)
            elif n == 1:
                open_borders.append(e)
            elif n > 2:
                over.append(e)

    watertight = (not stray and not open_borders and not over
                  and bool(mesh.faces))
    turned = reversed_faces(mesh)
    volume = None
    if watertight and not turned:
        # A reversed face makes the signed sum lie (a 2 m cube with its
        # top turned reads 2.667 m³), so a volume is only given once
        # every face looks out.
        volume = bim.face_set_volume([f for f in mesh.faces if _live(f)])

    return {
        "faces": len(mesh.faces),
        "edges": len(mesh.edges),
        "stray": stray,
        "open": open_borders,
        "over": over,
        "internal": internal_faces(mesh) if over else [],
        "reversed": turned,
        "watertight": watertight,
        "volume": volume,
    }


def is_sound(report: dict) -> bool:
    """Watertight AND every face looking out: a solid with nothing to fix."""
    return report["watertight"] and not report["reversed"]


def _problem_text(report: dict) -> str:
    """Human breakdown of the problems, empty string when there are none."""
    parts = []
    if report["open"]:
        parts.append(tr("{n} open borders", n=len(report["open"])))
    if report["stray"]:
        parts.append(tr("{n} stray edges", n=len(report["stray"])))
    if report["over"]:
        parts.append(tr("{n} overconnected", n=len(report["over"])))
    if report.get("internal"):
        parts.append(tr("{n} internal faces", n=len(report["internal"])))
    if report.get("reversed"):
        parts.append(tr("{n} reversed faces", n=len(report["reversed"])))
    return " · ".join(parts)


def _edge_clusters(edges) -> list:
    """``edges`` split into the runs that touch each other — a hole's whole
    rim is one problem, not one per edge."""
    parent: dict = {}

    def find(x):
        while parent.setdefault(x, x) != x:
            parent[x] = parent[parent[x]]
            x = parent[x]
        return x

    for e in edges:
        parent[find(e.v0.key)] = find(e.v1.key)
    runs: dict = {}
    for e in edges:
        runs.setdefault(find(e.v0.key), []).append(e)
    return list(runs.values())


#: How each kind of problem is named and coloured on screen, in the order
#: :func:`problem_items` walks them.
KINDS = {
    "open": ("Open border (hole)", (230, 40, 40)),
    "stray": ("Stray edges", (255, 140, 0)),
    "over": ("Overconnected edges", (150, 60, 220)),
    "internal": ("Internal face", (30, 130, 255)),
    "reversed": ("Reversed face", (230, 0, 160)),
}


def problem_items(report: dict) -> list:
    """The problems one at a time, as ``{"kind", "ents"}`` dicts (``ents``
    the edges or faces to blame): each hole rim, each run of stray edges,
    each internal or reversed face, and each run of overconnected edges
    that no internal face explains — the edges of a doubled face or of an
    unmarked partition are ITS problem, solved with it."""
    items = []
    for key in ("open", "stray"):
        items.extend({"kind": key, "ents": run}
                     for run in _edge_clusters(report[key]))
    explained = {_step_key(a, b) for f in report.get("internal", ())
                 for a, b in _steps(f)}
    rest = [e for e in report["over"]
            if _step_key(e.v0, e.v1) not in explained]
    items.extend({"kind": "over", "ents": run}
                 for run in _edge_clusters(rest))
    for key in ("internal", "reversed"):
        items.extend({"kind": key, "ents": [f]} for f in report.get(key, ()))
    return items


def _item_points(item) -> list:
    pts = []
    for ent in item["ents"]:
        if hasattr(ent, "loop"):
            pts.extend(v.position for v in ent.loop)
        else:
            pts.extend((ent.v0.position, ent.v1.position))
    return pts


# ---------------------------------------------------------------------------
# Repair (mutates the mesh — run it through RepairSolidsCommand for undo)
# ---------------------------------------------------------------------------

def _border_loops(mesh, edges=None) -> list:
    """Closed rims of open borders (of ``edges`` only, when given), each as
    its vertex loop. A rim that branches (a vertex with more than two open
    edges) is skipped: which way it closes is a choice, not a repair."""
    open_edges = [e for e in (mesh.edges if edges is None else edges)
                  if sum(1 for f in e.faces if _live(f)) == 1]
    at: dict = {}
    for e in open_edges:
        at.setdefault(e.v0.key, []).append(e)
        at.setdefault(e.v1.key, []).append(e)
    used: set = set()
    loops = []
    for e in open_edges:
        if e.key in used:
            continue
        used.add(e.key)
        start, cur = e.v0, e.v1
        loop = [start]
        ok = len(at[start.key]) == 2
        while ok and cur != start:
            ways = at[cur.key]
            nxt = [x for x in ways if x.key not in used]
            if len(ways) != 2 or not nxt:
                ok = False
                break
            loop.append(cur)
            used.add(nxt[0].key)
            cur = nxt[0].v1 if nxt[0].v0 == cur else nxt[0].v0
        if ok and len(loop) >= 3:
            loops.append(loop)
    return loops


def _flat(points, tolerance: float) -> bool:
    """Do ``points`` lie on one plane (and span some area)?"""
    nx = ny = nz = 0.0
    n = len(points)
    for i in range(n):
        p, q = points[i], points[(i + 1) % n]
        nx += (p.y() - q.y()) * (p.z() + q.z())
        ny += (p.z() - q.z()) * (p.x() + q.x())
        nz += (p.x() - q.x()) * (p.y() + q.y())
    length = (nx * nx + ny * ny + nz * nz) ** 0.5
    if length < 1e-12:
        return False
    nx, ny, nz = nx / length, ny / length, nz / length
    c = [sum(p.x() for p in points) / n, sum(p.y() for p in points) / n,
         sum(p.z() for p in points) / n]
    return all(abs((p.x() - c[0]) * nx + (p.y() - c[1]) * ny
                   + (p.z() - c[2]) * nz) <= tolerance for p in points)


def _agree_with_neighbour(mesh, face) -> None:
    """Wind a new cap so it walks its rim against the face beside it — the
    way two faces of one skin meet — even where the mesh is still open and
    the orientation pass cannot decide."""
    a, b = face.loop[0], face.loop[1]
    edge = mesh.find_edge(a, b)
    other = next((f for f in (edge.faces if edge else ())
                  if f != face and _live(f)), None)
    if other is None:
        return
    if any(x == a and y == b for x, y in _steps(other)):
        face.reverse_rings()


def _cap(mesh, loop):
    """Close the hole ``loop`` rims with a face, wound like its neighbours;
    ``None`` (and nothing done) when the rim is not flat."""
    from core.topology import _PLANAR_TOLERANCE
    pts = [v.position for v in loop]
    if not _flat(pts, _PLANAR_TOLERANCE):
        return None
    cap = mesh.add_face([QVector3D(p) for p in pts])
    _agree_with_neighbour(mesh, cap)
    return cap


def why_not_repairable(mesh, item) -> Optional[str]:
    """Why ``item`` has no single right repair, or ``None`` when it has."""
    kind = item["kind"]
    if kind == "open":
        loops = _border_loops(mesh, item["ents"])
        if len(loops) != 1 or len(loops[0]) != len(item["ents"]):
            return tr("This hole branches: close it by drawing its faces")
        from core.topology import _PLANAR_TOLERANCE
        if not _flat([v.position for v in loops[0]], _PLANAR_TOLERANCE):
            return tr("This hole is not flat: close it by drawing its faces")
    elif kind == "over":
        return tr("Edges shared by more than two faces: separate the "
                  "solids or erase a face by hand")
    return None


def repair_item(mesh, item) -> dict:
    """Repair ONE problem of :func:`problem_items`, in place. Returns the
    same counts as :func:`repair_mesh`; all zero when it had no single
    right repair (see :func:`why_not_repairable`)."""
    done = {"stray": 0, "duplicates": 0, "holes": 0, "partitions": 0,
            "reversed": 0}
    if why_not_repairable(mesh, item) is not None:
        return done
    kind, ents = item["kind"], item["ents"]
    if kind == "open":
        if _cap(mesh, _border_loops(mesh, ents)[0]) is not None:
            done["holes"] = 1
    elif kind == "stray":
        for e in ents:
            if not e.faces and e in mesh.edges:
                mesh.remove_edge(e)
                done["stray"] += 1
        mesh.prune_orphan_vertices()
    elif kind == "internal":
        face = ents[0]
        key = _face_key(face)
        if any(f != face and _face_key(f) == key for f in mesh.faces):
            mesh.remove_face(face)
            done["duplicates"] = 1
        else:
            face.interior = True
            done["partitions"] = 1
    elif kind == "reversed":
        from core.mesh import turn_face_over
        turn_face_over(ents[0])
        mesh._winding_rev = getattr(mesh, "_winding_rev", 0) + 1
        done["reversed"] = 1
    return done


def repair_mesh(mesh) -> dict:
    """Fix what has one right answer, in place. Returns how many of each
    repair were made: ``{"stray", "duplicates", "holes", "partitions",
    "reversed"}``."""
    from core.orient import orient_outward
    done = {"stray": 0, "duplicates": 0, "holes": 0, "partitions": 0,
            "reversed": 0}

    # 1. Stray edges: no face at all (an edge holding only interior
    #    partitions is part of them, not loose drawing).
    for e in [e for e in mesh.edges if not e.faces]:
        mesh.remove_edge(e)
        done["stray"] += 1

    # 2. Duplicated faces: the second copy of the same corners.
    seen: set = set()
    for f in list(mesh.faces):
        key = _face_key(f)
        if key in seen:
            mesh.remove_face(f)
            done["duplicates"] += 1
        else:
            seen.add(key)

    # 3. Holes with a flat rim get their cap back.
    for loop in _border_loops(mesh):
        if _cap(mesh, loop) is not None:
            done["holes"] += 1

    # 4. The engine's own pass: partitions marked interior, the skin
    #    turned outward (on a closed mesh; an open one has no outside).
    before = {f.key for f in mesh.faces if not _live(f)}
    flipped = orient_outward(mesh)
    done["reversed"] = len(flipped)
    done["partitions"] = sum(1 for f in mesh.faces
                             if not _live(f) and f.key not in before)
    if flipped:
        mesh._winding_rev = getattr(mesh, "_winding_rev", 0) + 1

    mesh.prune_orphan_vertices()
    return done


class RepairSolidsCommand(Command):
    """Repair several meshes (the loose one, closed groups' prototypes) as
    ONE undo step. Snapshot undo per mesh: the repairs weld, split and
    turn too much for a clean inverse, and the snapshot keeps every Face
    object, so references held elsewhere stay valid."""

    def __init__(self, meshes, fix=None) -> None:
        self.meshes = list(meshes)
        #: ``fix(mesh) -> counts``: :func:`repair_mesh` unless one problem
        #: alone is being repaired (:func:`repair_item`).
        self.fix = fix or repair_mesh
        self.before: list = []
        self.after: list = []
        self.interior_before: list = []
        self.interior_after: list = []
        self.done: dict = {}

    @staticmethod
    def _interior(mesh) -> dict:
        # The snapshot does not carry the partition marks; they are a
        # repair's output, so undo must give them back too.
        return {f: f.interior for f in mesh.faces}

    def do(self, scene) -> None:
        if not self.after:
            total = {}
            for m in self.meshes:
                self.before.append(m.capture_state())
                self.interior_before.append(self._interior(m))
                for k, v in self.fix(m).items():
                    total[k] = total.get(k, 0) + v
                self.after.append(m.capture_state())
                self.interior_after.append(self._interior(m))
            self.done = total
        else:
            self._restore(scene, self.after, self.interior_after)
            return
        self._touch(scene)

    def undo(self, scene) -> None:
        self._restore(scene, self.before, self.interior_before)

    def _restore(self, scene, snaps, marks) -> None:
        for m, snap, flags in zip(self.meshes, snaps, marks):
            m.restore_state(snap)
            for f, value in flags.items():
                f.interior = value
            m._winding_rev = getattr(m, "_winding_rev", 0) + 1
        self._touch(scene)

    @staticmethod
    def _touch(scene) -> None:
        # Edges and faces the repair erased must not stay selected.
        alive = set(scene.mesh.edges) | set(scene.mesh.faces)
        for s in [s for s in scene.selection
                  if (hasattr(s, "v0") or hasattr(s, "loop"))
                  and s not in alive]:
            scene.selection.discard(s)
        from ingetrazo_core import dirty_group_chunks
        dirty_group_chunks(scene)
        scene.version += 1
