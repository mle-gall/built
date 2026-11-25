# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The arrow-key axis lock lasts one operation, not until Esc (issue #30).

@pacaeiro: «In SK, if you're drawing a line and press Arrow to lock X and
finish drawing the line, the X lock is released automatically. In ingeTrazo
the Axis lock keeps active (it can be released with an Escape key, not a big
deal). If we are trying to somewhat mimic SK, it's a detail to consider.»

It is more than a detail: the lock outlived the line it was for, so the
NEXT line silently came out constrained to the same axis, and the label
stayed on screen claiming a state the user had finished with.

Same shape as the Alt toggle in issue #26 — a mode meant to last one
operation that lasted the session — and released at the same three places
an operation can end: a click that commits, a stroke released, a typed
value applied. The Down-arrow reference is deliberately left alone: it is
bound to an edge the user went and picked.
"""
from __future__ import annotations


from tests.shell_isolation import restore_shell_patches  # noqa: F401
from core.input_types import Input
from ingetrazo_render import MouseEvent
from ingetrazo_core import Vec3 as QVector3D


def _viewport(native_host):
    win = native_host.window
    return win, win.viewport


def _click(vp, x=5.0, y=5.0):
    vp._dispatch_tool_click(MouseEvent(
        "press", x, y, Input.LeftButton,
        Input.LeftButton, Input.NoModifier))


def test_the_lock_goes_the_moment_the_SEGMENT_exists(native_host):
    """Corrected 2026-09-18 by @pacaeiro, testing 0.4.4: «I draw a line with
    hard lock (arrows) and, after the line is created the lock is still
    active».

    This test used to assert the opposite — that the lock survives the
    second click — and that was my reading, not the real behaviour. The first fix
    released the lock when the OPERATION ended, and for the Line tool the
    operation is the whole polyline, because it chains. He means the
    moment the line EXISTS. He draws this way every day; the test was
    encoding my guess over his measurement.

    The signal is «did this click make something», not «is the tool idle».
    """
    win, vp = _viewport(native_host)
    win._activate_tool("line")
    vp.active_tool.start_point = QVector3D(0.0, 0.0, 0.0)
    vp.axis_lock = "x"
    _click(vp)                       # this one commits a segment
    assert vp.axis_lock is None


def test_but_a_click_that_makes_NOTHING_keeps_it(native_host):
    """The first click of a line only sets the start point, so a lock taken
    before it must still be there to constrain the segment — otherwise the
    lock would be useless in the only place it is used."""
    win, vp = _viewport(native_host)
    win._activate_tool("line")
    vp.axis_lock = "x"
    _click(vp)                       # first click: nothing committed
    assert vp.axis_lock == "x"
    assert vp._tool_busy(vp.active_tool), "the line is under way"


def test_the_lock_goes_when_the_line_is_finished(native_host):
    """The Line tool CHAINS — after the second click it keeps drawing from
    there — so «the operation» is the whole polyline, and it ends when the
    chain does: closing the loop resets the tool, and the lock goes with
    it. (Ending with Esc was already covered: releasing the constraint is
    the cascade's second step.)"""
    win, vp = _viewport(native_host)
    win._activate_tool("line")
    tool = vp.active_tool
    tool.start_point = QVector3D(1.0, 0.0, 0.0)
    vp.axis_lock = "x"
    tool.start_point = None                    # the chain closed
    vp._release_axis_lock_after_operation()
    assert vp.axis_lock is None


def test_switching_tool_drops_it_too(native_host):
    """The leak that was worse than the report: this reset cleared every
    other sticky state and forgot the axis lock, so a lock taken while
    drawing a line silently constrained the next tool."""
    win, vp = _viewport(native_host)
    win._activate_tool("line")
    vp.axis_lock = "x"
    win._activate_tool("rectangle")
    assert vp.axis_lock is None


def test_a_typed_length_ends_the_operation_too(native_host):
    """The lock is released however the line ends, and typing a distance and
    pressing Enter is the other way to end one."""
    win, vp = _viewport(native_host)
    win._activate_tool("line")
    vp.axis_lock = "y"
    vp.active_tool.start_point = None
    vp._release_axis_lock_after_operation()
    assert vp.axis_lock is None


def test_a_typed_length_actually_drops_the_lock(native_host):
    """The path that had the bug (Marco): the Line tool chains, so the
    instant the typed value commits a segment the tool is STILL busy. The
    old code called ``_release_axis_lock_after_operation()`` without the
    committed flag, the busy-tool guard ate it, and the hard lock survived a
    typed length while the second CLICK dropped it. Same commit signal as
    the click path: did this input make geometry?"""
    from ingetrazo_render import KeyEvent
    win, vp = _viewport(native_host)
    win._activate_tool("line")
    tool = vp.active_tool
    tool.start_point = QVector3D(0.0, 0.0, 0.0)
    tool.hover_point = QVector3D(1.0, 0.0, 0.0)     # pointing along X
    vp.axis_lock = "x"
    before = len(vp.history.undo_stack)
    for ch in "1500":
        vp._handle_value_key(
            KeyEvent(True, Input.Key_0, Input.NoModifier, ch))
    vp._handle_value_key(
        KeyEvent(True, Input.Key_Return, Input.NoModifier, "\r"))
    assert len(vp.history.undo_stack) == before + 1, "the segment exists"
    assert vp.axis_lock is None


def test_the_down_arrow_reference_is_left_alone(native_host):
    """Not in the report and not the same gesture: it is bound to an edge
    the user went and picked, and Esc still drops it."""
    win, vp = _viewport(native_host)
    win._activate_tool("line")
    vp.reference_mode = "parallel"
    vp.active_tool.start_point = None
    vp._release_axis_lock_after_operation()
    assert vp.reference_mode == "parallel"
    assert vp.release_constraints() is True     # …and Esc still does
    assert vp.reference_mode is None


def test_esc_still_releases_it_mid_operation(native_host):
    """The Esc cascade is untouched: mid-line you can still drop the lock
    without cancelling the line."""
    win, vp = _viewport(native_host)
    win._activate_tool("line")
    vp.active_tool.start_point = QVector3D(0.0, 0.0, 0.0)
    vp.axis_lock = "z"
    assert vp.release_constraints() is True
    assert vp.axis_lock is None
