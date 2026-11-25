# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Linear dimension — a measured annotation between two world points.

A ``Dimension`` records two endpoints ``a``/``b`` and an ``offset`` vector
giving where the dimension line sits relative to the measured segment.

An endpoint placed ON a vertex is **anchored** to it (a dimension belongs to
the geometry it measures): the endpoint reads the vertex's
position live, so scaling, moving or stretching the drawing takes the
dimension along and re-measures it — the first thing a user of DriveMeca's
video missed (2026-09-20, Marco: «dibujamos algo, lo acotamos, después lo
escalamos y la acotación se queda allí, no se redimensiona con el dibujo
y no se actualiza la nueva medición»). A vertex inside a component reads
through its placement's transform, so moving the instance moves the
dimension too. When the vertex is gone (erased, welded away) the endpoint
freezes where it last was. An endpoint snapped to a midpoint, an edge or a
face has no vertex to hold and stays static.

It is an annotation, not geometry: it lives in ``Scene.dimensions`` and is
drawn as a screen-space overlay (extension lines + dimension line + value
label), not in the mesh.

Two kinds, the usual convention: **aligned** (``axis`` is None) — the dimension
line parallel to a–b, measuring its length — and **linear** (``axis`` is
``"x"``/``"y"``/``"z"``) — the dimension line parallel to that axis,
measuring the segment's extent along it, the extension lines square to the
axis. A slanted line dimensioned by pulling the cursor to its SIDE gives
the vertical extent, pulling ABOVE gives the horizontal one, pulling square
off the line gives the aligned length (@pacaeiro, issue #50: «the Dimension
tool should be able to measure aligned (as it does now), but also linear»).
"""
from __future__ import annotations

import weakref

from core.doc_model import Record, record_fields

import ingetrazo_core as _NATIVE
from ingetrazo_core import Matrix4 as QMatrix4x4, Vec3 as QVector3D



#: Holds an endpoint remembers (see Dimension._live).
_MAX_HOLDS = 8


@record_fields('vertex', 'mesh', 'chain', 'element_key', 'occurrence_path')
class VertexAnchor(Record):
    """A hold on one mesh vertex: the vertex, the mesh that registers it
    (to notice when it is gone) and the chain of groups whose transforms
    place it in the world — empty for loose geometry, ``[group]`` for a
    classic group, ``[component, child, …]`` down a nested placement."""

    __slots__ = ("vertex", "mesh", "chain", "element_key", "occurrence_path")

    def __init__(self, vertex, mesh, chain=()) -> None:
        self.vertex = vertex
        self.mesh = mesh
        self.chain = tuple(chain)
        self.element_key = vertex.key
        self.occurrence_path = tuple(g.uid for g in self.chain)

    def matrix(self):
        """The composed placement transform, or ``None`` when every group
        on the chain keeps its geometry in world coordinates."""
        m = None
        for g in self.chain:
            x = getattr(g, "xform", None)
            if x is None:
                continue
            m = QMatrix4x4(x) if m is None else m * x
        return m

    def alive(self, scene=None) -> bool:
        """Whether the hold still means something: the mesh registers this
        very vertex (not erased, not welded into another one), something
        references it, and — given the scene — the mesh is still part of
        the model: the loose mesh, or a group the scene still lists. A
        group exploded, or grouped into another, leaves its old mesh
        behind with the vertex in it, positions frozen for ever."""
        v = self.vertex
        try:
            # By identity: looked up by POSITION, a vertex passing over
            # another during a Move preview read as gone, and the
            # dimension dropped it (#563).
            if not v.edges or not self.mesh.registers(v):
                return False
        except Exception:  # noqa: BLE001 — a mesh that cannot answer
            return False
        if scene is None:
            return True
        if not self.chain:
            return self.mesh is getattr(scene, "loose_mesh", scene.mesh)
        if self.mesh is not getattr(self.chain[-1], "mesh", None):
            # The group holds another mesh now: entering a component swaps
            # the instance onto a world copy of its definition (placement
            # cleared), leaving swaps it back. Read through the chain, the
            # old mesh would land a placement off (#621).
            return False
        if hasattr(scene, "resolve_occurrence"):
            return scene.resolve_occurrence(self.occurrence_path) is self.chain[-1]
        return self.chain[0] in (getattr(scene, "groups", None) or ())

    def last_position(self):
        """Where the vertex stood when the hold died, in world
        coordinates — a vertex welded into the one it landed on keeps
        that very spot — or ``None`` when the hold's own mesh is no
        longer the one its group shows (#621: the old placement would put
        the point off)."""
        if self.chain and self.mesh is not getattr(self.chain[-1], "mesh",
                                                   None):
            return None
        v = self.vertex
        m = self.matrix()
        return m.map(QVector3D(v.position)) if m is not None \
            else QVector3D(v.position)

    def position(self, scene=None):
        """The vertex's world position now — ``None`` once the hold is no
        longer :meth:`alive`, which is the anchor's cue to let go."""
        if not self.alive(scene):
            return None
        v = self.vertex
        m = self.matrix()
        return m.map(QVector3D(v.position)) if m is not None \
            else QVector3D(v.position)


def _anchor_candidates(scene, point, tol):
    groups = getattr(scene, "groups", ()) or ()
    key = (getattr(scene, "content_version", getattr(scene, "version", None)), len(groups),
           id(getattr(scene, "edit_group", None)), id(scene.mesh))
    cached = scene.__dict__.get("_anchor_placements")
    if cached is None or cached[0] != key:
        cached = scene._anchor_placements = key, _NATIVE.AnchorPlacements(groups)
    return cached[1].candidates(point, tol)


def resolve_vertex_anchor(scene, point: QVector3D, tol: float = 1e-4):
    """The vertex sitting at *point* (world), as a :class:`VertexAnchor`,
    or ``None``. Loose geometry first, then every group placement, the
    point taken back into each placement's local coordinates — O(1) per
    mesh through the vertex registry, so a click costs nothing on a big
    model. ``tol`` is the registry's own weld resolution (0.1 mm): a
    point that came through a placement's float32 matrix and back lands
    within a micron of the vertex, which a tighter tolerance rejected on
    far-out coordinates. A lazy mesh is only built when its own points
    say the vertex may be there (``dimension_may_hold``; it and the
    placements' broad phase are
    ``rust/ingetrazo_core/src/model/anchor_broad.rs``)."""

    def hit(mesh, local):
        if not _NATIVE.dimension_may_hold(mesh, local, tol):
            return None
        v = mesh.vertex_at(local)
        if v is None or (v.position - local).length() > tol:
            return None
        if not v.edges:
            # An orphan: a vertex nothing references any more. Make Group
            # moves the geometry into the group's mesh and leaves these
            # behind in the loose one, at the very same spots — so a
            # dimension on a fresh group anchored to the ghost, and the
            # group scaled away from under it (Marco, 2026-09-20: «faltaba
            # en grupo»). A point held by nothing holds nothing.
            return None
        return v

    # The LOOSE mesh, not ``scene.mesh``: inside a group edit that is the
    # open group's, which the loop below finds with its chain — held as
    # loose, the anchor was dead from birth (#621).
    loose = getattr(scene, "loose_mesh", scene.mesh)
    v = hit(loose, QVector3D(point))
    if v is not None:
        return VertexAnchor(v, loose, ())
    for g, placed, inv in _anchor_candidates(scene, point, tol):
        local = QVector3D(point) if inv is None else inv.map(QVector3D(point))
        v = hit(placed.mesh, local)
        if v is not None:
            return VertexAnchor(v, placed.mesh, _chain_to(g, placed))
    return None


def _chain_to(root, target) -> tuple:
    """The groups from *root* down to *target* (inclusive) — the transforms
    an anchor composes. Depth-first, like ``iter_placements``."""
    if root is target:
        return (root,)
    for child in getattr(root, "children", None) or ():
        below = _chain_to(child, target)
        if below:
            return (root,) + below
    return ()


#: The axes a linear dimension can run along. The frame maths (the linear
#: frame, the value, the squared offset, the line's ends, what the cursor
#: asks for) is ``rust/ingetrazo_core/src/model/dimension_frame.rs``.
_AXES = ("x", "y", "z")
_linear_frame = _NATIVE.dimension_linear_frame


@record_fields('_a', '_b', 'offset', 'axis', 'layer', 'text', 'anchor_a', 'anchor_b', '_holds',
               'attachment_a', 'attachment_b')
class Dimension(Record):
    """See the module docstring. ``a`` and ``b`` read live through their
    anchors; assigning them sets the frozen position (what a rigid
    transform of the whole model does) and keeps the anchor."""

    def __init__(self, a: QVector3D, b: QVector3D, offset: QVector3D,
                 layer: str | None = None, text: str | None = None,
                 axis: str | None = None) -> None:
        self.attachment_a = self.attachment_b = None
        self._a = QVector3D(a)
        self._b = QVector3D(b)
        #: displacement from the a–b segment to the dimension line
        self.offset = QVector3D(offset)
        #: ``None`` = aligned; ``"x"``/``"y"``/``"z"`` = linear along that
        #: world axis (see the module docstring).
        self.axis = axis if axis in _AXES else None
        #: Layer (tag) the annotation lives on; ``None`` = default
        #: layer. Scenes hide layers, so a plan scene can show a clean model
        #: and an "Anotaciones" layer can carry the cotas and leader texts.
        self.layer = layer
        #: Custom text (double-click the value to edit it);
        #: ``None`` shows the measured value, and ``<>`` inside the text
        #: stands for it.
        self.text = text
        self.anchor_a: VertexAnchor | None = None
        self.anchor_b: VertexAnchor | None = None
        #: Every hold each endpoint has had, oldest first (see _live).
        self._holds: dict = {"a": [], "b": []}
        self._scene = None                   # set by bind(); re-binding

    # ---- endpoints ----------------------------------------------------------
    def _hold(self, which: str, anchor) -> None:
        """Make ``anchor`` the endpoint's hold and remember it."""
        setattr(self, "anchor_" + which, anchor)
        if anchor is None:
            return
        holds = self._holds_of(which)
        if not any(h.vertex == anchor.vertex and h.mesh is anchor.mesh
                   for h in holds):
            holds.append(anchor)
            del holds[:-_MAX_HOLDS]

    def __deepcopy__(self, memo):
        out = super().__deepcopy__(memo)
        if self._scene is not None and (scene := self._scene()) is not None:
            out._scene = weakref.ref(memo.get(id(scene), scene))
        return out

    def _holds_of(self, which: str) -> list:
        holds = getattr(self, "_holds", None)
        if holds is None:
            holds = self._holds = {"a": [], "b": []}
        return holds[which]

    def _live(self, which: str) -> QVector3D:
        attachment = getattr(self, "attachment_" + which)
        if attachment is not None:
            point = attachment.position()
            if point is not None:
                setattr(self, "_" + which, point)
            return getattr(self, "_" + which)
        holds = self._holds_of(which)
        anchor = getattr(self, "anchor_" + which)
        if anchor is None and not holds:
            return getattr(self, "_" + which)
        scene = self._scene() if self._scene is not None else None
        # The OLDEST hold still alive wins. A push collapsing the measured
        # edge welds its vertex into the one below, and the endpoint takes
        # that one; undo — or the next preview frame of the same drag —
        # brings the first vertex back, and the dimension must go back to
        # it, not stay on the one it fell onto (#254).
        for h in holds:
            p = h.position(scene)
            if p is not None:
                setattr(self, "anchor_" + which, h)
                setattr(self, "_" + which, p)
                return p
        if anchor is not None and anchor not in holds:
            p = anchor.position(scene)
            if p is not None:
                setattr(self, "_" + which, p)
                return p
        # The hold is gone — but the geometry may just have changed
        # hands: Make Group puts new vertices in the group's mesh at
        # the very spots the loose ones stood, Explode the other way
        # round (Marco, 2026-09-20: a cube dimensioned, then cube AND
        # dimension grouped, then scaled — «la cota no sigue»). Look
        # again where the endpoint last was; freeze only when nothing
        # stands there any more.
        frozen = getattr(self, "_" + which)
        again = self._resolve_again(which, scene)
        p = again.position(scene) if again is not None else None
        if p is None:
            setattr(self, "anchor_" + which, None)
            return frozen
        self._hold(which, again)
        setattr(self, "_" + which, p)
        return p

    def _resolve_again(self, which: str, scene):
        """The vertex to hold now that every hold of the endpoint died.
        First where the last hold's vertex died: a vertex welded into the
        one it was moved onto keeps that spot, while the frozen position
        may be older — read before the move — and another vertex may stand
        there by now (#563). Then the frozen position, as before."""
        if scene is None:
            return None
        holds = self._holds_of(which)
        anchor = getattr(self, "anchor_" + which)
        last = holds[-1] if holds else anchor
        frozen = getattr(self, "_" + which)
        if last is not None:
            spot = last.last_position()
            if spot is not None and (spot - frozen).lengthSquared() > 1e-12:
                found = resolve_vertex_anchor(scene, spot)
                if found is not None:
                    return found
        return resolve_vertex_anchor(scene, frozen)

    @property
    def a(self) -> QVector3D:
        return self._live("a")

    @a.setter
    def a(self, value: QVector3D) -> None:
        self._a = QVector3D(value)

    @property
    def b(self) -> QVector3D:
        return self._live("b")

    @b.setter
    def b(self, value: QVector3D) -> None:
        self._b = QVector3D(value)

    def bind(self, scene) -> None:
        """Anchor each endpoint to the vertex under it, if there is one —
        at placement, and again when a document is opened (the .igz keeps
        positions, and the vertex at that very position is the one the
        drafter snapped to)."""
        self._scene = weakref.ref(scene)
        self._holds = {"a": [], "b": []}
        self._hold("a", resolve_vertex_anchor(scene, self._a))
        self._hold("b", resolve_vertex_anchor(scene, self._b))

    def refresh_anchors(self, scene) -> None:
        """Re-take a hold that died — the vertex changed meshes — from the
        vertex now standing at the endpoint; the history calls this after
        every command, so a group made and scaled in one breath still
        carries its dimension. An endpoint never anchored stays static."""
        self._scene = weakref.ref(scene)
        for which in ("a", "b"):
            anchor = getattr(self, "anchor_" + which)
            if anchor is None and not self._holds_of(which):
                continue
            if any(h.alive(scene) for h in self._holds_of(which)):
                continue
            if anchor is not None and anchor.alive(scene):
                continue
            self._hold(which, self._resolve_again(which, scene))

    @property
    def anchored(self) -> bool:
        """Both endpoints held by vertices."""
        return ((self.anchor_a is not None or self.attachment_a is not None)
                and (self.anchor_b is not None or self.attachment_b is not None))

    def anchor_points(self) -> list:
        """The world positions of the anchored endpoints (0, 1 or 2)."""
        return [getattr(self, end) for end in ("a", "b")
                if getattr(self, "anchor_" + end) is not None
                or getattr(self, "attachment_" + end) is not None]

    # ---- geometry -----------------------------------------------------------
    def display_text(self, measured: str) -> str:
        """What the annotation shows: the custom text with ``<>`` swapped
        for the formatted measurement, or the measurement itself."""
        if self.text:
            return self.text.replace("<>", measured)
        return measured

    def value(self) -> float:
        """Measured length (metres): the segment's, or — linear — its extent
        along the axis."""
        return _NATIVE.dimension_value(self.a, self.b, self.axis)

    def label(self) -> str:
        from core.units import fmt_len
        return fmt_len(self.value())

    def perpendicular_offset(self) -> QVector3D:
        """The offset squared to the measured segment (the geometry may have
        turned under an anchored dimension since it was placed); linear: only
        the pull along the extension direction."""
        return _NATIVE.dimension_perpendicular_offset(self.a, self.b, self.offset, self.axis)

    @property
    def line_point_a(self) -> QVector3D:
        return self.line_points()[0]

    @property
    def line_point_b(self) -> QVector3D:
        return self.line_points()[1]

    def line_points(self) -> tuple[QVector3D, QVector3D]:
        """The dimension line's endpoints. Aligned: ``a``/``b`` shifted by
        the offset. Linear: each endpoint pushed along the extension
        direction until both sit on one line parallel to the axis — the one
        through ``a + offset``."""
        return _NATIVE.dimension_line_points(self.a, self.b, self.offset, self.axis)

    def midpoint(self) -> QVector3D:
        return _NATIVE.dimension_midpoint(self.a, self.b, self.offset, self.axis)

    def ends_riding(self, group_ids) -> tuple:
        """Which endpoints (``"a"``, ``"b"``) hold a vertex placed by one of
        the groups whose ``id`` is in *group_ids* — the ones a transform of
        those groups carries along."""
        return tuple(
            which for which in ("a", "b")
            if (an := getattr(self, "anchor_" + which)) is not None
            and any(id(g) in group_ids for g in an.chain))

    def carried(self, m: QMatrix4x4, ends) -> "Dimension":
        """A frozen copy with the endpoints in *ends* taken through *m*:
        what the dimension will read once a groups transform previewed
        through *m* is committed (#621). The offset stays, as the commit
        leaves it; :meth:`line_points` squares it to the new segment."""
        import copy
        c = copy.copy(self)
        a, b = self.a, self.b
        c._a = m.map(a) if "a" in ends else QVector3D(a)
        c._b = m.map(b) if "b" in ends else QVector3D(b)
        c.offset = QVector3D(self.offset)
        c.attachment_a = c.attachment_b = None
        c.anchor_a = c.anchor_b = None
        c._holds = {"a": [], "b": []}
        c._scene = None
        return c

    #: The offset placing the dimension line through ``cursor``, parallel
    #: to ``a``–``b``: ``offset_for_cursor(a, b, cursor)``.
    offset_for_cursor = staticmethod(_NATIVE.dimension_offset_for_cursor)
    #: ``(offset, axis)`` for the cursor's position: pulled PAST both
    #: endpoints along one axis's extension direction, the linear dimension
    #: along that axis; square off the segment, anywhere else, or past both
    #: in two directions at once, the aligned one. An axis within 15° of
    #: the segment, or of square to it, is never offered.
    placement_for_cursor = staticmethod(_NATIVE.dimension_placement_for_cursor)
