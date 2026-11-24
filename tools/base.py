# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Base class for IngeTrazo tools and the per-event ``ToolContext`` they receive.

A tool is anything the user activates from the toolbar to interact with the
viewport: draw, modify, select. Both built-in tools and third-party plugins
inherit from :class:`Tool` so they can be registered uniformly.

Spatial tools (line, rectangle, push/pull, select, ...) override
:meth:`on_click`, :meth:`on_hover` and :meth:`on_cancel`. The viewport
raycasts the mouse pixel against the working plane and produces a
:class:`ToolContext` that combines the snapped world point, the raw screen
position, keyboard modifiers and the snap metadata.
"""
from __future__ import annotations

from ingetrazo_core import register_tool_adapter, register_tool_functions

from abc import ABC, abstractmethod
from dataclasses import dataclass

from core.input_types import QPointF, Qt
from ingetrazo_core import Vec3 as QVector3D, cursor_pixel_scale, cursor_plane_reference, cursor_plane_color, plane_input

from core.snap import POINT_SNAP_KINDS, SnapResult


#: Arrow keys → the axis a locked drawing plane is NORMAL to (Right =
#: red, Left = green, Up = blue), and the plane's everyday name.
PLANE_LOCK_KEYS = {int(Qt.Key_Right): "x", int(Qt.Key_Left): "y",
                   int(Qt.Key_Up): "z"}
# The drawing axes (core.axes): the world's at the top level, the open
# group's own inside it (issue #44). The same dict object, kept current.
from core.axes import AXES as PLANE_LOCK_AXES  # noqa: E402
PLANE_LOCK_NAMES = {"x": "YZ", "y": "XZ", "z": "XY"}


@register_tool_adapter
class AxisMagnet:
    """The Line tool's axis magnet, for the other drawing tools.

    @pacaeiro, issue #52: «The Draw commands in the group of ARCS and
    SHAPES should also use the magnetic Snaps, like LINE. Except
        Freehand» — and #51 for Text, #50 for Dimension. Same two numbers as
    Line (issue #31): within 3° of an axis from ``start_point`` the point
    lands ON the axis instead of merely lighting the cue, and the axis the
    work plane cannot hold is found in pixels. Alt still switches it off.

    A shape that lives in ONE plane (``PlaneLock``) still gets both, but
    the screen detector's answer is confined to the axes the plane HOLDS
    (``axis_in_plane`` in the snap engine): it widens the capture to the
    screen's 9 px for those two axes, never pulling the point off the plane.

    Only a click that is a *direction from the start point* wants this:
    an arc's bulge, a rotated rectangle's height or a dimension's placement
    are measured from somewhere else, and a magnet aimed at the first point
    would bend them. Those tools say when by overriding :meth:`magnet_on`;
    the viewport reads the two properties afresh on every hover.

    Rectangle stays out on purpose: its second click is the opposite
    corner, and a corner pulled onto the axis through the first is a
    rectangle of zero height — every thin rectangle near an axis would
    collapse. Its plane lock and the Shift/arrow locks are its magnets.
    """

    _MAGNET_DEG = 3.0
    _MAGNET_PX = 9.0


@register_tool_adapter
class PlaneLock:
    """Arrow keys BEFORE the first click lock a planar tool (circle,
    polygon, rectangle, the arcs) to a drawing plane: Right = the plane
    normal to X (YZ), Left = normal to Y (XZ), Up = normal to Z (XY); the
    same key again frees it. The classic plane lock (Marco, 2026-09-08:
    «quiero dibujar un círculo en el plano ZX… apretando las teclas de
    desplazamiento»).
        Before the first click the key only chooses the plane, and that click
    anchors it at the point; after it the plane stops
    following the cursor and the key holds the shape on the plane it is
    drawing on — the same key frees it (handing the plane back to the
    cursor), a different one switches it. The lock is spent by the shape
    (or Esc), the classic way. The arrows went back to being the
    viewport's linear axis lock after the first click until Marco asked
    for them to answer the whole way through («também deve funcionar»),
    except where a later click reads that lock —
    ``plane_lock_after_the_first_point``.

    The DOWN arrow is the classic magenta reference lock: over an
    edge the plane goes PERPENDICULAR to that edge, over a face PARALLEL
    to it; Down again frees it. It is the native way to start a pipe's
    circle on an inclined axis line, then Follow Me it (issue #10,
    @pacaeiro: «I draw lines to represent the axis of the tubes … then
    have to rotate [the circle] to align the normal of the circle's plane
    to the line»).

    Shift is the Down arrow HELD: for as long as the key is down the plane
    the cursor is detecting stays where it is, and letting go hands it
    back — nothing is locked afterwards, it is the way to look at the
    plane before committing to it (Marco: «antes de clicar no 1º ponto o
    Shift funciona como se clicasse na arrow down temporariamente… marca o
    plano que está a ser considerado, por exemplo uma face num cubo;
    depois do 1º click é igual»). It is additive: the pointer goes on
    marking midpoints, endpoints and alignments, and the inference lock
    the viewport takes off the cursor still happens (``Viewport.
    _capture_shift_lock``). A plane an arrow or Down is already holding is
    not Shift's to release. See ``mark_plane``."""

    # The lock's state is Rust's (``ingetrazo_core.PlaneLock``, rust/
    # ingetrazo_core/src/tools/plane.rs): ``plane_lock`` — the arrow-key
    # axis, ``"x"``/``"y"``/``"z"`` or None; ``plane_ref`` — the Down-arrow
    # / held-Shift reference ``(normal, kind)``, kind ``"edge"`` (the plane
    # perpendicular to the hovered edge), ``"face"`` (parallel to the
    # hovered face) or ``"plane"`` (the plane a held Shift marks; the
    # viewport lets it go on the release), exclusive with ``plane_lock``;
    # ``hover_plane`` — the plane the viewport used for the last cursor
    # hit: a shape started on a free point is laid out on it, so it
    # follows the view (Rafael's review, 2026-09-10).

    #: Whether the arrow lock goes on answering once the first point is
    #: down. The rotated rectangle says no: with the base edge down the
    #: arrows are the viewport's linear axis lock, which its width magnet
    #: reads (``RotatedRectangleTool._width_dir``).
    plane_lock_after_the_first_point: bool = True

    #: How far off the drawing plane a point must be to count as pointing
    #: ELSEWHERE. The free cursor lies on the plane by construction, so
    #: only a snap (endpoint, midpoint…) gets this far.
    OFF_PLANE_TOL = 1e-3

    #: Radius (circle) / half-side (rectangle) of the cursor preview, px.
    PREVIEW_PX = 22

    plane_color = staticmethod(cursor_plane_color)

    world_per_pixel = staticmethod(cursor_pixel_scale)
    plane_preview_scale = staticmethod(cursor_pixel_scale)

    hovered_reference = staticmethod(cursor_plane_reference)

@dataclass
class ToolContext:
    """Bundle of data a tool needs to react to a viewport event."""

    viewport: object  # forward reference to views.viewport.Viewport
    world: QVector3D
    screen: QPointF
    modifiers: Qt.KeyboardModifiers
    snap: SnapResult


class Tool(ABC):
    name: str = "Unnamed"
    icon: str | None = None
    shortcut: str | None = None
    #: A second key for the same tool, when a key changes hands and the old
    #: one is worth keeping. Push/Pull answers to the usual P and to the U
    #: it had here for a year. It is the SAME action with two shortcuts —
    #: never a second action, which is what Qt kills (tests/test_shortcuts.py).
    shortcut_alt: str | None = None
    #: What the tool does, in a sentence — without its name or its key,
    #: which the tooltip and F3 already show beside it (Blender's
    #: descriptions). English; it goes through ``tr`` where it is shown.
    description: str | None = None
    #: Where a plugin tool's entry goes in the Extensions menu among the
    #: others: lower first, the same keeps the folder order. The Python
    #: Console, a tool for developers, goes after the model tools.
    menu_order: int = 0
    # Drawing tools snap to geometry and show the snap markers/tooltips
    # (Endpoint, On Edge, On Face, ...). Tools that only pick existing
    # geometry (Select, Push/Pull) set this False: no snap engine, no markers.
    uses_snap: bool = True
    #: Whether the tool also takes the engine's SOFT inferences — the
    #: encouraged points (pausing over a corner), their «level with» /
    #: «from point» lines, the axis magnet. A tool that only reads the hard
    #: point snaps sets this False: the dwell encouragement never runs, so
    #: those directional cues are never produced. The Offset does.
    allow_inferences: bool = True
    #: The snap kinds a tool accepts from the engine; ``None`` accepts every
    #: kind. A tool whose own operation only makes sense on certain snaps
    #: sets this, and the engine then refuses the rest — the Offset only
    #: takes NAMED model points (endpoint, midpoint, …): ``on_edge`` and
    #: ``on_face`` follow the cursor and are not reliable snaps, so they are
    #: dropped (no distance change, no marker) instead of being offered.
    allowed_snap_kinds: frozenset | None = None
    # Tools that support a click-drag rubber-band box (Select). For these the
    # viewport defers the click to release: a tiny drag is a click (on_click),
    # a real drag is a box (on_box_select).
    box_select: bool = False
    # RGBA the viewport uses for this tool's preview lines, overriding the
    # snap-based rubber-band colour. ``None`` keeps the default behaviour;
    # Push/Pull sets the normal edge colour so its box reads as real geometry.
    wireframe_color: tuple[float, float, float, float] | None = None
    # When True the preview lines are depth-tested (hidden-line removal) instead
    # of floating on top. Push/Pull uses this so its forming box hides its own
    # back edges behind its faces, like real geometry; loose drawing tools keep
    # the default (preview always visible on top).
    wireframe_depth_tested: bool = False
            # Caption for the Measurements box (VCB) while this tool is
    # active — "Length", "Dimensions", "Distance". ``None`` hides the box.
    vcb_label: str | None = None
    #: The kind of the cursor's snap as of the last hover or click
    #: (``ToolContext.snap.kind``), pushed by the viewport before it calls
    #: the tool. ``"on_face"`` over a bare face, a ``core.snap.
    #: POINT_SNAP_KINDS`` value when the cursor names a point of the model.
    #: ``PlaneLock.snapped_plane`` reads it: only a cursor that names a
    #: point gets to move a planar shape's plane (#545).
    snap_kind: str | None = None

    @abstractmethod
    def on_activate(self, viewport) -> None:
        """Called when the user selects this tool."""

    @abstractmethod
    def on_deactivate(self, viewport) -> None:
        """Called when the user switches to another tool."""

    def on_click(self, ctx: ToolContext) -> None:
        """Left click at ``ctx.world`` (already snapped)."""

    def on_hover(self, ctx: ToolContext) -> None:
        """Mouse moved to ``ctx.world`` without a button pressed."""

    def on_double_click(self, ctx: ToolContext) -> None:
        """Left double-click. Qt swallows the second press, so tools that act
        on it (Push/Pull repeating its last distance) override this; the
        default is to treat it as a plain click so other tools keep their
        click-click rhythm even when the user clicks fast."""
        self.on_click(ctx)

    def on_triple_click(self, ctx: "ToolContext") -> None:
        """Third click in place (select all connected). Defaults to
        a plain click so unaware tools keep their rhythm."""
        self.on_click(ctx)

    def on_box_select(self, viewport, rect, crossing: bool,
                      additive: bool = False, mode: str | None = None) -> None:
        """Rubber-band box released. ``rect`` is ``(x0, y0, x1, y1)`` in screen
        pixels (normalized so x0<=x1, y0<=y1). ``crossing`` is True for a
        right-to-left drag (select anything the box touches) and False for a
        left-to-right drag (select only what's fully enclosed). ``mode`` is how
        the catch joins the selection — "replace" / "add" / "toggle" /
        "remove", from the keyboard modifiers (see
        ``tools.rust_tools.selection_mode``); ``additive`` is the older two-state
        spelling, kept for tools that only know it. Only tools with
        ``box_select = True`` receive this."""

    def drag_plane(self, viewport):
        """``(point, normal)`` of the plane the cursor should be read on
        while this tool is mid-operation, or ``None`` to leave the choice
        to the viewport (captured face, start point, ground).

        The viewport turns every pixel into a world point by hitting a
        plane, and hands the tool NOTHING — no hover, no click — when the
        ray misses it. A tool whose operation runs along a LINE (Push/Pull
        along its normal) only ever needs the pixel, and the ground stops
        answering the moment the cursor crosses the horizon: Rafael pulled
        a box up with the camera at ground level, the box froze at the
        horizon, and the click on its top never arrived (2026-09-16). Such a
        tool answers with a plane that contains its line and faces the
        camera, which the ray always hits."""
        return None

    def on_cancel(self, viewport) -> None:
        """Esc pressed — abandon any in-progress operation."""

    def on_release(self, viewport) -> None:
        """Left button released. Tools that act on press-drag-release strokes
        (the Eraser) override this; the default is a no-op, so click-driven
        tools are unaffected."""

    def on_key(self, viewport, key: int, modifiers: Qt.KeyboardModifiers) -> bool:
        """Tool gets first shot at the key. Return True to consume it."""
        return False

    def on_value(self, viewport, value: float) -> bool:
        """User typed a numeric length and pressed Enter (VCB style).

        Tools that accept a numeric value (line length, rectangle dimensions,
        circle radius, ...) override this and return True on success. The
        viewport handles digit buffering and dispatch; tools only need to
        consume the value.
        """
        return False

    def status_clause(self) -> str:
        """A short clause this tool adds to the status bar, or "".

        The classic tool keeps modifiers ON SCREEN the whole time the tool is
        active — «Ctrl = Líneas guía del ciclo/Puntos guía/Medida» sits
        there next to the instruction — instead of flashing them once when
        you press the key. A flash tells you what just happened; this tells
        you what you can do, and which way it is set right now (Marco,
        2026-09-17)."""
        return ""

    @staticmethod
    def ctrl_tapped(viewport) -> bool:
        """Whether the Ctrl being RELEASED was a tap of its own.

        A modifier toggled on the PRESS fires on every Ctrl+<key> chord too:
        Ctrl+Z reaches the viewport only as a ShortcutOverride, so the mode
        switched silently while undoing (issue #183, Alejandro Limón). Toggle
        on the RELEASE and ask this first — the viewport knows whether some
        other key came in between. Stand-ins without the tracker (tests)
        count as a tap."""
        tapped = getattr(viewport, "ctrl_tapped", None)
        return bool(tapped()) if callable(tapped) else True

    @staticmethod
    def refresh_hint(viewport) -> None:
        """Redraw the status bar, so ``status_clause`` shows the new mode."""
        hint = getattr(viewport, "refresh_status_hint", None)
        if callable(hint):
            hint()

    def rubber_band_lines(self):
        """Return ``[(a, b), ...]`` line segments to draw as the live preview.

        ``LineTool`` returns a single segment; ``RectangleTool`` returns four.
        The viewport renders whatever the active tool returns; tools that
        don't preview anything default to an empty list.
        """
        return []


    def guide_preview_lines(self):
        """``[(a, b), ...]`` — the guide LINES this tool is about to leave,
        drawn dashed like a real guide while the cursor moves (#89,
        @pacaeiro). Long segments are fine: the viewport clips them."""
        return []

    def inference_guide_lines(self):
        """``[(a, b, kind), ...]`` — reference segments drawn dashed in the
        snap-kind colour while the tool is inferring (Push/Pull's «level with
        that point» line from the geometry it locked onto to the moving cap).
        Tools that infer nothing default to an empty list."""
        return []

    def preview_faces(self):
        """Return ``Face`` objects to render shaded as a live solid preview.

        Push/Pull uses this so the extruded box appears filled while you drag,
        the way users expect the solid forming — not just its wireframe. The
        viewport triangulates and draws them depth-tested every frame; tools
        that have no solid preview default to an empty list.
        """
        return []

register_tool_functions(globals())
