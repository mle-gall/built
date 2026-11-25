# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Shadows as document data + the light's fitted projection + the panel.

The GL passes need a screen; what CAN be pinned headless is everything the
render derives from: the settings round-trip through the .igz, the sun's
ortho view-projection actually containing the model, and the tray panel
writing ``scene.shadows``.
"""
from __future__ import annotations

import pytest
from built_core import Vec3 as QVector3D
from tests.preferences_chrome import Controls
from tests.shell_isolation import restore_shell_patches  # noqa: F401

from built_core.scene import Scene  # noqa: E402
from built_core.sun import ShadowSettings  # noqa: E402


def V(x: float, y: float, z: float = 0.0) -> QVector3D:
    return QVector3D(float(x), float(y), float(z))


def test_shadows_round_trip_through_igz(tmp_path):
    from formats import igz
    scene = Scene()
    scene.add_edge(V(0, 0, 0), V(2, 0, 0))
    scene.shadows = ShadowSettings(enabled=True, month=6, day=21,
                                   hour=15, minute=30, darkness=0.4)
    path = tmp_path / "sombras.igz"
    igz.save_scene(scene, path)

    loaded = Scene()
    igz.load_into(loaded, path)
    assert loaded.shadows == scene.shadows

    # A document saved WITHOUT shadows loads with the defaults (off).
    plain = Scene()
    plain.add_edge(V(0, 0, 0), V(1, 0, 0))
    p2 = tmp_path / "plano.igz"
    igz.save_scene(plain, p2)
    igz.load_into(loaded, p2)
    assert loaded.shadows == ShadowSettings()


def test_light_vp_contains_the_model():
    """Every corner of the model's bounds must land inside the sun's ortho
    frustum — points outside the map sample as 'lit', so a bad fit would
    silently erase shadows at the rims."""
    from tests.controller_surface import ControllerSurface as Viewport
    lo, hi = V(-3, -2, 0), V(5, 7, 4)
    for d in ((0.3, 0.5, 0.8), (0.0, 0.0, 1.0), (-0.6, 0.2, 0.75)):
        n = (d[0] ** 2 + d[1] ** 2 + d[2] ** 2) ** 0.5
        d = (d[0] / n, d[1] / n, d[2] / n)
        m = Viewport._light_vp(d, lo, hi)
        for x in (lo.x(), hi.x()):
            for y in (lo.y(), hi.y()):
                for z in (lo.z(), hi.z(), 0.0):   # the ground patch too
                    p = m.map(QVector3D(x, y, z))
                    assert -1.0 < p.x() < 1.0
                    assert -1.0 < p.y() < 1.0
                    assert -1.0 < p.z() < 1.0


@pytest.fixture
def sun(native_host):
    return Controls(native_host.window, "appearance")


def test_panel_writes_scene_shadows(sun):
    sh = sun.env.scene.shadows
    assert not sun.value("sun_enabled")
    sun.set("sun_enabled", True)
    sun.set("sun_time", 15 * 60 + 30)
    sun.set("sun_dark", 60)
    assert sh.enabled and (sh.hour, sh.minute) == (15, 30)
    assert sh.darkness == pytest.approx(0.4)
    assert sun.row("sun_time").fmt(sun.value("sun_time")) == "15:30"
    sh.enabled, sh.hour = False, 9
    assert not sun.value("sun_enabled") and sun.value("sun_time") == 570


def test_panel_month_bar_and_time_zone(sun):
    sh = sun.env.scene.shadows
    assert sun.value("sun_zone") == "auto"
    sun.set("sun_day", 172)
    assert (sh.month, sh.day) == (6, 21) and sun.value("sun_date") == "06-21"
    sun.set("sun_date", "12-21")
    assert (sh.month, sh.day) == (12, 21) and sun.value("sun_day") == 355
    sun.set("sun_zone", "-4")
    assert sh.utc_offset == -4
    sun.set("sun_zone", "auto")
    assert sh.utc_offset is None


def test_time_slider_is_bounded_to_daylight(sun):
    time = sun.row("sun_time")
    assert 0 < time.lo < 720 < time.hi < 1439
    assert time.fmt(time.lo) in time.tip and time.fmt(time.hi) in time.tip
    sun.set("sun_time", time.hi)
    sun.set("sun_zone", "-10")
    time = sun.row("sun_time")
    sh = sun.env.scene.shadows
    assert time.hi < 14 * 60
    assert time.lo <= sh.hour * 60 + sh.minute == sun.value("sun_time") <= time.hi
