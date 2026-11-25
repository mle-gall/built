# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Group: a self-contained chunk of geometry with its own mesh.

The scene's main mesh welds every coincident vertex (sticky geometry), so a
block drawn against a wall fuses with it. A Group isolates geometry into its
*own* :class:`~core.mesh.Mesh` (still in world coordinates for now — no instance
transform yet, that's Components): nothing welds across the group boundary, so it
moves and edits as a unit without dragging the rest of the model.

Selected as a unit and moved/exploded via the commands in :mod:`core.history`.
"""
from __future__ import annotations

import ingetrazo_core as _NATIVE

import ingetrazo_core
import copy
from contextvars import ContextVar
import itertools
import os
import sys
import uuid

from core.mesh import Mesh

_counter = itertools.count(1)
_document_counter = ContextVar("document_group_counter", default=None)


# Fresh uids come from a pool filled by one ``os.urandom`` call: one
# ``uuid4`` (one urandom syscall) per group was 0.26 s of opening the ride,
# whose 104k groups each take one. Filled by ``_refill_uids``, emptied in a
# forked child so two processes never hand out the same pooled bytes.
_UID_POOL: list = []
_UID_BATCH = 4096


def _refill_uids() -> None:
    """Refill the pool with ``_UID_BATCH`` uids of the very form
    ``uuid4().hex[:16]`` had: 16 lowercase hex digits, 64 bits of which
    the 13th digit is the version nibble «4» (a uid read by an older build
    is indistinguishable from one it made)."""
    raw = os.urandom(8 * _UID_BATCH).hex()
    _UID_POOL.extend(raw[i:i + 12] + "4" + raw[i + 13:i + 16]
                     for i in range(0, len(raw), 16))


def _drop_uid_pool() -> None:
    _UID_POOL.clear()


if hasattr(os, "register_at_fork"):
    os.register_at_fork(after_in_child=_drop_uid_pool)


def new_uid() -> str:
    """A fresh group identity: random, so two documents merged (a ``.igz``
    imported as a component) never collide."""
    # list.pop is atomic under the GIL: two threads never get the same one.
    # A thread finding the pool empty refills it; a second refill racing it
    # only adds more fresh uids.
    while True:
        try:
            return _UID_POOL.pop()
        except IndexError:
            _refill_uids()


def _highest_group_number(names) -> int:
    """The largest N among the automatic "Group N" entries in ``names``
    (read in Rust: a Python regex per name was 67 ms of opening 50 000
    placements)."""
    return int(_dm.NATIVE.highest_group_number(names))


def reserve_group_names(names) -> None:
    """Move the automatic "Group N" counter past every N in ``names`` — a
    document that comes back with its names must not hand a new group a
    name it already uses."""
    global _counter
    top = _highest_group_number(names)
    nxt = next(_counter)
    _counter = itertools.count(max(nxt, top + 1))


import contextlib


@contextlib.contextmanager
def document_name_scope(names):
    """Number auto-named groups from a document's own ``names`` while it
    loads, not from the session's counter.

    A file can carry groups with no name (everything saved before names
    were written, some imports). Numbering those from the session counter
    made the names a document saves back depend on whatever was opened
    earlier in the session (issue #943). Inside the scope the counter
    continues the document's own highest "Group N", in document order;
    on exit the session counter moves past everything the document used,
    so a group created by hand afterwards never reuses one of its
    numbers either."""
    global _counter
    local = itertools.count(_highest_group_number(names) + 1)
    token = _document_counter.set(local)
    try:
        yield
    finally:
        first_unused = next(local)
        _document_counter.reset(token)
        session_next = next(_counter)
        _counter = itertools.count(max(first_unused, session_next))


from core import doc_model as _dm

#: Every attribute a group has, in one tuple (``ingetrazo_core.link_clone_shell`` copies
#: them). With the Rust document (core/doc_model.py) the ``DOC_FIELDS`` live
#: in the document and the rest stay Python slots.
GROUP_FIELDS = ("mesh", "name", "layer", "ifc", "billboard", "xform",
                "children", "owner", "context", "text3d", "hidden", "uid",
                "material", "axes", "component", "exploded",
                "explode_offset", "ext", "locked", "source_uid")
DOC_FIELDS = ("name", "layer", "xform", "children", "hidden", "uid",
              "material", "axes", "component", "locked")
#: Custom attributes: document values too (slice 2), handed out as tracked
#: containers that write back when edited in place (core/doc_model.py).
ATTR_FIELDS = ("ifc", "ext", "text3d", "exploded", "explode_offset")
DOC_FIELDS = DOC_FIELDS + ATTR_FIELDS + ("mesh", "billboard", "source_uid")
_PY_FIELDS = tuple(f for f in GROUP_FIELDS if f not in DOC_FIELDS)


class _GroupMethods:
    __slots__ = ()

    def __init__(self, mesh: Mesh | None = None, name: str | None = None,
                 uid: str | None = None) -> None:
        # Rust document: its fields start at these defaults already (no
        # tag, shown, unlocked, unpainted, no matrix, no children, a
        # component), so only the name and the uid are written.
        self.mesh = mesh if mesh is not None else Mesh()
        self.name = name or f"Group {next(_document_counter.get() or _counter)}"
        # Stable identity that survives save/load — what a scene names when
        # it remembers which objects it hides. Fresh per object; a copy gets
        # its own (see ``copy_group``).
        # ``uid``: a stand-in that carries another group's identity (the
        # viewport's placement proxies, 100k of them on the ride: a random
        # uid each was 0.75 s of every resident pack).
        self.uid = uid or new_uid()
        # Set only on the throwaway placement proxies the viewport builds for
        # nested children: the top-level object a click must select. A real
        # group in ``scene.groups`` always has ``owner is None``.
        self.owner = None
        # The group-edit context this entry belongs to, while the viewport is
        # INSIDE a group: its children and their subtrees carry it, so the
        # draw passes know they are the subject and not the surroundings.
        # ``None`` at the root and on anything outside the open context.
        self.context = None
        # ``source_uid``: set only on the parts of a linked file
        # (core/links.py), never in ``__init__``: the uid the part has in
        # its source document, from which its uid in this one is derived —
        # what lets a copy of the link give its parts the uids a fresh link
        # (or the file reopened) gives them. Read it with ``getattr``.

    def is_component(self) -> bool:
        """A component instance, as the user knows it — not merely a group
        that carries a matrix."""
        return self.xform is not None and bool(self.component)

    def adopt(self, children) -> None:
        """Take ``children`` as nested placements, guaranteeing the invariant
        every consumer relies on: **a group that owns placements is always an
        instance**.

        Move, Rotate and Scale compose into ``xform`` for an instance and walk
        the vertices otherwise — and walking the vertices would move the
        group's own mesh while leaving its children behind. An identity
        transform costs nothing and closes that road."""
        from ingetrazo_core import Matrix4 as QMatrix4x4
        children = list(children)
        if getattr(self, "_doc_key", 0):
            # One parent per placement: adopting takes a child out of the
            # list that held it (a loaded file's top level, a subset of a
            # tree), unless that list is this group's own. (A viewport
            # proxy holds no document list: it takes nothing away.)
            for c in children:
                if getattr(c, "_doc_key", 0) and c._parent is not None \
                        and c._parent is not self:
                    c._detach()
        self.children = children
        if self.children and self.xform is None:
            self.xform = QMatrix4x4()

    def is_instance(self) -> bool:
        return self.xform is not None

    def materialize(self) -> None:
        """Bake this instance into its OWN world-space mesh ('make
        unique'): sibling instances keep the shared prototype untouched.
        Editing into an instance no longer does this — the session edits a
        world copy and shares it back on leaving (Scene.begin_group_edit).

        Nested placements are baked in too and then dropped — ``world_mesh``
        already folded them, so keeping them would draw and export the
        child geometry a second time."""
        if self.xform is None and not self.children:
            return
        frame = group_frame(self)
        self.mesh = world_mesh(self)
        self.xform = None
        self.axes = frame                   # baked, but it still faces its way
        self.children = []

    def make_unique(self) -> None:
        """Make Unique: this instance stops sharing with its siblings.

        A COMPONENT stays a component — a definition of its own with this
        one copy, placed where it was: a private copy of its mesh and of
        every subgroup below it, so editing it never reaches the others
        and the subgroups stay subgroups (issue #90, @fafecm: «it explodes
        all the subgroups within it»). It used to come out a group, which
        is not what Make Unique says (#620: «the new component»).

        A copied GROUP (the copy entering a group makes unique on the way
        in) bakes into a classic group without nested placements
        (``materialize``), or keeps its tree, privately, with them."""
        from ingetrazo_core import unique_placement
        return unique_placement(self)

    def __repr__(self) -> str:  # pragma: no cover - debug aid
        kind = " instance" if self.xform is not None else ""
        return (f"Group({self.name!r}{kind}: {len(self.mesh.faces)} faces, "
                f"{len(self.mesh.edges)} edges)")


class Group(_GroupMethods, _dm.NATIVE.Placement):
    """A placement: a group, a component instance or a nested part.
    Its document fields (``DOC_FIELDS``) live in the Rust document
    (core/doc_model.py); see ``_GroupMethods`` for the rest."""
    __slots__ = _PY_FIELDS
    source_uid = _dm.NATIVE.DocAttr("source_uid", optional=True)

    # copy, deepcopy and pickle: a new handle, then every field.
    def __reduce__(self):
        return (_dm.new_handle, (type(self),),
                _dm.handle_state(self, GROUP_FIELDS))

    __setstate__ = _dm.set_handle_state

for _name in ATTR_FIELDS:
    setattr(Group, _name, _dm.attribute(_name))
del _name

class PlacementProxy(Group):
    """A placement the VIEWPORT makes (``Viewport._expand_placements``:
    one per nested part, carrying its composed world matrix, inherited
    tag and paint): a ``Group`` to every draw, pick and ``isinstance``,
    and no part of the document. Every field is a Python slot of its
    own (they shadow the document's descriptors) and the handle names
    no node (``_detached``), so building 18,000 of them per pack writes
    nothing into the document and reads at slot speed."""
    __slots__ = (*DOC_FIELDS, "source_occurrence")

    def __new__(cls, *_args, **_kwargs):
        return _dm.NATIVE.Placement.__new__(cls, _detached=True)

    def __init__(self, mesh: Mesh | None = None, name: str | None = None,
                 uid: str | None = None) -> None:
        _GroupMethods.__init__(self, mesh, name, uid)
        self.billboard = False
        self.ifc = self.ext = self.text3d = None
        self.exploded = self.explode_offset = None
        self.layer = None
        self.hidden = False
        self.locked = False
        self.material = None
        self.xform = None
        self.children = []
        self.axes = None
        self.component = True

    def __reduce__(self):
        return (_dm.new_handle, (type(self),),
                _dm.handle_state(self, GROUP_FIELDS))


# ---- Local axes (issue #44) ---------------------------------------------------

def carry_axes(group, m) -> None:
    """A classic group's geometry was just transformed by ``m`` in world
    space: its axes go with it (the analogue of composing ``xform`` for an
    instance). Instances are left alone — their ``xform`` already moved."""
    if group is None or getattr(group, "xform", None) is not None:
        return
    from ingetrazo_core import Matrix4 as QMatrix4x4
    base = group.axes if group.axes is not None else QMatrix4x4()
    group.axes = m * base


def group_frame(group):
    """The world matrix of ``group``'s own axes, or ``None`` for the world
    axes. ``axes`` lives in the space of the group's mesh, so the frame is
    ``xform · axes``: an instance's placement, a classic group's stored
    axes, or both when a classic group became an instance (a container's
    child once the container is opened)."""
    if group is None:
        return None
    xform = getattr(group, "xform", None)
    axes = getattr(group, "axes", None)
    if xform is None:
        return axes
    return xform if axes is None else xform * axes


def frame_axes(m):
    """``(origin, x, y, z)`` of a frame matrix, the three axes made unit and
    square to each other (a scaled or skewed placement still draws
    orthogonal axes; red first, green squared to it, blue from both — and
    a mirrored placement keeps its handedness). ``m=None`` is the world."""
    from ingetrazo_core import Vec3 as QVector3D
    if m is None:
        return (QVector3D(0, 0, 0), QVector3D(1, 0, 0), QVector3D(0, 1, 0),
                QVector3D(0, 0, 1))
    o = m.map(QVector3D(0, 0, 0))
    x = m.mapVector(QVector3D(1, 0, 0))
    y = m.mapVector(QVector3D(0, 1, 0))
    z = m.mapVector(QVector3D(0, 0, 1))
    if x.length() < 1e-12:
        x = QVector3D(1, 0, 0)
    x = x.normalized()
    y = y - x * QVector3D.dotProduct(y, x)
    if y.length() < 1e-12:
        y = QVector3D.crossProduct(z, x)
    y = y.normalized()
    zz = QVector3D.crossProduct(x, y).normalized()
    if QVector3D.dotProduct(zz, z) < 0:
        zz = -zz                           # a mirrored placement
    return o, x, y, zz


def make_billboard_group(image_path: str, height: float, name: str,
                         aspect: float, position=None) -> Group:
    """A face-me billboard group: one textured quad, ``height`` metres tall,
    anchored at ``position`` (origin by default). The viewport rotates it
    around that anchor to face the camera."""
    from ingetrazo_core import Vec3 as QVector3D
    mesh = Mesh()
    w = height * aspect
    p = position if position is not None else QVector3D(0, 0, 0)
    quad = [p + QVector3D(-w / 2, 0, 0), p + QVector3D(w / 2, 0, 0),
            p + QVector3D(w / 2, 0, height), p + QVector3D(-w / 2, 0, height)]
    face = mesh.add_face(quad)
    face.attrs["texture"] = {"path": str(image_path), "sw": w, "sh": height}
    g = Group(mesh, name=name)
    g.billboard = True
    return g


#: ``iter_placements(group, base=None)``: ``(group, world_matrix)`` for
#: ``group`` and every nested placement below it, depth first, each matrix
#: already composed from the root down (``None`` only while the whole chain is
#: untransformed: a classic group whose mesh already sits in world
#: coordinates). The geometry lives in ``g.mesh`` (prototype, local
#: coordinates) and the matrix puts it in the world; nothing is copied. A
#: native iterator over the Rust document
#: (``rust/ingetrazo_core/src/model/placement_walks.rs``), lazy as a
#: generator; ``ingetrazo_core.group_iter_painted_placements`` adds the paint
#: each placement's default faces show.
iter_placements = _NATIVE.group_iter_placements


#: ``world_mesh(group, placements=None)``: a world-space mesh for a group:
#: the mesh itself for a classic group, a transformed deep copy for an
#: instance, every nested placement (or those ``placements`` yields)
#: merged into one mesh (``rust/ingetrazo_core/src/model/group_mesh.rs``).
world_mesh = _NATIVE.group_world_mesh


def copy_group(group, delta=None, _in_definition=False, _ifc_copy_context=None,
               _project_occurrence=False):
    """A pastable duplicate of ``group``, optionally translated by ``delta``.

    A component instance stays an instance: the duplicate SHARES the prototype
    mesh and only gets its own transform (copying an instance adds a
    sibling, O(1)). A classic group gets a deep mesh copy.

    Everything inside a COMPONENT belongs to its definition, so the groups
    nested in it are shared too, at every depth (issue #97: editing a group
    inside one copy left the other copies as they were). A classic child
    becomes an identity placement first — same place in the world — so the
    original and the copy hold the same mesh and an edit inside either
    shares back to both."""
    from ingetrazo_core import Matrix4 as QMatrix4x4, Vec3 as QVector3D
    copy_root = _ifc_copy_context is None
    if copy_root:
        _ifc_copy_context = {'groups': {}, 'openings': {}, 'copies': []}
    share = _in_definition or (group.xform is not None
                               and bool(getattr(group, "component", True)))
    # A linked file's placement: its parts are the source's, read-only,
    # and the copy is a live link of the same file (``ingetrazo_core.link_copy_parts``).
    linked = _is_link(group)
    if share and not linked and not _project_occurrence:
        for child in group.children or ():
            if child.xform is None:
                child.xform = QMatrix4x4()
                child.component = False     # it was a group, it stays one
    t = QMatrix4x4()
    if delta is not None:
        t.translate(QVector3D(delta))
    if group.xform is not None or _project_occurrence:
        g = Group(group.mesh, name=group.name)
        g.xform = t * group.xform if group.xform is not None else None
        # The local axes live in the mesh's space, which the copy shares:
        # they come along as they are. Dropping them turned a pasted
        # rotated group's axes back to the world's (issue #78, @pacaeiro:
        # Copy makes a classic group an instance, and Paste copies that).
        if getattr(group, "axes", None) is not None:
            g.axes = QMatrix4x4(group.axes)
    else:
        g = Group(_NATIVE.group_transformed_mesh(group.mesh, t), name=group.name)
        if getattr(group, "axes", None) is not None:
            g.axes = t * group.axes
    if g.mesh is not group.mesh:
        # A deep geometry copy owns its face tags; shared definitions instead
        # scope their tags to the placement uid when exported.
        for face in g.mesh.faces:
            if face.attrs.get("ifc"):
                face.attrs["ifc"] = {k: v for k, v in face.attrs["ifc"].items()
                                     if k != "guid"}
    g.layer = group.layer
    from copy import deepcopy
    g.ifc = deepcopy({k: v for k, v in group.ifc.items() if k != "guid"}) if group.ifc else None
    _ifc_copy_context['groups'][group.uid] = g.uid
    _ifc_copy_context['copies'].append(g)
    if g.ifc:
        from core.ifc_identity import ensure_guid
        for opening in g.ifc.get('openings', []) or []:
            if isinstance(opening, dict):
                previous = opening.pop('guid', None)
                if previous:
                    _ifc_copy_context['openings'][previous] = ensure_guid(opening)
    g.billboard = group.billboard
    g.text3d = dict(group.text3d) if group.text3d else None
    g.ext = copy.deepcopy(group.ext) if getattr(group, "ext", None) else None
    g.hidden = group.hidden
    g.locked = getattr(group, "locked", False)
    g.material = dict(group.material) if getattr(group, "material", None) else None
    g.component = getattr(group, "component", True)
    g.exploded = dict(group.exploded) if group.exploded else None
    g.explode_offset = group.explode_offset
    # Nested placements ride along untranslated: ``delta`` already moved the
    # parent, and a child's transform is relative to it.
    if linked:
        g.children = ingetrazo_core.link_copy_parts(group, g)
    else:
        g.children = [copy_group(c, _in_definition=share,
                                 _ifc_copy_context=_ifc_copy_context,
                                 _project_occurrence=_project_occurrence)
                      for c in (group.children or ())]
    if copy_root:
        # Resolve after the complete tree is copied: a filling can precede
        # its host in traversal order. External references stay external.
        for placed in _ifc_copy_context['copies']:
            if not placed.ifc:
                continue
            for opening in placed.ifc.get('openings', []) or []:
                if isinstance(opening, dict) and opening.get('group_uid') in _ifc_copy_context['groups']:
                    opening['group_uid'] = _ifc_copy_context['groups'][opening['group_uid']]
            filling = placed.ifc.get('fills')
            if isinstance(filling, str) and filling in _ifc_copy_context['openings']:
                placed.ifc['fills'] = _ifc_copy_context['openings'][filling]
    return g


def _is_link(group) -> bool:
    """Whether ``group`` is a placed linked file (core/links.py), without
    importing it for every copied group."""
    ext = getattr(group, "ext", None)
    link = ext.get("link") if isinstance(ext, dict) else None
    return isinstance(link, dict) and bool(link.get("path"))


def _copy_definition_mesh(mesh):
    # StoreMesh.__deepcopy__ forks the store chunks under fresh keys.
    from copy import deepcopy
    return deepcopy(mesh)


def _hull_2d(pts):
    """Convex hull of 2D points (monotone chain), counter-clockwise from
    the lowest x, as an ``(H, 2)`` array. Points strictly inside the
    quadrilateral of the four axis extremes, then of sixteen directions'
    extremes, are dropped first (Akl-Toussaint): the 189k vertices of a
    hedge come down to a few hundred before the scan. In Rust
    (``rust/ingetrazo_core/src/group_frame.rs``)."""
    import numpy as np
    from ingetrazo_core import group_hull_2d
    return group_hull_2d(np.ascontiguousarray(np.asarray(pts, dtype=np.float64)[:, :2]))


def frame_from_points(positions) -> tuple:
    """An orthonormal frame for a group, derived from its vertices: the yaw
    about Z whose footprint is tightest, with Z kept upright.

    The usual convention gives every group its own axes and draws the
    selection box in them, so the box hugs the object. Nothing stores those axes here yet (an
    imported group is baked to world coordinates, a classic group never had
    a frame), so they are derived — and derived *exactly*: the minimum-area
    rectangle around a point set always has a side flush with an edge of its
    convex hull, so trying each hull edge finds the true optimum rather than
    guessing at it. The barbecue in piscina.igz sits at 23.5 degrees and needs
    half the footprint the world axes give it.

    Keeping Z upright is the domain's assumption, not a shortcut: buildings,
    walls and furniture stand up, and a box tilted off vertical would read as
    an error even where it wrapped tighter. An earlier attempt derived the
    frame from face geometry instead and picked triangulation DIAGONALS for
    the long axis, which is how a rectangle ends up with a skew frame."""
    import numpy as np
    from ingetrazo_core import Vec3 as QVector3D, group_frame_axis
    world = (QVector3D(1.0, 0.0, 0.0), QVector3D(0.0, 1.0, 0.0),
             QVector3D(0.0, 0.0, 1.0))
    if positions is None or len(positions) < 3:
        return world
    u = group_frame_axis(np.ascontiguousarray(
        np.asarray(positions, dtype=np.float64)[:, :2]))
    if u is None:
        return world
    return (QVector3D(u[0], u[1], 0.0), QVector3D(-u[1], u[0], 0.0),
            QVector3D(0.0, 0.0, 1.0))


def _local_points(mesh):
    """The vertex positions of ``mesh`` as an ``(N, 3)`` float64 array, or
    None when it has none. A mesh still deferred (a lazy mesh of a big
    document) is NOT built for it: its points are the welded positions the
    build would make, in the same order. A box drawn over the 590 groups of
    a 5-million-face model built every one of them, past 12 GB."""
    import numpy as np
    from ingetrazo_core import deferred_weld
    from core.mesh import DeferredMesh
    if type(mesh) is DeferredMesh:
        d = mesh.__dict__
        hit = d.get("_deferred_plan")
        if hit is not None:
            pos = hit.pos
        else:
            s = d["_deferred_source"]
            pos = deferred_weld(s["pos"], s.get("sequential", False))[0]
        return np.array(pos, dtype=np.float64) if len(pos) else None
    from core.store_mesh import StoreMesh
    if mesh.__class__ is StoreMesh:
        # The store's dense positions, in ``mesh.vertices`` order: walking
        # the views made a cached QVector3D per vertex, and selecting a
        # linked file measures every mesh of it (the selection box).
        pos = mesh.__dict__["_store"].arrays()["pos"]
        return (np.array(pos, dtype=np.float64).reshape(-1, 3)
                if len(pos) else None)
    verts = mesh.vertices
    if not verts:
        return None
    return np.array([v.position_tuple() for v in verts], dtype=np.float64)


def placement_points(group, cache=None):
    """Every vertex position of ``group`` and its nested placements, in WORLD
    space, as an ``(N, 3)`` float64 array. ``cache`` (``{id(mesh): array}``)
    lets a caller keep each prototype's local array across calls — filled
    here, validated by the caller (the viewport keys it on the chunk's
    revision): rebuilding the plaza's arrays cost 73 ms per box.

    A bounding box only ever needed the POINTS, but the only way to get them
    used to be ``world_mesh`` — which welds a merged copy of the whole
    component to answer a question about eight corners. On the hedge that was
    23 s per selection once its geometry moved into nested placements (5 s
    before, already the wrong shape of work). Here each prototype's array is
    built ONCE and every placement is one matrix multiply over it."""
    import numpy as np
    out: list = []
    if cache is None:
        cache = {}
    for g, m in _NATIVE.group_model_placements(group):
        arr = cache.get(id(g.mesh))
        if arr is None:
            arr = _local_points(g.mesh)
            if arr is None:
                continue
            cache[id(g.mesh)] = arr
        if m is not None:
            from ingetrazo_core import group_placed_points
            arr = group_placed_points(np.ascontiguousarray(arr, dtype=np.float64),
                                      np.ascontiguousarray(m, dtype=np.float64))
        out.append(arr)
    return np.concatenate(out) if out else np.empty((0, 3))


def oriented_bounds(mesh, frame=None, points=None) -> tuple:
    """``(frame, lo, hi)`` — the mesh's extent along ``frame``'s axes (derived
    with :func:`local_frame` when not given). ``lo``/``hi`` are the min/max
    coordinates in that frame, so the box corners are
    ``sum(axis * c for axis, c in zip(frame, corner))``.

    ``points`` short-circuits the mesh: an ``(N, 3)`` world-space array from
    :func:`placement_points`, so a component with nested placements is not
    merged into one mesh just to be measured."""
    import numpy as np
    from ingetrazo_core import Vec3 as QVector3D
    if frame is None:
        frame = (QVector3D(1.0, 0.0, 0.0), QVector3D(0.0, 1.0, 0.0),
                 QVector3D(0.0, 0.0, 1.0))
    if points is not None:
        if len(points) == 0:
            return frame, (0.0, 0.0, 0.0), (0.0, 0.0, 0.0)
        pos = points
    else:
        verts = mesh.vertices
        if not verts:
            return frame, (0.0, 0.0, 0.0), (0.0, 0.0, 0.0)
        pos = np.array([[v.position.x(), v.position.y(), v.position.z()]
                        for v in verts], dtype=np.float64)
    from ingetrazo_core import group_oriented_extent
    axes = np.array([[a.x(), a.y(), a.z()] for a in frame], dtype=np.float64)
    lo, hi = group_oriented_extent(
        np.ascontiguousarray(np.asarray(pos, dtype=np.float64).reshape(-1, 3)), axes)
    return frame, lo, hi


def oriented_box_corners(frame, lo, hi) -> list:
    """The eight corners of an oriented box, in world coordinates."""
    from ingetrazo_core import Vec3 as QVector3D
    u, v, w = frame
    out = []
    for i in range(8):
        out.append(u * (hi[0] if i & 1 else lo[0])
                   + v * (hi[1] if i & 2 else lo[1])
                   + w * (hi[2] if i & 4 else lo[2]))
    return [QVector3D(p) for p in out]


def effective_material(group):
    """The paint a placement's default faces show: its own, else the
    top-level object's it belongs to (a nested piece of a painted
    component). ``None`` for an unpainted one."""
    if group is None:
        return None
    own = getattr(group, "material", None)
    if own:
        return own
    owner = getattr(group, "owner", None)
    if owner is not None and owner is not group:
        return getattr(owner, "material", None) or None
    return None
