# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Esc escalates: value buffer → sticky constraints (axis lock / reference)
→ in-progress tool action → selection. The axis-lock release is the "Esc
should stop constraining" report: the window must release the arrow-key lock before it cancels a tool."""
from __future__ import annotations

from core.input_types import Qt
from tests.shell_isolation import restore_shell_patches  # noqa: F401


def escape(host):
    host.key(True, Qt.Key_Escape)
    host.key(False, Qt.Key_Escape)


def test_esc_releases_axis_lock_first(native_host):
    vp = native_host.viewport
    from ingetrazo_core import Vec3
    native_host.window._activate_tool("line")
    native_host.view.tools.sync()
    tool = vp.active_tool
    tool.start_point = Vec3(1, 2, 3)
    vp.axis_lock = "x"
    escape(native_host)
    assert vp.axis_lock is None and tool.start_point == Vec3(1, 2, 3)
    escape(native_host)
    assert tool.start_point is None


def test_esc_releases_reference_mode(native_host):
    vp = native_host.viewport
    vp.reference_edge = object()
    vp.reference_mode = "parallel"
    escape(native_host)
    assert vp.reference_mode is None and vp.reference_edge is None


def test_esc_clears_value_buffer_before_constraints(native_host):
    vp = native_host.viewport
    native_host.view.tools.sync()
    vp._value_buffer = "2.5"
    vp.axis_lock = "z"
    escape(native_host)                    # 1st Esc: clear typed value
    assert vp._value_buffer == ""
    assert vp.axis_lock == "z"               # lock survives that press
    escape(native_host)                    # 2nd Esc: release the lock
    assert vp.axis_lock is None


def test_esc_without_constraints_still_clears_selection(native_host):
    vp = native_host.viewport
    f = vp.scene.mesh.add_face(_square())
    vp.scene.selection.add(f)
    escape(native_host)
    assert not vp.scene.selection


def test_viewport_release_constraints_reports_whether_it_did(native_host):
    vp = native_host.viewport
    assert vp.release_constraints() is False
    vp.axis_lock = "y"
    assert vp.release_constraints() is True
    assert vp.release_constraints() is False


def _square():
    from ingetrazo_core import Vec3 as QVector3D
    return [QVector3D(0, 0, 0), QVector3D(1, 0, 0),
            QVector3D(1, 1, 0), QVector3D(0, 1, 0)]
